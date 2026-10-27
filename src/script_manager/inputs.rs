//! A script's `input.*` declarations as editable cards, and the per-script cache of the values
//! tried in the preview. Ported from `PineInputsView`, `SettingsCardRow` and
//! `ScriptPreviewInputsStore`.

use crate::model::pine_instance::StoredInput as Stored;
use crate::pine::program::{InputControl, InputDefinition, InputSchema, InputValue};
use crate::pine::token::ValueType;
use crate::ui::icons::Icon;
use crate::ui::kit::StepDirection as Direction;
use crate::ui::settings_card;
use crate::ui::theme;
use eframe::egui::{self, Align, Align2, Color32, FontId, Id, Layout, Margin, Ui, vec2};
use std::collections::HashMap;
use std::path::PathBuf;

/// Override map by input id; an input with no entry shows its default.
pub type InputValues = HashMap<String, InputValue>;

/// What a card changed.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEdit {
    Set(String, InputValue),
    Reset,
}

// MARK: Persistence

/// The input values a user tried in the preview, per script: a small cache beside the scripts,
/// not part of them. Nothing here is applied to a chart.
pub struct InputsStore {
    path: Option<PathBuf>,
    entries: HashMap<String, HashMap<String, Stored>>,
}

impl InputsStore {
    pub fn open(path: Option<PathBuf>) -> Self {
        let entries = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    pub fn inputs(&self, script_id: &str) -> InputValues {
        self.entries
            .get(script_id)
            .map(|map| {
                map.iter()
                    .map(|(k, v)| (k.clone(), v.clone().into()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// An empty map forgets the script, so defaults are what the next session starts from.
    pub fn set_inputs(&mut self, script_id: &str, inputs: &InputValues) {
        let stored: HashMap<String, Stored> =
            inputs.iter().map(|(k, v)| (k.clone(), v.into())).collect();
        let before = self.entries.get(script_id);
        if stored.is_empty() {
            if before.is_none() {
                return;
            }
            self.entries.remove(script_id);
        } else {
            if before == Some(&stored) {
                return;
            }
            self.entries.insert(script_id.to_owned(), stored);
        }
        self.save();
    }

    /// Adds the values of scripts that have none yet, returning how many scripts it filled. A
    /// script the user already tried in this app keeps its own values.
    pub fn import_missing(&mut self, imported: HashMap<String, InputValues>) -> usize {
        let mut added = 0;
        for (script_id, values) in imported {
            if values.is_empty() || self.entries.contains_key(&script_id) {
                continue;
            }
            let stored = values.iter().map(|(k, v)| (k.clone(), v.into())).collect();
            self.entries.insert(script_id, stored);
            added += 1;
        }
        if added > 0 {
            self.save();
        }
        added
    }

    /// Drops the values of scripts that no longer exist.
    pub fn prune(&mut self, keeping: &[String]) {
        let before = self.entries.len();
        self.entries.retain(|id, _| keeping.contains(id));
        if self.entries.len() != before {
            self.save();
        }
    }

    fn save(&self) {
        if let Some(path) = &self.path
            && let Ok(bytes) = serde_json::to_vec_pretty(&self.entries)
        {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Keeps only the overrides whose input still exists with the same kind of value.
pub fn prune_values(values: &InputValues, schema: &InputSchema) -> InputValues {
    values
        .iter()
        .filter(|(id, value)| {
            schema.inputs.iter().any(|input| {
                input.id == **id
                    && std::mem::discriminant(&input.default_value) == std::mem::discriminant(value)
            })
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Applies a card's change: a value equal to the default is no override at all.
pub fn apply_edit(values: &mut InputValues, schema: &InputSchema, edit: InputEdit) {
    match edit {
        InputEdit::Reset => values.clear(),
        InputEdit::Set(id, value) => {
            let default = schema
                .inputs
                .iter()
                .find(|input| input.id == id)
                .map(|input| input.default_value.clone());
            if default.as_ref() == Some(&value) {
                values.remove(&id);
            } else {
                values.insert(id, value);
            }
        }
    }
}

// MARK: Layout

/// Inputs sectioned by their `group`, in the order each group first appears.
pub fn groups(schema: &InputSchema) -> Vec<(Option<String>, Vec<&InputDefinition>)> {
    let mut groups: Vec<(Option<String>, Vec<&InputDefinition>)> = Vec::new();
    for input in &schema.inputs {
        match groups.iter_mut().find(|g| g.0 == input.group) {
            Some(group) => group.1.push(input),
            None => groups.push((input.group.clone(), vec![input])),
        }
    }
    groups
}

/// An options input's choices: the stored value and the title the menu shows for it.
pub fn option_titles(input: &InputDefinition) -> Vec<(String, String)> {
    input
        .options
        .iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, option)| {
            let InputValue::String(text) = option else {
                return None;
            };
            let title = input
                .option_titles
                .as_ref()
                .and_then(|titles| titles.get(index))
                .cloned()
                .unwrap_or_else(|| text.clone());
            Some((text.clone(), title))
        })
        .collect()
}

/// Width cap for an options dropdown: a long title would crowd the card's name and hint.
pub fn menu_width(titles: &[String]) -> Option<f32> {
    (titles.iter().map(|t| t.chars().count()).max().unwrap_or(0) > 20).then_some(95.0)
}

pub fn color_from_rgba(rgba: u32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (rgba >> 24) as u8,
        (rgba >> 16) as u8,
        (rgba >> 8) as u8,
        rgba as u8,
    )
}

pub fn rgba_from_color(color: Color32) -> u32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    u32::from(r) << 24 | u32::from(g) << 16 | u32::from(b) << 8 | u32::from(a)
}

/// The Swift app's `script_preview_inputs.json`: script UUID → input id → a synthesized-Codable
/// `PineInputValue` (`{"bool": {"_0": true}}`). Entries it cannot read are skipped.
pub fn parse_swift_inputs(bytes: &[u8]) -> HashMap<String, InputValues> {
    let Ok(serde_json::Value::Object(scripts)) = serde_json::from_slice(bytes) else {
        return HashMap::new();
    };
    scripts
        .into_iter()
        .filter_map(|(script, inputs)| {
            let values: InputValues = inputs
                .as_object()?
                .iter()
                .filter_map(|(id, value)| Some((id.clone(), swift_value(value)?)))
                .collect();
            Some((script, values))
        })
        .collect()
}

fn swift_value(value: &serde_json::Value) -> Option<InputValue> {
    let (case, payload) = value.as_object()?.iter().next()?;
    let payload = payload.get("_0")?;
    Some(match case.as_str() {
        "int" => InputValue::Int(payload.as_i64()?),
        "float" => InputValue::Float(payload.as_f64()?),
        "bool" => InputValue::Bool(payload.as_bool()?),
        "string" => InputValue::String(payload.as_str()?.to_owned()),
        "color" => InputValue::Color(u32::try_from(payload.as_u64()?).ok()?),
        "source" => InputValue::Source(payload.as_str()?.to_owned()),
        _ => return None,
    })
}

// MARK: View

/// The Inputs tab: cards sectioned by group, then "Reset to Defaults".
pub fn show(ui: &mut Ui, schema: &InputSchema, values: &InputValues) -> Vec<InputEdit> {
    let mut edits = Vec::new();
    if schema.inputs.is_empty() {
        super::drawer::placeholder(
            ui,
            "No inputs",
            "Declare one with input.int(), input.float(), input.bool() and friends, and it appears here.",
        );
        return edits;
    }
    egui::ScrollArea::vertical()
        .id_salt("script-inputs-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    edits = cards(ui, schema, values);
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!values.is_empty(), egui::Button::new("Reset to Defaults"))
                            .clicked()
                        {
                            edits.push(InputEdit::Reset);
                        }
                    });
                });
        });
    edits
}

/// `PineInputsView`: one card per input, sectioned by group. Stateless and unscrolled, so a
/// popover and the Script Manager can each frame it their own way.
pub fn cards(ui: &mut Ui, schema: &InputSchema, values: &InputValues) -> Vec<InputEdit> {
    let mut edits = Vec::new();
    ui.spacing_mut().item_spacing.y = 12.0;
    for (title, inputs) in groups(schema) {
        if let Some(title) = title {
            ui.add_space(6.0);
            ui.label(crate::ui::kit::text(
                title,
                11.0,
                crate::ui::kit::SEMIBOLD,
                theme::text_secondary(),
            ));
        }
        for input in inputs {
            let active = schema.is_active(input, values);
            if let Some(value) = card(ui, input, values.get(&input.id), active) {
                edits.push(InputEdit::Set(input.id.clone(), value));
            }
        }
    }
    edits
}

/// One script input as a card; returns the new value when the control changed.
fn card(
    ui: &mut Ui,
    input: &InputDefinition,
    override_value: Option<&InputValue>,
    active: bool,
) -> Option<InputValue> {
    let title = input.title.clone().unwrap_or_else(|| input.id.clone());
    let current = override_value.unwrap_or(&input.default_value).clone();
    let icon = match (&input.kind, &current) {
        (_, InputValue::Bool(_)) => Icon::Switch,
        (ValueType::Time, InputValue::Int(_)) => Icon::Calendar,
        (_, InputValue::Int(_) | InputValue::Float(_)) => Icon::Hash,
        (_, InputValue::String(_)) if input.options.is_some() => Icon::List,
        (_, InputValue::String(_)) if input.control == InputControl::Timeframe => Icon::List,
        (_, InputValue::String(_)) => Icon::TextFormat,
        (_, InputValue::Color(_)) => Icon::Palette,
        (_, InputValue::Source(_)) => Icon::ChartLine,
    };
    // A greyed-out card still shows the value the script reads; it just cannot be changed.
    ui.add_enabled_ui(active, |ui| {
        settings_card::card(ui, icon, &title, input.tooltip.as_deref(), |ui| {
            control(ui, input, &title, &current)
        })
    })
    .inner
}

fn control(
    ui: &mut Ui,
    input: &InputDefinition,
    title: &str,
    current: &InputValue,
) -> Option<InputValue> {
    match (&input.kind, current) {
        (_, InputValue::Bool(value)) => {
            let mut on = *value;
            settings_card::switch(ui, &mut on).then_some(InputValue::Bool(on))
        }
        (ValueType::Time, InputValue::Int(ms)) => time_field(ui, input, *ms),
        (_, InputValue::Int(value)) => int_field(ui, input, title, *value),
        (_, InputValue::Float(value)) => float_field(ui, input, title, *value),
        (_, InputValue::String(value)) if input.options.is_some() => {
            let titles = option_titles(input);
            let labels: Vec<String> = titles.iter().map(|(_, title)| title.clone()).collect();
            let selected = titles.iter().position(|(stored, _)| stored == value);
            // A value the options no longer list shows as itself, until another is picked.
            let shown = labels_with_current(&labels, selected, value);
            let width = menu_width(&labels).unwrap_or_else(|| picker_width(ui, &shown));
            crate::ui::kit::menu_picker(ui, width, &shown, selected.unwrap_or(labels.len()), true)
                .and_then(|index| titles.get(index))
                .map(|(stored, _)| InputValue::String(stored.clone()))
        }
        (_, InputValue::String(value)) if input.control == InputControl::Timeframe => {
            let labels: Vec<String> = TIMEFRAMES
                .iter()
                .map(|(_, label)| (*label).to_owned())
                .collect();
            let selected = TIMEFRAMES.iter().position(|(stored, _)| stored == value);
            let shown = labels_with_current(&labels, selected, value);
            let width = picker_width(ui, &shown);
            crate::ui::kit::menu_picker(ui, width, &shown, selected.unwrap_or(labels.len()), true)
                .and_then(|index| TIMEFRAMES.get(index))
                .map(|(stored, _)| InputValue::String((*stored).to_owned()))
        }
        (_, InputValue::String(value)) if input.control == InputControl::TextArea => {
            let mut text = value.clone();
            let response = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .hint_text(title)
                    .desired_rows(3)
                    .desired_width(220.0)
                    .margin(Margin::symmetric(6, 4)),
            );
            response.changed().then_some(InputValue::String(text))
        }
        (_, InputValue::String(value)) => {
            let mut text = value.clone();
            // A session that is not one reads in red, as the script would find it unusable.
            let invalid = input.control == InputControl::Session && !session_is_valid(value);
            let hint = match input.control {
                InputControl::Symbol => "EXCHANGE:SYMBOL",
                InputControl::Session => "0930-1600",
                _ => title,
            };
            let mut edit = egui::TextEdit::singleline(&mut text)
                .hint_text(hint)
                .desired_width(140.0)
                .margin(Margin::symmetric(6, 4));
            if invalid {
                edit = edit.text_color(theme::LOSS);
            }
            let response = ui.add(edit);
            if invalid {
                response.clone().on_hover_text(
                    "Not a session. Write it as 0930-1600, with days after a colon (0930-1600:23456) or 24x7.",
                );
            }
            response.changed().then_some(InputValue::String(text))
        }
        (_, InputValue::Color(value)) => crate::ui::kit::color_well(
            ui,
            vec2(44.0, 24.0),
            title,
            color_from_rgba(*value),
            egui::color_picker::Alpha::OnlyBlend,
        )
        .map(|color| InputValue::Color(rgba_from_color(color))),
        (_, InputValue::Source(value)) => {
            let names = SOURCES;
            let labels: Vec<String> = names.iter().map(|name| capitalized(name)).collect();
            let width = picker_width(ui, &labels);
            let selected = names.iter().position(|name| name == value).unwrap_or(3);
            crate::ui::kit::menu_picker(ui, width, &labels, selected, true)
                .map(|index| InputValue::Source(names[index].to_owned()))
        }
    }
}

/// The sources `input.source` offers: the bar's prices and their averages, and the volume.
const SOURCES: [&str; 9] = [
    "open", "high", "low", "close", "hl2", "hlc3", "ohlc4", "hlcc4", "volume",
];

/// The timeframes `input.timeframe` offers when the script lists none, as Pine writes them and as
/// they read; the empty string is the chart's own.
const TIMEFRAMES: [(&str, &str); 15] = [
    ("", "Chart"),
    ("1", "1 minute"),
    ("3", "3 minutes"),
    ("5", "5 minutes"),
    ("15", "15 minutes"),
    ("30", "30 minutes"),
    ("45", "45 minutes"),
    ("60", "1 hour"),
    ("120", "2 hours"),
    ("180", "3 hours"),
    ("240", "4 hours"),
    ("1D", "1 day"),
    ("1W", "1 week"),
    ("1M", "1 month"),
    ("3M", "3 months"),
];

/// Whether a session input holds a session Pine can read (an empty one is the chart's own).
fn session_is_valid(text: &str) -> bool {
    text.is_empty() || crate::pine::trading_session::TradingSession::parse(text).is_some()
}

/// `labels`, plus `current` when it matches none of them, so the picker has something to show.
fn labels_with_current(labels: &[String], selected: Option<usize>, current: &str) -> Vec<String> {
    let mut shown = labels.to_vec();
    if selected.is_none() {
        shown.push(current.to_owned());
    }
    shown
}

/// A `.menu` picker's natural width: its widest choice, the chevrons and the padding.
fn picker_width(ui: &Ui, labels: &[String]) -> f32 {
    let widest = labels
        .iter()
        .map(|label| {
            crate::ui::kit::galley(
                ui,
                label,
                crate::ui::kit::size::BODY,
                crate::ui::kit::REGULAR,
                theme::text_primary(),
                f32::INFINITY,
            )
            .size()
            .x
        })
        .fold(0.0, f32::max);
    (widest + 40.0).max(72.0)
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// How far one arrow press moves a number: a step, ten steps with ⇧, a tenth of a step with ⌥.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stride {
    Fine,
    Normal,
    Coarse,
}

impl Stride {
    fn from_modifiers(modifiers: egui::Modifiers) -> Self {
        if modifiers.shift {
            Self::Coarse
        } else if modifiers.alt {
            Self::Fine
        } else {
            Self::Normal
        }
    }
}

/// What a numeric input (integer or float) knows about its own values; [`number_field`] drives
/// the editing around it.
trait NumberKind {
    type Value: Copy + PartialEq;

    fn format(&self, value: Self::Value) -> String;
    /// The number a buffer holds, or `None` when it is not one.
    fn parse(&self, text: &str) -> Option<Self::Value>;
    fn contains(&self, value: Self::Value) -> bool;
    fn clamp(&self, value: Self::Value) -> Self::Value;
    /// `value` moved one `stride` in `direction`, within the limits.
    fn nudge(&self, value: Self::Value, direction: Direction, stride: Stride) -> Self::Value;
    /// The tooltip for a field with declared limits.
    fn hint(&self) -> Option<String>;
}

/// What an integer input accepts: its declared `minval`/`maxval` (open when absent) and the step
/// of its arrows.
struct IntBounds {
    min: Option<i64>,
    max: Option<i64>,
    step: i64,
}

impl IntBounds {
    fn of(input: &InputDefinition) -> Self {
        Self {
            min: input.min_value.map(|v| v as i64),
            max: input.max_value.map(|v| v as i64),
            step: input.step.map_or(1, |s| (s as i64).max(1)),
        }
    }
}

impl NumberKind for IntBounds {
    type Value = i64;

    fn format(&self, value: i64) -> String {
        value.to_string()
    }

    fn parse(&self, text: &str) -> Option<i64> {
        parse_int(text)
    }

    fn contains(&self, value: i64) -> bool {
        self.min.is_none_or(|min| value >= min) && self.max.is_none_or(|max| value <= max)
    }

    fn clamp(&self, value: i64) -> i64 {
        value.clamp(self.min.unwrap_or(i64::MIN), self.max.unwrap_or(i64::MAX))
    }

    /// A whole number has no finer step than one, so `Fine` moves like `Normal`.
    fn nudge(&self, value: i64, direction: Direction, stride: Stride) -> i64 {
        let step = match stride {
            Stride::Coarse => self.step.saturating_mul(10),
            Stride::Normal | Stride::Fine => self.step,
        };
        let moved = match direction {
            Direction::Up => value.saturating_add(step),
            Direction::Down => value.saturating_sub(step),
        };
        self.clamp(moved)
    }

    fn hint(&self) -> Option<String> {
        limits_hint(self.min, self.max)
    }
}

/// What a float input accepts: its declared `minval`/`maxval` (open when absent) and the step of
/// its arrows — the declared `step`, else one unit of the default's last decimal place (1 for a
/// whole default, as Pine does).
struct FloatBounds {
    min: Option<f64>,
    max: Option<f64>,
    step: f64,
}

/// Decimals beyond this are float noise, not something a person types.
const MAX_PLACES: usize = 10;

impl FloatBounds {
    fn of(input: &InputDefinition) -> Self {
        let step = input
            .step
            .filter(|step| step.is_finite() && *step > 0.0)
            .unwrap_or_else(|| match &input.default_value {
                InputValue::Float(default) => 10f64.powi(-(places_of(*default) as i32)),
                _ => 1.0,
            });
        Self {
            min: input.min_value,
            max: input.max_value,
            step,
        }
    }
}

impl NumberKind for FloatBounds {
    type Value = f64;

    /// Always with a decimal part (`2.0`, not `2`), so a float reads as one beside an integer, and
    /// with at least as many places as the step needs so stepping keeps one format (`2.00 → 2.05`).
    /// Digits the value really has are never cut.
    fn format(&self, value: f64) -> String {
        let have = trim_float(value)
            .split_once('.')
            .map_or(0, |(_, fraction)| fraction.len());
        let places = have.max(places_of(self.step)).max(1);
        format!("{value:.places$}")
    }

    fn parse(&self, text: &str) -> Option<f64> {
        parse_float(text)
    }

    fn contains(&self, value: f64) -> bool {
        self.min.is_none_or(|min| value >= min) && self.max.is_none_or(|max| value <= max)
    }

    fn clamp(&self, value: f64) -> f64 {
        let value = self.min.map_or(value, |min| value.max(min));
        self.max.map_or(value, |max| value.min(max))
    }

    /// Moves by the step relative to `value` (a typed 2.37 stays off the step's grid), rounded to
    /// the finer of the two decimal resolutions so `0.1 + 0.2` reads `0.3`.
    fn nudge(&self, value: f64, direction: Direction, stride: Stride) -> f64 {
        let (factor, extra_places) = match stride {
            Stride::Fine => (0.1, 1),
            Stride::Normal => (1.0, 0),
            Stride::Coarse => (10.0, 0),
        };
        let step_places = (places_of(self.step) + extra_places).min(MAX_PLACES);
        let amount = round_to(self.step * factor, step_places);
        let moved = match direction {
            Direction::Up => value + amount,
            Direction::Down => value - amount,
        };
        self.clamp(round_to(moved, step_places.max(places_of(value))))
    }

    fn hint(&self) -> Option<String> {
        limits_hint(self.min.map(trim_float), self.max.map(trim_float))
    }
}

/// "2 to 200", "2 or more", "200 or less": a limits tooltip for whatever formats as text.
fn limits_hint<T: std::fmt::Display>(min: Option<T>, max: Option<T>) -> Option<String> {
    match (min, max) {
        (Some(min), Some(max)) => Some(format!("{min} to {max}")),
        (Some(min), None) => Some(format!("{min} or more")),
        (None, Some(max)) => Some(format!("{max} or less")),
        (None, None) => None,
    }
}

/// A whole number typed into an integer field: optional sign, digits, and `20.0` style text for
/// a whole value. Anything else (empty, `1.5`, `abc`) is not a number.
fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim();
    text.parse::<i64>().ok().or_else(|| {
        let parsed = text.parse::<f64>().ok()?;
        (parsed.is_finite() && parsed.fract() == 0.0 && parsed.abs() < 9.0e15)
            .then_some(parsed as i64)
    })
}

/// A decimal number typed into a float field; a comma works as the decimal point. Not a number
/// when empty, malformed, or infinite.
fn parse_float(text: &str) -> Option<f64> {
    text.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite())
}

/// Decimal places in `value`'s shortest text (`0.25` → 2, `3.0` → 0), capped at [`MAX_PLACES`].
fn places_of(value: f64) -> usize {
    format!("{value}")
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len())
        .min(MAX_PLACES)
}

fn round_to(value: f64, places: usize) -> f64 {
    let scale = 10f64.powi(places as i32);
    (value * scale).round() / scale
}

/// The arrow key (↑/↓) pressed this frame and how far it moves: one step, ten with ⇧, a tenth
/// with ⌥. The key is consumed so the text field does not also act on it.
fn arrow_key(ui: &Ui) -> Option<(Direction, Stride)> {
    use egui::{Key, Modifiers};
    ui.input_mut(|input| {
        // Most specific first: a plain `NONE` pattern also matches while ⇧ or ⌥ is held.
        for (modifiers, stride) in [
            (Modifiers::SHIFT, Stride::Coarse),
            (Modifiers::ALT, Stride::Fine),
            (Modifiers::NONE, Stride::Normal),
        ] {
            if input.consume_key(modifiers, Key::ArrowUp) {
                return Some((Direction::Up, stride));
            }
            if input.consume_key(modifiers, Key::ArrowDown) {
                return Some((Direction::Down, stride));
            }
        }
        None
    })
}

/// The numeric text field shared by the integer and float inputs: right-aligned digits in a
/// rounded well as tall as the stepper, ringed in the accent while focused and in red when
/// `invalid`.
fn number_well(ui: &mut Ui, edit_id: Id, text: &mut String, invalid: bool) -> egui::Response {
    let well = ui.painter().add(egui::Shape::Noop);
    let (rect, area) = ui.allocate_exact_size(
        vec2(NUMBER_WELL_WIDTH, NUMBER_WELL_HEIGHT),
        egui::Sense::click(),
    );
    // The text is inset by hand: egui does not apply a margin to right-aligned text.
    let inner = rect.shrink2(vec2(NUMBER_WELL_INSET, 0.0));
    let response = ui.put(
        inner,
        egui::TextEdit::singleline(text)
            .id(edit_id)
            .frame(egui::Frame::NONE)
            .font(FontId::proportional(crate::ui::kit::size::BODY))
            .text_color(theme::text_primary())
            .horizontal_align(Align::Max)
            .vertical_align(Align::Center)
            .desired_width(inner.width())
            .min_size(inner.size())
            .margin(Margin::symmetric(0, 3)),
    );
    // The padding is part of the field: a click there focuses it too.
    if area.clicked() && !response.has_focus() {
        response.request_focus();
    }
    area.on_hover_cursor(egui::CursorIcon::Text);
    let hovered = response.hovered() || ui.rect_contains_pointer(rect);
    let ring = if invalid {
        theme::with_alpha(crate::ui::kit::RED, 0.7)
    } else if response.has_focus() {
        theme::with_alpha(crate::ui::kit::ACCENT, 0.7)
    } else {
        theme::ink(if hovered { 40 } else { 30 })
    };
    ui.painter().set(
        well,
        egui::epaint::RectShape::new(
            rect,
            6.0,
            theme::quaternary(0.45),
            egui::Stroke::new(1.0, ring),
            egui::StrokeKind::Inside,
        ),
    );
    response
}

/// The stepper's height, so the field and its arrows line up; wide enough for six digits.
const NUMBER_WELL_HEIGHT: f32 = 22.0;
const NUMBER_WELL_WIDTH: f32 = 68.0;
const NUMBER_WELL_INSET: f32 = 9.0;

/// A numeric input: a typed field with the macOS stepper beside it.
///
/// The field edits a buffer (kept only while it has focus) and commits on Return or when it
/// loses focus, clamped to the input's limits; Escape drops the edit, and ↑/↓ (⇧ for ten steps,
/// ⌥ for a tenth) move the value as the stepper does. A buffer that is not a number, or lies
/// outside the limits, rings the field red until it is fixed.
fn number_field<K: NumberKind>(
    ui: &mut Ui,
    kind: &K,
    id: Id,
    title: &str,
    value: K::Value,
) -> Option<K::Value> {
    let edit_id = id.with("edit");
    ui.spacing_mut().item_spacing.x = 6.0;

    // Right-to-left layout: the stepper first (it is rightmost), then the field.
    let mut result = crate::ui::kit::stepper_arrows(ui, id.with("stepper"))
        .map(|direction| {
            let stride = ui.input(|input| Stride::from_modifiers(input.modifiers));
            kind.nudge(value, direction, stride)
        })
        .filter(|next| *next != value);

    let editing = ui.memory(|memory| memory.has_focus(edit_id));
    let mut text = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| kind.format(value));
    let mut cancelled = false;
    if editing {
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            cancelled = true;
            ui.memory_mut(|memory| memory.surrender_focus(edit_id));
        } else if let Some((direction, stride)) = arrow_key(ui) {
            let next = kind.nudge(kind.parse(&text).unwrap_or(value), direction, stride);
            text = kind.format(next);
            if next != value {
                result = Some(next);
            }
        }
    }
    let invalid = editing
        && kind
            .parse(&text)
            .is_none_or(|parsed| !kind.contains(parsed));

    let response = number_well(ui, edit_id, &mut text, invalid);
    let response = crate::ui::accessibility::label(response, title);
    if let Some(hint) = kind.hint() {
        response.clone().on_hover_text(hint);
    }

    if response.gained_focus() && ui.input(|input| input.key_pressed(egui::Key::Tab)) {
        // Tabbing in selects the whole number, as a native field does.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
            let end = egui::text::CCursor::new(text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    end,
                )));
            state.store(ui.ctx(), edit_id);
        }
    }
    if response.lost_focus()
        && !cancelled
        && result.is_none()
        && let Some(parsed) = kind.parse(&text)
    {
        let next = kind.clamp(parsed);
        if next != value {
            result = Some(next);
        }
    }
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, text));
    } else {
        ui.data_mut(|data| data.remove::<String>(id));
    }
    result
}

fn int_field(ui: &mut Ui, input: &InputDefinition, title: &str, value: i64) -> Option<InputValue> {
    let id = Id::new(("input-int", &input.id));
    number_field(ui, &IntBounds::of(input), id, title, value).map(InputValue::Int)
}

fn float_field(
    ui: &mut Ui,
    input: &InputDefinition,
    title: &str,
    value: f64,
) -> Option<InputValue> {
    let id = Id::new(("input-float", &input.id));
    number_field(ui, &FloatBounds::of(input), id, title, value).map(InputValue::Float)
}

fn trim_float(value: f64) -> String {
    let text = format!("{value}");
    text
}

// MARK: Time input

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(super) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2024-03-09 14:30` in the system time zone, like the date picker it stands in for.
pub fn format_time(ms: i64) -> String {
    let secs = (ms + crate::service::timezone::local_utc_offset_secs(ms) * 1000).div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

pub fn parse_time(text: &str) -> Option<i64> {
    let (date, time) = text
        .trim()
        .split_once(' ')
        .unwrap_or((text.trim(), "00:00"));
    let mut d = date.split('-');
    let (y, m, day) = (
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<i64>().ok()?,
    );
    let mut t = time.split(':');
    let (h, min) = (
        t.next()?.parse::<i64>().ok()?,
        t.next()?.parse::<i64>().ok()?,
    );
    if !(1..=12).contains(&m)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&h)
        || !(0..60).contains(&min)
    {
        return None;
    }
    let local_ms = (days_from_civil(y, m, day) * 86_400 + h * 3600 + min * 60) * 1000;
    Some(local_ms - crate::service::timezone::local_utc_offset_secs(local_ms) * 1000)
}

fn time_field(ui: &mut Ui, input: &InputDefinition, ms: i64) -> Option<InputValue> {
    let id = Id::new(("input-time", &input.id));
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| format_time(ms));
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id.with("edit"))
            .desired_width(130.0)
            .hint_text("YYYY-MM-DD HH:MM")
            .margin(Margin::symmetric(6, 4)),
    );
    let mut result = None;
    if response.changed()
        && let Some(parsed) = parse_time(&text)
    {
        result = Some(InputValue::Int(parsed));
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    result
}

#[allow(dead_code)]
fn _keep(_: Align2, _: FontId) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, kind: ValueType, default: InputValue, group: Option<&str>) -> InputDefinition {
        InputDefinition {
            id: id.into(),
            kind,
            control: InputControl::Standard,
            default_value: default,
            title: None,
            tooltip: None,
            group: group.map(str::to_owned),
            inline: None,
            confirm: false,
            min_value: None,
            max_value: None,
            step: None,
            options: None,
            option_titles: None,
            active: None,
        }
    }

    #[test]
    fn groups_keep_first_appearance_order() {
        let schema = InputSchema {
            inputs: vec![
                def("a", ValueType::Int, InputValue::Int(1), Some("B")),
                def("b", ValueType::Int, InputValue::Int(1), None),
                def("c", ValueType::Int, InputValue::Int(1), Some("B")),
                def("d", ValueType::Int, InputValue::Int(1), Some("A")),
            ],
        };
        let titles: Vec<Option<String>> = groups(&schema).into_iter().map(|g| g.0).collect();
        assert_eq!(titles, [Some("B".into()), None, Some("A".into())]);
        assert_eq!(groups(&schema)[0].1.len(), 2);
    }

    #[test]
    fn option_titles_fall_back_to_the_value_and_long_titles_cap_the_menu() {
        let mut input = def("m", ValueType::String, InputValue::String("a".into()), None);
        input.options = Some(vec![
            InputValue::String("a".into()),
            InputValue::String("b".into()),
        ]);
        input.option_titles = Some(vec!["Alpha".into()]);
        assert_eq!(
            option_titles(&input),
            [("a".into(), "Alpha".into()), ("b".into(), "b".into())]
        );
        assert_eq!(menu_width(&["short".into()]), None);
        assert_eq!(menu_width(&["x".repeat(21)]), Some(95.0));
    }

    #[test]
    fn the_cache_round_trips_and_forgets_empty_maps() {
        let dir = std::env::temp_dir().join(format!("degenview-inputs-{}", std::process::id()));
        let path = dir.join("script_preview_inputs.json");
        let mut store = InputsStore::open(Some(path.clone()));
        let mut values = InputValues::new();
        values.insert("len".into(), InputValue::Int(30));
        values.insert("c".into(), InputValue::Color(0xff00_00ff));
        store.set_inputs("s1", &values);
        assert_eq!(InputsStore::open(Some(path.clone())).inputs("s1"), values);
        store.set_inputs("s1", &InputValues::new());
        assert!(InputsStore::open(Some(path)).inputs("s1").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn swift_inputs_are_read_and_never_replace_existing_values() {
        let json = br#"{"A":{"colorCandles":{"bool":{"_0":true}},"n":{"int":{"_0":3}},
            "x":{"float":{"_0":1.5}},"p":{"string":{"_0":"Top left"}},"c":{"color":{"_0":255}},
            "s":{"source":{"_0":"close"}},"odd":{"matrix":{"_0":1}}},"B":{"k":{"int":{"_0":1}}}}"#;
        let parsed = parse_swift_inputs(json);
        let a = &parsed["A"];
        assert_eq!(a.len(), 6, "the unknown case is skipped");
        assert_eq!(a["colorCandles"], InputValue::Bool(true));
        assert_eq!(a["x"], InputValue::Float(1.5));
        assert_eq!(a["c"], InputValue::Color(255));
        assert!(parse_swift_inputs(b"not json").is_empty());

        let mut store = InputsStore::open(None);
        let mut own = InputValues::new();
        own.insert("k".into(), InputValue::Int(9));
        store.set_inputs("B", &own);
        let added = store.import_missing(parsed);
        assert_eq!(added, 1);
        assert_eq!(store.inputs("B"), own, "an existing entry stays");
        assert_eq!(store.inputs("A").len(), 6);
    }

    #[test]
    fn values_for_removed_or_retyped_inputs_are_pruned() {
        let schema = InputSchema {
            inputs: vec![def("len", ValueType::Int, InputValue::Int(1), None)],
        };
        let mut values = InputValues::new();
        values.insert("len".into(), InputValue::Int(9));
        values.insert("gone".into(), InputValue::Int(1));
        let kept = prune_values(&values, &schema);
        assert_eq!(kept.len(), 1);
        values.insert("len".into(), InputValue::Float(1.0));
        assert!(prune_values(&values, &schema).is_empty(), "type changed");
    }

    #[test]
    fn typed_integers_parse_whole_numbers_only() {
        assert_eq!(parse_int("14"), Some(14));
        assert_eq!(parse_int("  -3 "), Some(-3));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int("20.0"), Some(20));
        assert_eq!(parse_int("1.5"), None);
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("-"), None);
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int("NaN"), None);
        assert_eq!(parse_int("1e30"), None);
    }

    #[test]
    fn integer_bounds_clamp_to_the_declared_limits_only() {
        let mut input = def("len", ValueType::Int, InputValue::Int(14), None);
        let open = IntBounds::of(&input);
        assert_eq!(
            open.clamp(1_000_000),
            1_000_000,
            "undeclared limits are open"
        );
        assert_eq!(open.clamp(-40), -40);
        assert_eq!(open.hint(), None);
        assert_eq!(open.step, 1);

        input.min_value = Some(2.0);
        input.max_value = Some(200.0);
        input.step = Some(5.0);
        let bounded = IntBounds::of(&input);
        assert_eq!(bounded.clamp(0), 2);
        assert_eq!(bounded.clamp(999), 200);
        assert!(bounded.contains(2) && bounded.contains(200) && !bounded.contains(201));
        assert_eq!(bounded.hint().as_deref(), Some("2 to 200"));
        assert_eq!(bounded.step, 5);
    }

    fn float_input(default: f64, min: Option<f64>, step: Option<f64>) -> InputDefinition {
        let mut input = def("mult", ValueType::Float, InputValue::Float(default), None);
        input.min_value = min;
        input.step = step;
        input
    }

    #[test]
    fn typed_floats_take_a_comma_and_reject_non_numbers() {
        assert_eq!(parse_float("2.75"), Some(2.75));
        assert_eq!(parse_float(" 2,75 "), Some(2.75));
        assert_eq!(parse_float("-0.5"), Some(-0.5));
        assert_eq!(parse_float("3"), Some(3.0));
        for bad in ["", "-", "abc", "NaN", "inf", "-infinity", "1.2.3"] {
            assert_eq!(parse_float(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_float_steps_by_its_declared_step_else_its_default_s_last_decimal() {
        let step = |default, declared| FloatBounds::of(&float_input(default, None, declared)).step;
        assert_eq!(step(2.0, Some(0.5)), 0.5, "declared wins");
        assert_eq!(step(0.5, None), 0.1);
        assert_eq!(step(0.05, None), 0.01);
        assert_eq!(step(2.0, None), 1.0, "a whole default steps by Pine's 1");
        assert_eq!(step(2.0, Some(0.0)), 1.0, "a non-positive step is ignored");
        assert_eq!(step(2.0, Some(f64::NAN)), 1.0);
    }

    #[test]
    fn float_arrows_do_not_drift_and_move_relative_to_the_value() {
        let bounds = FloatBounds::of(&float_input(0.1, None, None));
        let up = |value| bounds.nudge(value, Direction::Up, Stride::Normal);
        assert_eq!(up(0.2), 0.3, "0.1 + 0.2 style drift is rounded away");
        assert_eq!(up(0.7), 0.8);
        assert_eq!(bounds.nudge(0.3, Direction::Down, Stride::Normal), 0.2);
        assert_eq!(bounds.nudge(0.3, Direction::Up, Stride::Coarse), 1.3);
        assert_eq!(bounds.nudge(0.3, Direction::Up, Stride::Fine), 0.31);

        // A typed value off the step's grid is moved by the step, not snapped to the grid.
        let half = FloatBounds::of(&float_input(2.0, None, Some(0.5)));
        assert_eq!(half.nudge(2.37, Direction::Up, Stride::Normal), 2.87);
        // The finer of the two resolutions is kept.
        assert_eq!(half.nudge(2.375, Direction::Up, Stride::Normal), 2.875);
    }

    #[test]
    fn a_float_always_shows_a_decimal_part_and_the_steps_places() {
        let shown =
            |value, default, step| FloatBounds::of(&float_input(default, None, step)).format(value);
        assert_eq!(shown(2.0, 2.0, Some(0.1)), "2.0");
        assert_eq!(shown(2.0, 2.0, None), "2.0", "whole default, step 1");
        assert_eq!(shown(2.5, 2.0, Some(0.5)), "2.5");
        assert_eq!(shown(2.0, 2.0, Some(0.05)), "2.00");
        assert_eq!(shown(2.1, 2.0, Some(0.05)), "2.10");
        assert_eq!(
            shown(0.125, 2.0, Some(0.1)),
            "0.125",
            "digits are never cut"
        );
        assert_eq!(shown(-3.0, 2.0, Some(0.1)), "-3.0");
        assert_eq!(shown(0.0, 0.5, None), "0.0");
        assert_eq!(shown(1.0e-12, 2.0, None), "0.000000000001");
        // What is shown reads back as the same number.
        let bounds = FloatBounds::of(&float_input(2.0, None, Some(0.05)));
        for value in [2.0, 2.05, -0.5, 0.125, 1234.5] {
            assert_eq!(bounds.parse(&bounds.format(value)), Some(value));
        }
    }

    #[test]
    fn float_limits_clamp_and_the_arrows_stop_at_them() {
        let mut input = float_input(2.0, Some(0.5), Some(0.5));
        input.max_value = Some(4.0);
        let bounds = FloatBounds::of(&input);
        assert_eq!(bounds.nudge(0.5, Direction::Down, Stride::Normal), 0.5);
        assert_eq!(bounds.nudge(4.0, Direction::Up, Stride::Normal), 4.0);
        assert_eq!(bounds.nudge(3.0, Direction::Up, Stride::Coarse), 4.0);
        assert!(bounds.contains(0.5) && bounds.contains(4.0) && !bounds.contains(4.1));
        assert_eq!(bounds.clamp(0.1), 0.5);
        assert_eq!(bounds.hint().as_deref(), Some("0.5 to 4"));
        assert_eq!(FloatBounds::of(&float_input(2.0, None, None)).hint(), None);
    }

    #[test]
    fn integer_arrows_scale_with_the_stride() {
        let mut input = def("len", ValueType::Int, InputValue::Int(14), None);
        input.min_value = Some(1.0);
        input.step = Some(2.0);
        let bounds = IntBounds::of(&input);
        assert_eq!(bounds.nudge(14, Direction::Up, Stride::Normal), 16);
        assert_eq!(bounds.nudge(14, Direction::Up, Stride::Coarse), 34);
        assert_eq!(
            bounds.nudge(14, Direction::Up, Stride::Fine),
            16,
            "no finer than a step"
        );
        assert_eq!(
            bounds.nudge(3, Direction::Down, Stride::Coarse),
            1,
            "clamped to minval"
        );
        assert_eq!(
            bounds.nudge(i64::MAX, Direction::Up, Stride::Coarse),
            i64::MAX
        );
    }

    #[test]
    fn time_text_round_trips() {
        let ms = parse_time("2024-03-09 14:30").unwrap();
        assert_eq!(format_time(ms), "2024-03-09 14:30");
        assert_eq!(parse_time("1970-01-01 00:00"), Some(0));
        assert_eq!(parse_time("2024-13-01 00:00"), None);
        assert_eq!(parse_time("nonsense"), None);
    }

    #[test]
    fn rgba_converts_both_ways() {
        // Opaque colours round-trip exactly; translucent ones lose at most a unit to premultiplying.
        assert_eq!(
            rgba_from_color(color_from_rgba(0x11_22_33_ff)),
            0x11_22_33_ff
        );
        let c = color_from_rgba(0x11_22_33_80);
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        assert_eq!(a, 0x80);
        assert!(r.abs_diff(0x11) <= 1 && g.abs_diff(0x22) <= 1 && b.abs_diff(0x33) <= 1);
    }

    #[test]
    fn a_value_equal_to_the_default_is_no_override() {
        let schema = InputSchema {
            inputs: vec![def("len", ValueType::Int, InputValue::Int(14), None)],
        };
        let mut values = InputValues::new();
        apply_edit(
            &mut values,
            &schema,
            InputEdit::Set("len".into(), InputValue::Int(30)),
        );
        assert_eq!(values.get("len"), Some(&InputValue::Int(30)));
        apply_edit(
            &mut values,
            &schema,
            InputEdit::Set("len".into(), InputValue::Int(14)),
        );
        assert!(values.is_empty(), "back at the default");
        apply_edit(
            &mut values,
            &schema,
            InputEdit::Set("len".into(), InputValue::Int(5)),
        );
        apply_edit(&mut values, &schema, InputEdit::Reset);
        assert!(values.is_empty());
    }

    #[test]
    fn a_session_input_is_valid_when_pine_can_read_it() {
        for text in ["", "0930-1600", "0930-1600:23456", "24x7", "1800-0600"] {
            assert!(session_is_valid(text), "{text}");
        }
        for text in ["nine to five", "0930", "2500-1600", "0930-1600:9"] {
            assert!(!session_is_valid(text), "{text}");
        }
    }

    #[test]
    fn the_standard_timeframes_are_pine_strings_and_none_repeats() {
        let mut stored: Vec<&str> = TIMEFRAMES.iter().map(|(stored, _)| *stored).collect();
        for text in &stored {
            assert!(
                text.is_empty() || crate::pine::time::seconds_of_timeframe(text).is_some(),
                "{text}"
            );
        }
        let count = stored.len();
        stored.sort_unstable();
        stored.dedup();
        assert_eq!(stored.len(), count);
        // The chart's own is first, so it is what an empty value shows.
        assert_eq!(TIMEFRAMES[0], ("", "Chart"));
    }

    #[test]
    fn every_source_input_offers_resolves_as_a_series() {
        for name in SOURCES {
            assert!(
                crate::pine::builtin_types::identifier(name)
                    .kind()
                    .is_some(),
                "{name}"
            );
        }
    }

    fn string_input(control: InputControl, default: &str) -> InputDefinition {
        InputDefinition {
            control,
            ..def(
                "s",
                ValueType::String,
                InputValue::String(default.into()),
                None,
            )
        }
    }

    /// Draws `schema` on a headless frame and returns the edits it asked for (none, with no user).
    fn drawn(schema: &InputSchema) -> usize {
        let ctx = egui::Context::default();
        let mut edits = 0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            edits = show(ui, schema, &InputValues::default()).len();
        });
        output.textures_delta.clear();
        edits
    }

    #[test]
    fn every_kind_of_input_draws() {
        let schema = InputSchema {
            inputs: vec![
                string_input(InputControl::Symbol, "BINANCE:BTCUSDT"),
                string_input(InputControl::Timeframe, "60"),
                string_input(InputControl::Timeframe, "7"),
                string_input(InputControl::Session, "0930-1600"),
                string_input(InputControl::Session, "garbage"),
                string_input(InputControl::TextArea, "line one\nline two"),
                InputDefinition {
                    control: InputControl::Price,
                    ..def("p", ValueType::Float, InputValue::Float(1.5), None)
                },
                def(
                    "src",
                    ValueType::Float,
                    InputValue::Source("hl2".into()),
                    None,
                ),
            ],
        };
        assert_eq!(drawn(&schema), 0, "nobody touched anything");
    }

    /// Clicks where a card's switch sits (its right end, mid-height) and returns the edits asked for.
    fn clicked_switch(schema: &InputSchema, values: &InputValues) -> Vec<InputEdit> {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(480.0, 200.0));
        let frame = |events: Vec<egui::Event>, edits: &mut Vec<InputEdit>| {
            let mut used = egui::Rect::NOTHING;
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                *edits = cards(ui, schema, values);
                used = ui.min_rect();
            });
            output.textures_delta.clear();
            used
        };
        let mut ignored = Vec::new();
        let card_rect = frame(Vec::new(), &mut ignored);
        let at = egui::pos2(card_rect.right() - 14.0 - 18.0, card_rect.center().y);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at)], &mut ignored);
        frame(vec![button(true)], &mut ignored);
        let mut edits = Vec::new();
        frame(vec![button(false)], &mut edits);
        edits
    }

    #[test]
    fn a_greyed_out_input_cannot_be_changed_and_an_active_one_can() {
        let switch = |active: Option<crate::pine::ast::Expression>| InputSchema {
            inputs: vec![InputDefinition {
                active,
                ..def("flag", ValueType::Bool, InputValue::Bool(true), None)
            }],
        };
        let locked = crate::pine::ast::Expression::Literal(
            crate::pine::ast::Literal::Bool(false),
            crate::pine::token::SourceRange::ZERO,
        );
        let values = InputValues::default();
        assert_eq!(
            clicked_switch(&switch(None), &values),
            vec![InputEdit::Set("flag".into(), InputValue::Bool(false))],
            "the click lands on the switch"
        );
        assert_eq!(
            clicked_switch(&switch(Some(locked)), &values),
            Vec::<InputEdit>::new(),
            "the same click on a greyed-out input changes nothing"
        );
    }
}
