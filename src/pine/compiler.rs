//! Source text → `CompiledProgram`: lex, parse, read the declaration, fold constants, collect
//! inputs, then run the structural and type checks. Ported from `PineCompiler*.swift` +
//! `PineLibraryLinker.swift`.

use std::collections::{HashMap, HashSet};

use super::alert_frequency::AlertFrequency;
use super::ast::{Argument, BinaryOperator, EnumMember, Expression, Statement};
use super::builtins;
use super::calendar;
use super::input_activity;
use super::input_arguments::InputArguments;
use super::operators;
use super::outputs::{LineExtend, LineStyle, MarkerLocation, MarkerShape, Size, TablePosition};
use super::program::{
    CompiledProgram, DeclarationMetadata, Import, InputDefinition, InputSchema, InputValue,
    ScriptType,
};
use super::runtime_value::RuntimeValue;
use super::session::Session;
use super::strategy::StrategySettings;
use super::structure_validator;
use super::symbol_catalog;
use super::timestamp;
use super::traversal::assigned_names;
use crate::pine::{Diagnostic, DiagnosticCategory, Lexer, Limits, Parser, SourceRange, ValueType};

/// Where an `import user/Library/version` finds its source. The compiler never reads files: the
/// app supplies the libraries it has, and tests supply their own.
pub trait LibraryResolver {
    fn source_for_library(&self, path: &str) -> Option<String>;
}

/// The default: no library can be found.
pub struct NoLibraries;

impl LibraryResolver for NoLibraries {
    fn source_for_library(&self, _path: &str) -> Option<String> {
        None
    }
}

const SUPPORTED_VERSION: &str = "6";

pub fn compile(source: &str, limits: Limits, libraries: &dyn LibraryResolver) -> CompiledProgram {
    compile_internal(source, limits, libraries, &[])
}

fn compile_internal(
    source: &str,
    limits: Limits,
    libraries: &dyn LibraryResolver,
    import_stack: &[String],
) -> CompiledProgram {
    let normalized = normalize_line_endings(source);
    let mut diagnostics = Vec::new();
    let version = declared_version(&normalized, &mut diagnostics);

    let lexed = Lexer::with_limits(&normalized, limits).lex();
    diagnostics.extend(lexed.diagnostics);

    let scanned = scan_imports(&normalized);
    diagnostics.extend(scanned.diagnostics);

    let import_aliases: HashSet<String> = scanned
        .declarations
        .iter()
        .map(|d| d.alias.clone())
        .collect();
    let mut parser = Parser::new(lexed.tokens, limits, import_aliases.clone());
    let (statements, parse_diagnostics) = parser.parse();
    diagnostics.extend(parse_diagnostics);

    let linked = link_imports(&scanned.declarations, libraries, limits, import_stack);
    diagnostics.extend(linked.diagnostics);
    diagnostics.extend(validate_usage(&statements, &linked.imports));

    let environment = constant_environment(&statements);
    let metadata = declaration_metadata(
        &statements,
        version.as_deref(),
        &environment,
        &mut diagnostics,
    );
    if metadata.script_type != ScriptType::Library {
        diagnostics.extend(parser.export_ranges().iter().map(|range| {
            Diagnostic::error(
                "PINE3037",
                DiagnosticCategory::Semantic,
                "export is only allowed in a library().",
                *range,
            )
        }));
    }
    let mut schema = InputSchema::default();
    collect_inputs(&statements, &environment, &mut schema, &mut diagnostics);

    diagnostics.extend(structure_validate(
        &statements,
        parser.method_names(),
        &normalized,
    ));
    diagnostics.extend(super::alert_call::validate(&statements));
    diagnostics.extend(super::log_call::validate(&statements));
    let has_early_error = diagnostics.iter().any(|d| {
        matches!(
            d.category,
            DiagnosticCategory::Lexical | DiagnosticCategory::Syntax
        )
    });
    if !has_early_error {
        diagnostics.extend(super::type_checker::TypeChecker::check(&statements));
        diagnostics.extend(super::ignored_features::validate(&statements));
        diagnostics.extend(super::name_check::validate(
            &statements,
            &import_aliases,
            parser.method_names(),
        ));
    }

    CompiledProgram {
        source: normalized,
        statements,
        declaration: metadata,
        input_schema: schema,
        diagnostics,
        method_names: parser.method_names().clone(),
        exported_names: parser.exported_names().clone(),
        imports: linked.imports,
        statement_ends: parser.statement_ends(),
    }
}

/// Text copied from browsers can contain CR-only or Unicode line separators.
pub fn normalize_line_endings(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}'], "\n")
}

/// The version in a `//@version=6` comment. TradingView also takes blanks around the parts:
/// `// @version = 6`.
fn version_annotation(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("//")?.trim_start();
    let rest = rest.strip_prefix("@version")?.trim_start();
    Some(rest.strip_prefix('=')?.trim().to_owned())
}

fn declared_version(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<String> {
    let versions: Vec<String> = source.lines().filter_map(version_annotation).collect();
    let first = versions.first().cloned();
    match first {
        None => {
            diagnostics.push(Diagnostic::error(
                "PINE0001",
                DiagnosticCategory::Semantic,
                "Missing //@version=6 annotation.",
                SourceRange::ZERO,
            ));
            None
        }
        Some(version) => {
            if version != SUPPORTED_VERSION {
                diagnostics.push(Diagnostic::error(
                    "PINE0002",
                    DiagnosticCategory::Semantic,
                    format!(
                        "Pine Script version {version} is not supported; this runtime targets v6."
                    ),
                    SourceRange::ZERO,
                ));
            }
            Some(version)
        }
    }
}

// MARK: Declaration

struct ScriptDeclaration {
    script_type: ScriptType,
    name: String,
    arguments: Vec<Argument>,
    range: SourceRange,
}

fn script_declarations(statements: &[Statement]) -> Vec<ScriptDeclaration> {
    statements
        .iter()
        .filter_map(|statement| {
            let Statement::Expression(Expression::Call {
                name,
                arguments,
                range,
                ..
            }) = statement
            else {
                return None;
            };
            let script_type = match name.as_str() {
                "indicator" => ScriptType::Indicator,
                "strategy" => ScriptType::Strategy,
                "library" => ScriptType::Library,
                _ => return None,
            };
            Some(ScriptDeclaration {
                script_type,
                name: name.clone(),
                arguments: arguments.clone(),
                range: *range,
            })
        })
        .collect()
}

const COMMON_ARGUMENTS: [&str; 18] = [
    "calc_bars_count",
    "timeframe",
    "timeframe_gaps",
    "title",
    "shorttitle",
    "overlay",
    "format",
    "precision",
    "max_bars_back",
    "max_lines_count",
    "max_labels_count",
    "max_boxes_count",
    "max_polylines_count",
    "behind_chart",
    "explicit_plot_zorder",
    "dynamic_requests",
    "scale",
    "backadjustment",
];

const STRATEGY_ARGUMENTS: [&str; 18] = [
    "use_bar_magnifier",
    "fill_orders_on_standard_ohlc",
    "backtest_fill_limits_assumption",
    "risk_free_rate",
    "initial_capital",
    "default_qty_type",
    "default_qty_value",
    "commission_type",
    "commission_value",
    "slippage",
    "pyramiding",
    "currency",
    "process_orders_on_close",
    "calc_on_order_fills",
    "calc_on_every_tick",
    "close_entries_rule",
    "margin_long",
    "margin_short",
];

fn declaration_metadata(
    statements: &[Statement],
    version: Option<&str>,
    environment: &HashMap<String, RuntimeValue>,
    diagnostics: &mut Vec<Diagnostic>,
) -> DeclarationMetadata {
    let declarations = script_declarations(statements);
    if declarations.len() != 1 {
        diagnostics.push(Diagnostic::error(
            "PINE3001",
            DiagnosticCategory::Semantic,
            "A script must contain exactly one indicator(), strategy(), or library() declaration.",
            declarations.first().map_or(SourceRange::ZERO, |d| d.range),
        ));
    }
    let mut metadata = DeclarationMetadata {
        script_type: declarations
            .first()
            .map_or(ScriptType::Indicator, |d| d.script_type),
        pine_version: version.and_then(|v| v.parse().ok()),
        ..DeclarationMetadata::default()
    };
    if metadata.script_type == ScriptType::Strategy {
        metadata.strategy = Some(StrategySettings::default());
    }
    if let Some(declaration) = declarations.first() {
        apply_declaration(declaration, environment, &mut metadata, diagnostics);
    }
    metadata
}

fn apply_declaration(
    declaration: &ScriptDeclaration,
    environment: &HashMap<String, RuntimeValue>,
    metadata: &mut DeclarationMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(first) = declaration.arguments.first()
        && first.name.as_deref().is_none_or(|n| n == "title")
        && let Some(RuntimeValue::String(title)) = constant_value(&first.value, environment)
    {
        metadata.title = title;
        return apply_remaining(declaration, environment, metadata, diagnostics);
    }
    diagnostics.push(Diagnostic::error(
        "PINE3002",
        DiagnosticCategory::Semantic,
        format!("{}() title must be a constant string.", declaration.name),
        declaration.range,
    ));
    apply_remaining(declaration, environment, metadata, diagnostics);
}

fn apply_remaining(
    declaration: &ScriptDeclaration,
    environment: &HashMap<String, RuntimeValue>,
    metadata: &mut DeclarationMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let supported: HashSet<&str> = {
        let mut set: HashSet<&str> = COMMON_ARGUMENTS.iter().copied().collect();
        if metadata.script_type == ScriptType::Strategy {
            set.extend(STRATEGY_ARGUMENTS);
        }
        set
    };
    for argument in &declaration.arguments {
        let Some(name) = argument.name.as_deref() else {
            continue;
        };
        if !supported.contains(name) {
            diagnostics.push(Diagnostic::error(
                "PINE9001",
                DiagnosticCategory::Unsupported,
                format!("Unsupported {}() argument '{name}'.", declaration.name),
                argument.name_range.unwrap_or(argument.value.range()),
            ));
            continue;
        }
        let Some(value) = constant_value(&argument.value, environment) else {
            continue;
        };
        apply_argument(name, value, metadata);
    }
}

fn apply_argument(name: &str, value: RuntimeValue, metadata: &mut DeclarationMetadata) {
    match (name, &value) {
        ("shorttitle", RuntimeValue::String(v)) => metadata.short_title = Some(v.clone()),
        ("overlay", RuntimeValue::Bool(v)) => metadata.overlay = *v,
        ("format", RuntimeValue::String(v)) => metadata.format = Some(v.clone()),
        ("precision", RuntimeValue::Int(v)) => metadata.precision = Some(*v),
        ("max_bars_back", RuntimeValue::Int(v)) => metadata.max_bars_back = Some(*v),
        ("max_lines_count", RuntimeValue::Int(v)) => metadata.max_lines_count = Some(*v),
        ("max_labels_count", RuntimeValue::Int(v)) => metadata.max_labels_count = Some(*v),
        ("max_boxes_count", RuntimeValue::Int(v)) => metadata.max_boxes_count = Some(*v),
        ("max_polylines_count", RuntimeValue::Int(v)) => metadata.max_polylines_count = Some(*v),
        ("behind_chart", RuntimeValue::Bool(v)) => metadata.behind_chart = Some(*v),
        _ => apply_strategy_setting(name, value, metadata),
    }
}

fn apply_strategy_setting(name: &str, value: RuntimeValue, metadata: &mut DeclarationMetadata) {
    let Some(settings) = metadata.strategy.as_mut() else {
        return;
    };
    match (name, &value) {
        ("initial_capital", _) => {
            if let Some(n) = value.number().filter(|n| *n > 0.0) {
                settings.initial_capital = n;
            }
        }
        ("default_qty_type", RuntimeValue::String(v)) => {
            settings.quantity_type = match v.as_str() {
                "strategy.cash" => super::strategy::QuantityType::Cash,
                "strategy.percent_of_equity" => super::strategy::QuantityType::PercentOfEquity,
                _ => super::strategy::QuantityType::Fixed,
            };
        }
        ("default_qty_value", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0) {
                settings.quantity_value = n;
            }
        }
        ("commission_type", RuntimeValue::String(v)) => {
            settings.commission_type = match v.as_str() {
                "strategy.commission.cash_per_order" => {
                    super::strategy::CommissionType::CashPerOrder
                }
                "strategy.commission.cash_per_contract" => {
                    super::strategy::CommissionType::CashPerContract
                }
                _ => super::strategy::CommissionType::Percent,
            };
        }
        ("commission_value", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0) {
                settings.commission_value = n;
            }
        }
        ("slippage", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0 && *n < 1e6) {
                settings.slippage = n as usize;
            }
        }
        ("pyramiding", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0 && *n < 1e6) {
                settings.pyramiding = n as usize;
            }
        }
        ("currency", RuntimeValue::String(v)) => settings.currency = Some(v.clone()),
        ("process_orders_on_close", RuntimeValue::Bool(v)) => settings.process_orders_on_close = *v,
        ("calc_on_every_tick", RuntimeValue::Bool(v)) => settings.calc_on_every_tick = *v,
        _ => {}
    }
}

// MARK: Constants

fn constant_environment(statements: &[Statement]) -> HashMap<String, RuntimeValue> {
    let reassigned = assigned_names(statements);
    let mut environment = HashMap::new();
    for statement in statements {
        let Statement::Declaration {
            name, mode, value, ..
        } = statement
        else {
            continue;
        };
        if *mode != super::ast::DeclarationMode::Ordinary || reassigned.contains(name) {
            continue;
        }
        if let Some(value) = constant_value(value, &environment) {
            environment.insert(name.clone(), value);
        }
    }
    environment
}

pub(super) fn constant_value(
    expression: &Expression,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<RuntimeValue> {
    match expression {
        Expression::Literal(value, _) => {
            let value = match value {
                super::ast::Literal::Int(n) => RuntimeValue::Int(*n),
                super::ast::Literal::Float(n) => RuntimeValue::Float(*n),
                super::ast::Literal::Bool(b) => RuntimeValue::Bool(*b),
                super::ast::Literal::String(s) => RuntimeValue::String(s.clone()),
                super::ast::Literal::Color(c) => RuntimeValue::Color(*c),
                super::ast::Literal::Na => return None,
            };
            Some(value)
        }
        Expression::Identifier(name, _) => {
            if let Some(value) = environment.get(name) {
                return Some(value.clone());
            }
            if let Some(value) = builtins::constant(name) {
                return Some(value);
            }
            if let Some(color) = builtins::color(name) {
                return Some(RuntimeValue::Color(color));
            }
            // A named constant is the string of its own name (`size.small`, `shape.circle`).
            if is_named_constant(name) {
                Some(RuntimeValue::String(name.clone()))
            } else {
                None
            }
        }
        Expression::Unary {
            op: super::ast::UnaryOperator::Negate,
            operand,
            ..
        } => match constant_value(operand, environment)? {
            RuntimeValue::Int(n) => Some(RuntimeValue::Int(0i64.wrapping_sub(n))),
            RuntimeValue::Float(n) => Some(RuntimeValue::Float(-n)),
            _ => None,
        },
        Expression::Unary {
            op: super::ast::UnaryOperator::Not,
            operand,
            ..
        } => Session::truth(constant_value(operand, environment)?)
            .map(|value| RuntimeValue::Bool(!value)),
        Expression::Binary { lhs, op, rhs, .. } => {
            let a = constant_value(lhs, environment)?;
            let b = constant_value(rhs, environment)?;
            let value = match op {
                BinaryOperator::And => RuntimeValue::Bool(Session::truth(a)? && Session::truth(b)?),
                BinaryOperator::Or => RuntimeValue::Bool(Session::truth(a)? || Session::truth(b)?),
                _ => operators::apply(*op, &a, &b),
            };
            if value == RuntimeValue::Na {
                None
            } else {
                Some(value)
            }
        }
        Expression::Ternary {
            condition,
            when_true,
            when_false,
            ..
        } => {
            let chosen = if Session::truth(constant_value(condition, environment)?)? {
                when_true
            } else {
                when_false
            };
            constant_value(chosen, environment)
        }
        Expression::Call {
            name, arguments, ..
        } if name == "timestamp" => constant_timestamp(arguments, environment),
        Expression::Call {
            name, arguments, ..
        } if name == "color.new" || name == "color.rgb" => {
            constant_color(expression, environment).map(RuntimeValue::Color)
        }
        _ => None,
    }
}

fn constant_timestamp(
    arguments: &[Argument],
    environment: &HashMap<String, RuntimeValue>,
) -> Option<RuntimeValue> {
    let mut positional = Vec::new();
    let mut named = HashMap::new();
    for argument in arguments {
        let value = constant_value(&argument.value, environment)?;
        if let Some(name) = &argument.name {
            named.insert(name.clone(), value);
        } else {
            positional.push(value);
        }
    }
    timestamp::evaluate(&positional, &named).map(RuntimeValue::Int)
}

fn constant_color(
    expression: &Expression,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<u32> {
    match expression {
        Expression::Literal(super::ast::Literal::Color(rgba), _) => Some(*rgba),
        Expression::Identifier(_, _) => match constant_value(expression, environment)? {
            RuntimeValue::Color(rgba) => Some(rgba),
            _ => None,
        },
        Expression::Call {
            name, arguments, ..
        } if name == "color.new" => {
            if arguments.len() < 2 {
                return None;
            }
            let base = constant_color(&arguments[0].value, environment)?;
            let transparency = constant_number(&arguments[1].value, environment)?;
            Some(builtins::with_transparency(base, transparency))
        }
        Expression::Call {
            name, arguments, ..
        } if name == "color.rgb" => {
            let numbers: Vec<f64> = arguments
                .iter()
                .filter_map(|a| constant_number(&a.value, environment))
                .collect();
            if numbers.len() != arguments.len() || numbers.len() < 3 {
                return None;
            }
            Some(builtins::rgb(
                numbers[0],
                numbers[1],
                numbers[2],
                numbers.get(3).copied().unwrap_or(0.0),
            ))
        }
        _ => None,
    }
}

fn constant_number(
    expression: &Expression,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<f64> {
    match expression {
        Expression::Literal(value, _) => match value {
            super::ast::Literal::Int(n) => Some(*n as f64),
            super::ast::Literal::Float(n) => Some(*n),
            _ => None,
        },
        Expression::Identifier(_, _) => constant_value(expression, environment)?.number(),
        Expression::Unary {
            op: super::ast::UnaryOperator::Negate,
            operand,
            ..
        } => constant_number(operand, environment).map(|n| -n),
        _ => None,
    }
}

// MARK: Inputs

fn collect_inputs(
    statements: &[Statement],
    environment: &HashMap<String, RuntimeValue>,
    schema: &mut InputSchema,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let enums = enum_declarations(statements);
    for statement in statements {
        if let Statement::Declaration { name, value, .. } = statement
            && let Expression::Call {
                name: fn_name,
                arguments,
                range,
                ..
            } = value
            && (fn_name.starts_with("input.") || fn_name == "input")
        {
            if fn_name == "input.enum" {
                if let Some(mut input) = enum_input(name, arguments, &enums, environment) {
                    input.active =
                        input_active(fn_name, arguments, environment, schema, diagnostics);
                    schema.inputs.push(input);
                } else {
                    diagnostics.push(Diagnostic::error(
                        "PINE3010",
                        DiagnosticCategory::Semantic,
                        "input.enum requires a member of a declared enum as its default value.",
                        *range,
                    ));
                }
                continue;
            }
            let inputs = InputArguments::new(fn_name, arguments);
            let constant = |expression: &Expression| constant_value(expression, environment);
            let Some(default) = inputs.find("defval") else {
                diagnostics.push(Diagnostic::error(
                    "PINE3010",
                    DiagnosticCategory::Semantic,
                    format!("{fn_name} requires a constant default value."),
                    *range,
                ));
                continue;
            };
            let Some(default_value) = input_value(&default.value, fn_name, environment) else {
                diagnostics.push(Diagnostic::error(
                    "PINE3010",
                    DiagnosticCategory::Semantic,
                    format!("{fn_name} requires a constant default value."),
                    *range,
                ));
                continue;
            };
            let active = input_active(fn_name, arguments, environment, schema, diagnostics);
            schema.inputs.push(InputDefinition {
                id: name.clone(),
                kind: builtins::input_kind(fn_name, &default_value),
                control: builtins::input_control(fn_name),
                default_value,
                title: input_title(&inputs, &constant),
                tooltip: inputs.string("tooltip", &constant),
                group: inputs.string("group", &constant),
                inline: inputs.string("inline", &constant),
                confirm: inputs.flag("confirm"),
                min_value: inputs.number("minval", &constant),
                max_value: inputs.number("maxval", &constant),
                step: inputs.number("step", &constant),
                options: input_options(&inputs, fn_name, environment),
                option_titles: None,
                active,
            });
        }
        if let Statement::Conditional {
            when_true,
            when_false,
            ..
        } = statement
        {
            collect_inputs(when_true, environment, schema, diagnostics);
            collect_inputs(when_false, environment, schema, diagnostics);
        }
    }
}

/// When the call's `active` lets the user edit the input, as a condition on other inputs. None
/// when the call sets none, and when what it sets cannot be known before the script runs: that is
/// warned about, and the input stays editable.
fn input_active(
    function: &str,
    arguments: &[Argument],
    environment: &HashMap<String, RuntimeValue>,
    schema: &InputSchema,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Expression> {
    let argument = InputArguments::new(function, arguments).find("active")?;
    let Some(condition) = input_activity::compile(argument, environment, &schema.inputs) else {
        diagnostics.push(super::ignored_features::warning(
            "PINE7001",
            format!(
                "{function}(): `active` is ignored. Only inputs declared above it and constants can set it, so the input can always be edited."
            ),
            argument.name_range.unwrap_or(argument.value.range()),
        ));
        return None;
    };
    // An input that is always active needs no condition.
    (condition != Expression::Literal(super::ast::Literal::Bool(true), condition.range()))
        .then_some(condition)
}

fn enum_declarations(statements: &[Statement]) -> HashMap<String, Vec<EnumMember>> {
    let mut enums = HashMap::new();
    for statement in statements {
        if let Statement::EnumDeclaration { name, members, .. } = statement {
            enums.insert(name.clone(), members.clone());
        }
    }
    enums
}

fn enum_input(
    variable: &str,
    args: &[Argument],
    enums: &HashMap<String, Vec<EnumMember>>,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<InputDefinition> {
    let inputs = InputArguments::new("input.enum", args);
    let constant = |expression: &Expression| constant_value(expression, environment);
    let first = inputs.find("defval")?;
    let (initial_name, _) = enum_member(&first.value, enums)?;
    let mut choices: Vec<(String, EnumMember)> = Vec::new();
    if let Some(listed) = inputs.find("options") {
        if let Expression::Tuple(values, _) = &listed.value {
            choices = values
                .iter()
                .filter_map(|v| enum_member(v, enums))
                .collect();
        }
    } else if let Some((prefix, _)) = initial_name.split_once('.')
        && let Some(all) = enums.get(prefix)
    {
        choices = all
            .iter()
            .map(|m| (format!("{prefix}.{}", m.name), m.clone()))
            .collect();
    }
    Some(InputDefinition {
        id: variable.to_owned(),
        kind: ValueType::String,
        default_value: InputValue::String(initial_name.clone()),
        control: super::program::InputControl::Standard,
        title: input_title(&inputs, &constant),
        tooltip: inputs.string("tooltip", &constant),
        group: inputs.string("group", &constant),
        inline: inputs.string("inline", &constant),
        confirm: inputs.flag("confirm"),
        min_value: None,
        max_value: None,
        step: None,
        options: Some(
            choices
                .iter()
                .map(|(n, _)| InputValue::String(n.clone()))
                .collect(),
        ),
        option_titles: Some(choices.iter().map(|(_, m)| m.title.clone()).collect()),
        active: None,
    })
}

fn enum_member(
    expression: &Expression,
    enums: &HashMap<String, Vec<EnumMember>>,
) -> Option<(String, EnumMember)> {
    let Expression::Identifier(full, _) = expression else {
        return None;
    };
    let (prefix, member_name) = full.split_once('.')?;
    let member = enums
        .get(prefix)?
        .iter()
        .find(|m| m.name == member_name)?
        .clone();
    Some((full.clone(), member))
}

/// The input's title: `title`, in the second place when passed by position.
fn input_title(
    inputs: &InputArguments,
    constant: &dyn Fn(&Expression) -> Option<RuntimeValue>,
) -> Option<String> {
    inputs.string("title", constant)
}

/// The `options` the call lists, as values; None when it lists none or one of them is not constant.
fn input_options(
    inputs: &InputArguments,
    function: &str,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<Vec<InputValue>> {
    let Expression::Tuple(values, _) = &inputs.find("options")?.value else {
        return None;
    };
    let options: Vec<InputValue> = values
        .iter()
        .filter_map(|v| input_value(v, function, environment))
        .collect();
    (options.len() == values.len() && !options.is_empty()).then_some(options)
}

fn input_value(
    expression: &Expression,
    function: &str,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<InputValue> {
    if let Expression::Identifier(name, _) = expression
        && (function == "input.source"
            || (function == "input"
                && super::builtin_types::identifier(name).kind() == Some(ValueType::Float)))
    {
        return Some(InputValue::Source(name.clone()));
    }
    match constant_value(expression, environment)? {
        RuntimeValue::Int(value) => Some(InputValue::Int(value)),
        RuntimeValue::Float(value) => Some(InputValue::Float(value)),
        RuntimeValue::Bool(value) => Some(InputValue::Bool(value)),
        RuntimeValue::String(value) => Some(InputValue::String(value)),
        RuntimeValue::Color(value) => Some(InputValue::Color(value)),
        _ => None,
    }
}

// MARK: Libraries

struct ImportDeclaration {
    path: String,
    alias: String,
    range: SourceRange,
}

struct ScanResult {
    declarations: Vec<ImportDeclaration>,
    diagnostics: Vec<Diagnostic>,
}

fn scan_imports(source: &str) -> ScanResult {
    let mut declarations: Vec<ImportDeclaration> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut offset = 0usize;
    for (index, line) in source.split('\n').enumerate() {
        let line_len = line.encode_utf16().count();
        let range = SourceRange::new(
            crate::pine::SourcePosition::new(index + 1, 1, offset),
            crate::pine::SourcePosition::new(index + 1, line_len + 1, offset + line_len),
        );
        offset += line_len + 1;
        if !line.starts_with("import ") && !line.starts_with("import\t") {
            continue;
        }
        let Some((path, alias)) = parse_import_line(line) else {
            diagnostics.push(Diagnostic::error(
                "PINE3042",
                DiagnosticCategory::Semantic,
                "An import reads: import user/Library/version [as alias].",
                range,
            ));
            continue;
        };
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 3
            || parts.iter().any(|p| p.is_empty())
            || !parts[2].chars().all(|c| c.is_ascii_digit())
        {
            diagnostics.push(Diagnostic::error(
                "PINE3042",
                DiagnosticCategory::Semantic,
                format!(
                    "'{path}' is not a library path: expected user/Library/version, e.g. me/Math/1."
                ),
                range,
            ));
            continue;
        }
        let alias = alias.unwrap_or_else(|| parts[1].to_owned());
        if declarations
            .iter()
            .any(|d| d.alias.as_str() == alias.as_str())
            || RESERVED_WORDS.contains(&alias.as_str())
        {
            diagnostics.push(Diagnostic::error(
                "PINE3045",
                DiagnosticCategory::Semantic,
                format!("'{alias}' is already used and cannot be an import alias."),
                range,
            ));
        } else {
            declarations.push(ImportDeclaration { path, alias, range });
        }
    }
    ScanResult {
        declarations,
        diagnostics,
    }
}

fn parse_import_line(line: &str) -> Option<(String, Option<String>)> {
    let rest = line
        .strip_prefix("import ")
        .or_else(|| line.strip_prefix("import\t"))?;
    let rest = rest.split("//").next().unwrap_or(rest).trim_end();
    let mut parts = rest.split_whitespace();
    let path = parts.next()?.to_owned();
    let alias = match parts.next() {
        None => None,
        Some("as") => Some(parts.next()?.to_owned()),
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((path, alias))
}

const MAXIMUM_DEPTH: usize = 8;
const RESERVED_WORDS: [&str; 6] = ["import", "export", "method", "type", "enum", "as"];

fn link_imports(
    declarations: &[ImportDeclaration],
    resolver: &dyn LibraryResolver,
    limits: Limits,
    stack: &[String],
) -> LinkResult {
    let mut imports = Vec::new();
    let mut diagnostics = Vec::new();
    for declaration in declarations {
        let fail = |code: &str,
                    message: String,
                    imports: &mut Vec<Import>,
                    diagnostics: &mut Vec<Diagnostic>| {
            diagnostics.push(Diagnostic::error(
                code,
                DiagnosticCategory::Semantic,
                message,
                declaration.range,
            ));
            imports.push(Import {
                path: declaration.path.clone(),
                alias: declaration.alias.clone(),
                range: declaration.range,
                library: None,
            });
        };
        if stack.contains(&declaration.path) {
            fail(
                "PINE3041",
                format!(
                    "Library '{}' imports itself, directly or through another library.",
                    declaration.path
                ),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        }
        if stack.len() >= MAXIMUM_DEPTH {
            fail(
                "PINE3047",
                format!("Libraries import each other more than {MAXIMUM_DEPTH} levels deep."),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        }
        let Some(source) = resolver.source_for_library(&declaration.path) else {
            fail(
                "PINE3040",
                format!("Library '{}' was not found.", declaration.path),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        };
        let mut stack = stack.to_vec();
        stack.push(declaration.path.clone());
        let program = compile_internal(&source, limits, resolver, &stack);
        if program.declaration.script_type != ScriptType::Library {
            fail(
                "PINE3043",
                format!(
                    "'{}' is not a library: it does not declare library().",
                    declaration.path
                ),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        }
        if let Some(problem) = program
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::pine::DiagnosticSeverity::Error)
        {
            if problem.code == "PINE3041" || problem.code == "PINE3047" {
                fail(
                    &problem.code,
                    problem.message.clone(),
                    &mut imports,
                    &mut diagnostics,
                );
            } else {
                fail(
                    "PINE3043",
                    format!(
                        "Library '{}' has errors (line {}: {}).",
                        declaration.path, problem.range.start.line, problem.message
                    ),
                    &mut imports,
                    &mut diagnostics,
                );
            }
            continue;
        }
        imports.push(Import {
            path: declaration.path.clone(),
            alias: declaration.alias.clone(),
            range: declaration.range,
            library: Some(Box::new(program)),
        });
    }
    LinkResult {
        imports,
        diagnostics,
    }
}

struct LinkResult {
    imports: Vec<Import>,
    diagnostics: Vec<Diagnostic>,
}

/// The namespaces whose members are builtin functions or variables.
pub(super) fn is_builtin_namespace(alias: &str) -> bool {
    matches!(
        alias,
        "ta" | "math"
            | "str"
            | "array"
            | "matrix"
            | "map"
            | "color"
            | "request"
            | "ticker"
            | "timeframe"
            | "syminfo"
            | "input"
            | "strategy"
            | "chart"
            | "barstate"
            | "session"
            | "line"
            | "label"
            | "box"
            | "table"
            | "linefill"
            | "polyline"
            | "log"
            | "runtime"
            | "alert"
    )
}

fn validate_usage(statements: &[Statement], imports: &[Import]) -> Vec<Diagnostic> {
    let libraries: HashMap<&str, &CompiledProgram> = imports
        .iter()
        .filter_map(|entry| {
            entry
                .library
                .as_deref()
                .map(|library| (entry.alias.as_str(), library))
        })
        .collect();
    if libraries.is_empty() {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    super::traversal::for_each_call(statements, &mut |name, range| {
        let Some((alias, rest)) = name.split_once('.') else {
            return;
        };
        let Some(library) = libraries.get(alias) else {
            return;
        };
        let member = rest.split('.').next().unwrap_or(rest);
        // A library imported under a builtin namespace's name (`TradingView/ta/14` as `ta`) only
        // shadows what it exports: any other `ta.*` is the builtin.
        if !library.exported_names.contains(member) && !is_builtin_namespace(alias) {
            diagnostics.push(Diagnostic::error(
                "PINE3044",
                DiagnosticCategory::Semantic,
                format!("'{alias}' does not export '{member}'."),
                range,
            ));
        }
    });
    diagnostics
}

// MARK: Named constants and structure (reduced until the symbol catalog + validator land)

pub fn is_named_constant(name: &str) -> bool {
    if builtins::color(name).is_some() || builtins::constant(name).is_some() {
        return true;
    }
    if Size::NAMED.iter().any(|s| s.pine_name() == Some(name)) {
        return true;
    }
    let line_styles = [
        LineStyle::Solid,
        LineStyle::Dotted,
        LineStyle::Dashed,
        LineStyle::ArrowLeft,
        LineStyle::ArrowRight,
        LineStyle::ArrowBoth,
    ];
    if line_styles.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let extends = [
        LineExtend::None,
        LineExtend::Left,
        LineExtend::Right,
        LineExtend::Both,
    ];
    if extends.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let locations = [
        MarkerLocation::AboveBar,
        MarkerLocation::BelowBar,
        MarkerLocation::Top,
        MarkerLocation::Bottom,
        MarkerLocation::Absolute,
    ];
    if locations.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let positions = [
        TablePosition::TopLeft,
        TablePosition::TopCenter,
        TablePosition::TopRight,
        TablePosition::MiddleLeft,
        TablePosition::MiddleCenter,
        TablePosition::MiddleRight,
        TablePosition::BottomLeft,
        TablePosition::BottomCenter,
        TablePosition::BottomRight,
    ];
    if positions.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let shapes = [
        MarkerShape::Xcross,
        MarkerShape::Cross,
        MarkerShape::Circle,
        MarkerShape::TriangleUp,
        MarkerShape::TriangleDown,
        MarkerShape::Flag,
        MarkerShape::ArrowUp,
        MarkerShape::ArrowDown,
        MarkerShape::LabelUp,
        MarkerShape::LabelDown,
        MarkerShape::Square,
        MarkerShape::Diamond,
    ];
    if shapes.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    symbol_catalog::is_reserved_constant(name) || AlertFrequency::PINE_NAMES.contains(&name)
}

/// Structure validation (PINE3020–3024, PINE3048, PINE9003) is deferred until `PineSymbolCatalog`
/// lands; it composes the runtime's dispatch tables. For now it reports nothing.
fn structure_validate(
    statements: &[Statement],
    methods: &HashSet<String>,
    source: &str,
) -> Vec<Diagnostic> {
    structure_validator::validate(statements, methods, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_annotation_may_have_blanks() {
        for header in [
            "//@version=6",
            "// @version=6",
            "// @version = 6",
            "//  @version =6 ",
        ] {
            let source = format!("{header}\nindicator(\"T\")\nplot(close)");
            let program = compile(&source, Limits::default(), &NoLibraries);
            assert!(program.is_valid(), "{header}: {:?}", program.diagnostics);
        }
        let program = compile(
            "// @version = 5\nindicator(\"T\")",
            Limits::default(),
            &NoLibraries,
        );
        assert!(program.diagnostics.iter().any(|d| d.code == "PINE0002"));
    }

    #[test]
    fn compile_enforces_the_supplied_source_limit() {
        let program = compile(
            "//@version=6\nindicator(\"Too long\")\nplot(close)",
            Limits {
                source_characters: 8,
                ..Limits::default()
            },
            &NoLibraries,
        );

        assert!(
            program
                .diagnostics
                .iter()
                .any(|item| item.code == "PINE8001")
        );
    }

    #[test]
    fn compile_enforces_the_supplied_token_limit() {
        let program = compile(
            "//@version=6\nindicator(\"Tokens\")\na = close + open\nplot(a)",
            Limits {
                tokens: 4,
                ..Limits::default()
            },
            &NoLibraries,
        );

        assert!(
            program
                .diagnostics
                .iter()
                .any(|item| item.code == "PINE8002")
        );
    }
}
