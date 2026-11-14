//! Pine script tools: list, read, compile and edit the Pine Studio's scripts, and read, apply,
//! change and remove the scripts applied to charts. Script reads and compiles run on a worker
//! against `ScriptStore`; what a chart has computed is read on the app thread.

use crate::mcp::bridge::{McpError, McpReply, REQUEST_TIMEOUT};
use crate::mcp::chart::tools::ChartPineParams;
use crate::mcp::pine::{self, InputArgs, PineCommand, Target, Version};
use crate::mcp::state::ClientIdentity;
use crate::mcp::tools::{
    Caller, DegenViewMcp, failure, invalid, parse_chart_id, parse_id, parse_workspace_id,
    unexpected,
};
use crate::mcp::types::{
    AppliedPineDetail, AppliedPineList, PineAlertsResult, PineCompileResult, PineEditResult,
    PineInputsResult, PineInstanceChangeResult, PineReportResult, PineScriptSource,
    PineScriptsResult, PineStylesResult, ScriptStatus, SourceVersion,
};
use crate::model::mcp::Permission;
use crate::store::scripts::ScriptStore;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// Adds the diagnostics compile to an edit that already succeeded. If the compile could not run
/// (too many script jobs, interrupted, library unreadable) the edit stands and says so, instead
/// of reporting `compile: null` as if there were nothing to report.
fn attach_compile(
    result: &mut PineEditResult,
    compiled: Result<PineCompileResult, CallToolResult>,
) {
    match compiled {
        Ok(compiled) => result.compile = Some(compiled),
        Err(_) => {
            result.compile = None;
            result.compile_warning = Some(
                "The edit was applied, but its diagnostics could not be compiled right now (the script library was unreadable, too many script requests were running, or the request was interrupted). Call degenview_compile_pine_script to get them"
                    .into(),
            );
        }
    }
}

/// Which text of a script: the last saved source, or the unsaved draft kept beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpScriptVersion {
    Saved,
    Draft,
}

impl McpScriptVersion {
    fn version(self) -> Version {
        match self {
            Self::Saved => Version::Saved,
            Self::Draft => Version::Draft,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPineScriptParams {
    /// A `script_id` from `degenview_list_pine_scripts`.
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// `saved` or `draft`. Asking for a draft that does not exist is an error, never the saved text.
    pub version: McpScriptVersion,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompilePineParams {
    /// A script to compile; give this or `source`, not both.
    #[schemars(length(min = 1, max = 64))]
    pub script_id: Option<String>,
    /// With `script_id`: `saved` (default) or `draft`.
    pub version: Option<McpScriptVersion>,
    /// Source to check without saving it anywhere; give this or `script_id`.
    #[schemars(length(max = 200000))]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePineParams {
    /// A `script_id` from `degenview_list_pine_scripts`.
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// The complete new source text.
    #[schemars(length(max = 200000))]
    pub source: String,
    /// The `source_hash` of the text you read and based this on. If the script is not at that
    /// revision any more (the user typed, or another client edited) the edit is refused with a
    /// CONFLICT and nothing is overwritten.
    #[schemars(length(min = 64, max = 64))]
    pub expected_revision: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePineParams {
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// The `source_hash` of the draft you reviewed (the revision returned by the last edit).
    #[schemars(length(min = 64, max = 64))]
    pub expected_revision: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PineInputsParams {
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// `saved` (default) or `draft`.
    pub version: Option<McpScriptVersion>,
}

/// Source accepted inline for a compile (the request body is capped near this anyway).
const MAX_INLINE_SOURCE_CHARS: usize = 200_000;

/// Script ids are file-store ids; refuse anything that is not plain id characters.
fn clean_script_id(raw: &str) -> Result<String, CallToolResult> {
    let id = raw.trim();
    let plain = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if plain {
        Ok(id.to_owned())
    } else {
        Err(invalid(
            "script_id must be an id returned by degenview_list_pine_scripts",
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PineInstanceParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// An `instance_id` from `degenview_list_chart_pine_instances`. The same script can be applied
    /// to a chart more than once; each application is its own instance.
    #[schemars(length(max = 20))]
    pub instance_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PineReportParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    #[schemars(length(max = 20))]
    pub instance_id: String,
    /// Closed trades to skip (oldest first). Default 0.
    #[schemars(range(min = 0, max = 1000000))]
    pub trades_offset: Option<u32>,
    /// Closed trades to return, 1 to 200. Default 50.
    #[schemars(range(min = 1, max = 200))]
    pub trades_limit: Option<u32>,
    /// Include the equity curve, thinned to at most 200 points. Default false.
    pub include_equity: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddPineToChartParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// A `script_id` from `degenview_list_pine_scripts`: an indicator or strategy. Its last
    /// *saved* source is applied, never an unsaved draft.
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// Input overrides by input id (see `degenview_get_pine_inputs`): numbers, booleans, or text
    /// (an option's value, a source name such as `close`, a colour as `#RRGGBB` or `#RRGGBBAA`).
    /// Inputs left out run at their defaults.
    pub inputs: Option<InputArgs>,
    /// Whether the script is shown. Default true.
    pub visible: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePineInstanceParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// An `instance_id` from `degenview_list_chart_pine_instances`.
    #[schemars(length(max = 20))]
    pub instance_id: String,
    /// Input overrides to change, by input id, as for `degenview_add_pine_script_to_chart`.
    /// `null` puts an input back to its default; inputs not named keep their current value.
    pub inputs: Option<InputArgs>,
    /// Show or hide the script. A hidden script does not run.
    pub visible: Option<bool>,
}

fn parse_instance_id(raw: &str) -> Result<u64, CallToolResult> {
    parse_id(raw, "instance_id", "degenview_list_chart_pine_instances")
}

#[tool_router(router = pine_router, vis = "pub(in crate::mcp)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_list_pine_scripts",
        description = "List the Pine scripts in DegenView's Pine Studio: id, name, type (indicator, strategy, library), whether an unsaved draft exists, and the last compile status. No file paths are returned.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_pine_scripts(
        &self,
        Caller(client): Caller,
    ) -> Result<Json<PineScriptsResult>, CallToolResult> {
        self.pine_job(&client, Permission::PineScriptsList, |store| {
            pine::list_scripts(store)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_get_pine_script",
        description = "Read a Pine script's source: the saved text or the unsaved draft (never one in place of the other), with its SHA-256 source_hash and revision id.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_script(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<GetPineScriptParams>,
    ) -> Result<Json<PineScriptSource>, CallToolResult> {
        let id = clean_script_id(&params.script_id)?;
        let version = params.version.version();
        self.pine_job(&client, Permission::PineScriptsRead, move |store| {
            pine::get_script(store, &id, version)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_update_pine_script",
        description = "Replace a Pine script's DRAFT with new source text. Only the draft changes: the saved source, charts using the script and alert subscriptions are untouched, and nothing is run. The edit is checked against what the user sees now (their open editor, else the draft, else the saved source) using expected_revision; if that moved, it is refused with CONFLICT. The open Pine Studio editor shows the new text. Returns the new revision and the draft's compile diagnostics. Use degenview_save_pine_script to make it the saved source.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_pine_script(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<UpdatePineParams>,
    ) -> Result<Json<PineEditResult>, CallToolResult> {
        let id = clean_script_id(&params.script_id)?;
        if !pine::is_revision(&params.expected_revision) {
            return Err(invalid(
                "expected_revision must be the 64-character source_hash from degenview_get_pine_script",
            ));
        }
        if params.source.chars().count() > pine::MAX_EDIT_CHARS {
            return Err(failure(&McpError::ResourceLimit(format!(
                "source is over {} characters",
                pine::MAX_EDIT_CHARS
            ))));
        }
        let source = params.source.clone();
        let mut result = match self
            .bridge
            .call(
                &client,
                Permission::PineScriptsWrite,
                PineCommand::EditScript {
                    script_id: id.clone(),
                    expected_revision: params.expected_revision,
                    source: Some(params.source),
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineEdit(result)) => *result,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        // Diagnostics for the text that is now the draft: compiled off the app thread.
        let script_id = id.clone();
        let compiled = self
            .pine_job(&client, Permission::PineScriptsWrite, move |store| {
                let mut compiled = pine::compile_script(store, Target::Inline(source))?;
                compiled.script_id = Some(script_id);
                compiled.version = SourceVersion::Draft;
                Ok(compiled)
            })
            .await;
        attach_compile(&mut result, compiled);
        Ok(Json(result))
    }

    #[tool(
        name = "degenview_save_pine_script",
        description = "Save a Pine script's current draft over its saved source, as the editor's Save does (records a revision, keeps history). Needs expected_revision to match the draft you reviewed, and the draft must compile without errors (COMPILATION_FAILED otherwise; the draft is kept). Charts keep running the source they were applied with; alert subscriptions to this script will show 'script changed'. Saving cannot be undone from here (revision history stays in the Pine Studio). Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn save_pine_script(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<SavePineParams>,
    ) -> Result<Json<PineEditResult>, CallToolResult> {
        let id = clean_script_id(&params.script_id)?;
        if !pine::is_revision(&params.expected_revision) {
            return Err(invalid(
                "expected_revision must be the 64-character source_hash from degenview_get_pine_script",
            ));
        }
        let mut result = match self
            .bridge
            .call(
                &client,
                Permission::PineScriptsSave,
                PineCommand::EditScript {
                    script_id: id.clone(),
                    expected_revision: params.expected_revision,
                    source: None,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineEdit(result)) => *result,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        let compiled = self
            .pine_job(&client, Permission::PineScriptsSave, move |store| {
                pine::compile_script(store, Target::Script(id, Version::Saved))
            })
            .await;
        attach_compile(&mut result, compiled);
        Ok(Json(result))
    }

    #[tool(
        name = "degenview_compile_pine_script",
        description = "Compile a Pine script (saved or draft) or a source string with DegenView's own compiler and return its errors and warnings with codes and line/column positions. Compiling does not run the script: no trades, alerts, webhooks, saves or chart changes. A name nothing defines is reported as an error (PINE3050 variable, PINE3051 function). The check is lenient (a name declared anywhere counts, and members of builtin namespaces such as ta.* are judged only at run time), so a clean compile does not prove the script runs; apply it to a chart (degenview_add_pine_script_to_chart) and read its last-run error with degenview_get_pine_instance.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn compile_pine_script(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<CompilePineParams>,
    ) -> Result<Json<PineCompileResult>, CallToolResult> {
        let target = match (params.script_id, params.source) {
            (Some(id), None) => Target::Script(
                clean_script_id(&id)?,
                params
                    .version
                    .map_or(Version::Saved, McpScriptVersion::version),
            ),
            (None, Some(source)) if params.version.is_none() => {
                if source.chars().count() > MAX_INLINE_SOURCE_CHARS {
                    return Err(failure(&McpError::ResourceLimit(format!(
                        "source is over {MAX_INLINE_SOURCE_CHARS} characters"
                    ))));
                }
                Target::Inline(source)
            }
            (None, Some(_)) => {
                return Err(invalid("version applies to a script_id, not to source"));
            }
            (Some(_), Some(_)) => return Err(invalid("give script_id or source, not both")),
            (None, None) => return Err(invalid("give script_id or source")),
        };
        self.pine_job(&client, Permission::PineDiagnosticsRead, move |store| {
            pine::compile_script(store, target)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_get_pine_inputs",
        description = "List a Pine script's input definitions (id, title, type, default, min/max/step, options, group, tooltip) as the compiler declares them.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_inputs(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PineInputsParams>,
    ) -> Result<Json<PineInputsResult>, CallToolResult> {
        let id = clean_script_id(&params.script_id)?;
        let version = params
            .version
            .map_or(Version::Saved, McpScriptVersion::version);
        self.pine_job(&client, Permission::PineInputsRead, move |store| {
            pine::script_inputs(store, &id, version)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_list_chart_pine_instances",
        description = "List the Pine scripts applied to one chart: instance id, the script it came from, visibility, the source hash it runs, and whether its last run was ok, failed, or has not happened yet (scripts run only while their chart is drawn). The same script can appear more than once.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_chart_pine_instances(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<ChartPineParams>,
    ) -> Result<Json<AppliedPineList>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::PineScriptsList,
                PineCommand::ListInstances {
                    workspace_id,
                    chart_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartPine(list)) => Ok(Json(list)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_get_pine_instance",
        description = "Inspect one applied Pine script: its input overrides, style overrides, visibility, last-run status and error, and whether its source still matches the saved script it came from. Source text is not included (see degenview_get_pine_script).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_instance(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PineInstanceParams>,
    ) -> Result<Json<AppliedPineDetail>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        let mut detail = match self
            .bridge
            .call(
                &client,
                Permission::PineInputsRead,
                PineCommand::GetInstance {
                    workspace_id,
                    chart_id,
                    instance_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartPineDetail(detail)) => detail,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        detail.script_status = self
            .script_status(
                &client,
                Permission::PineInputsRead,
                detail.instance.script_id.clone(),
                detail.instance.source_hash.clone(),
            )
            .await;
        Ok(Json(detail))
    }

    #[tool(
        name = "degenview_get_pine_styles",
        description = "List the style options of an applied Pine script (each plot, line, fill, marker...) with the script's own default color and width and any overrides set on this instance. Options exist only once the script has run on its chart.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_styles(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PineInstanceParams>,
    ) -> Result<Json<PineStylesResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::PineStylesRead,
                PineCommand::GetStyles {
                    workspace_id,
                    chart_id,
                    instance_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineStyles(styles)) => Ok(Json(styles)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_get_pine_report",
        description = "Read the strategy backtest report a chart computed for an applied Pine strategy: metrics, settings, a page of closed trades, open trades and optionally the equity curve, with the instrument, timeframe, bar count, inputs and source hash it was produced with. If there is no report (not a strategy, not run yet, failed) the result says so; an old report is never passed off as current. Nothing is run by this call.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_report(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PineReportParams>,
    ) -> Result<Json<PineReportResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        let limit = params.trades_limit.unwrap_or(50);
        let offset = params.trades_offset.unwrap_or(0);
        if !(1..=200).contains(&limit) {
            return Err(invalid("trades_limit must be between 1 and 200"));
        }
        if offset > 1_000_000 {
            return Err(invalid("trades_offset must be at most 1000000"));
        }
        let mut report = match self
            .bridge
            .call(
                &client,
                Permission::PineReportsRead,
                PineCommand::GetReport {
                    workspace_id,
                    chart_id,
                    instance_id,
                    offset: offset as usize,
                    limit: limit as usize,
                    include_equity: params.include_equity.unwrap_or(false),
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineReport(report)) => *report,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if let Some(context) = report.context.as_mut() {
            context.script_status = self
                .script_status(
                    &client,
                    Permission::PineReportsRead,
                    context.script_id.clone(),
                    context.source_hash.clone(),
                )
                .await;
        }
        Ok(Json(report))
    }

    #[tool(
        name = "degenview_list_pine_alerts",
        description = "List the alert subscriptions of Pine scripts: script, market, timeframe, state (active, paused, script_changed, script_deleted) and note. DegenView keeps no log of fired script alerts, so no history is returned (event_history_available is false); reading never runs a script or sends anything.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_pine_alerts(
        &self,
        Caller(client): Caller,
    ) -> Result<Json<PineAlertsResult>, CallToolResult> {
        match self
            .bridge
            .call(
                &client,
                Permission::PineAlertsRead,
                PineCommand::ListAlerts,
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineAlerts(alerts)) => Ok(Json(alerts)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_add_pine_script_to_chart",
        description = "Apply a saved Pine indicator or strategy to a chart, as the app's Add to Chart does, optionally with input overrides. Uses the script's last saved source (save a draft first with degenview_save_pine_script). Refuses libraries and scripts that do not compile. The same script can be applied more than once. The script runs once its chart draws it, so check degenview_list_chart_pine_instances for run_status. The user can undo it in the app.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_pine_script_to_chart(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<AddPineToChartParams>,
    ) -> Result<Json<PineInstanceChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let id = clean_script_id(&params.script_id)?;
        let args = params.inputs.unwrap_or_default();
        let script = self
            .pine_job(
                &client,
                Permission::ChartsPineInstancesWrite,
                move |store| pine::script_to_apply(store, &id, &args),
            )
            .await?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsPineInstancesWrite,
                PineCommand::AddInstance {
                    workspace_id,
                    chart_id,
                    script: Box::new(script),
                    visible: params.visible.unwrap_or(true),
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineInstanceChanged(changed)) => Ok(Json(*changed)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_update_pine_instance",
        description = "Change a script applied to a chart: set or reset input overrides (null resets an input) and/or show or hide it. Inputs are checked against the source the instance was applied with. Give inputs, visible, or both. Refused with CONFLICT while a script alert is armed on the instance and the inputs would change (the alert follows the instance's inputs); showing or hiding is always allowed. To run a newer saved version of the script, remove the instance and apply it again.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_pine_instance(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<UpdatePineInstanceParams>,
    ) -> Result<Json<PineInstanceChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        let inputs = params.inputs.filter(|inputs| !inputs.is_empty());
        if inputs.is_none() && params.visible.is_none() {
            return Err(invalid("give inputs (not empty), visible, or both"));
        }
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsPineInstancesWrite,
                PineCommand::UpdateInstance {
                    workspace_id,
                    chart_id,
                    instance_id,
                    inputs,
                    visible: params.visible,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineInstanceChanged(changed)) => Ok(Json(*changed)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_remove_pine_instance",
        description = "Take a script off a chart. The saved script is left alone, and the user can undo the removal in the app. Refused with CONFLICT while a script alert is armed on the instance, because removing it would silence that alert. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn remove_pine_instance(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<PineInstanceParams>,
    ) -> Result<Json<PineInstanceChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsPineInstancesWrite,
                PineCommand::RemoveInstance {
                    workspace_id,
                    chart_id,
                    instance_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineInstanceChanged(changed)) => Ok(Json(*changed)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}

/// Aborts a blocking task when dropped. A no-op once the task has started or finished.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl DegenViewMcp {
    /// Runs a script-library read or compile off the async workers and off the app thread, at
    /// most `PINE_JOBS` at a time. The permission is checked again once the work is done: a
    /// credential revoked meanwhile gets nothing.
    async fn pine_job<T: Send + 'static>(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        job: impl FnOnce(&mut ScriptStore) -> Result<T, McpError> + Send + 'static,
    ) -> Result<T, CallToolResult> {
        let Some(permit) = self.shared.try_pine_job() else {
            return Err(failure(&McpError::ResourceLimit(
                "Too many script requests are running; try again shortly".into(),
            )));
        };
        let shared = self.shared.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut store = shared.script_store().ok_or_else(|| {
                McpError::AppUnavailable("The script library is unavailable".into())
            })?;
            job(&mut store)
        });
        // If this call is dropped (the client went away), a job still waiting for a thread is
        // cancelled. One already running finishes and frees its permit; nothing here can stop a
        // compile part-way.
        let _abort_if_dropped = AbortOnDrop(handle.abort_handle());
        let outcome = handle.await.map_err(|_| {
            failure(&McpError::AppUnavailable(
                "The request was interrupted".into(),
            ))
        })?;
        if !self.shared.allows(&client.id, permission) {
            return Err(failure(&McpError::PermissionDenied(
                "This credential no longer has the required permission".into(),
            )));
        }
        outcome.map_err(|error| failure(&error))
    }

    /// Whether an applied script's source still matches the saved script it came from. The
    /// answer is a courtesy: if the library cannot be read it is `unknown`, not an error.
    async fn script_status(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        script_id: Option<String>,
        applied_hash: String,
    ) -> ScriptStatus {
        self.pine_job(client, permission, move |store| {
            pine::script_status(store, script_id.as_deref(), &applied_hash)
        })
        .await
        .unwrap_or(ScriptStatus::Unknown)
    }
}
