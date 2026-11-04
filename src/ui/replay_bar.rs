//! The strip above the charts while a replay is open: status, transport, speed and
//! resolution, the timeline, the cursor's clock and the way back to live. Pure view:
//! it reads the engine and returns `ReplayBarAction`s for the workspace to apply.

use crate::format::civil_from_days;
use crate::model::{ReplayEngine, ReplayInterval, ReplaySpeed, ReplayStatus};
use crate::service::timezone::local_utc_offset_secs;
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::theme;
use crate::ui::workspace_toolbar::paint_bubble;
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Key, Modifiers, Pos2, Rect, Response, Sense, Shape, Stroke,
    Vec2,
};

/// Below this width the timeline drops to a second row.
const WIDE_ROW_MIN_WIDTH: f32 = 1_040.0;
/// Below this width the start options collapse into one menu.
const START_OPTIONS_MIN_WIDTH: f32 = 330.0;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub struct ReplayBarInput<'a> {
    pub engine: &'a ReplayEngine,
    /// History is loading: transport and timeline are disabled.
    pub is_preparing: bool,
    /// A dismissible fallback notice.
    pub notice: Option<&'a str>,
    /// What Auto resolved to for the primary chart, when it has granular data.
    pub resolved_interval: Option<ReplayInterval>,
    pub available_intervals: &'a [ReplayInterval],
}

/// The height the bar needs: one row, or two when the timeline wraps, and a notice below.
pub fn preferred_height(status: ReplayStatus, width: f32, has_notice: bool) -> f32 {
    let rows = if matches!(status, ReplayStatus::Ready | ReplayStatus::SelectingStart)
        || width >= WIDE_ROW_MIN_WIDTH
    {
        ROW
    } else {
        ROW + SECOND_ROW
    };
    rows + if has_notice { NOTICE } else { 0.0 }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReplayBarAction {
    SelectOnChart,
    ChooseDate,
    RandomBar,
    FirstBar,
    CancelSelection,
    ReturnToLive,
    ChangeStart,
    Restart,
    StepBack,
    StepForward,
    TogglePlayback,
    /// Timeline fraction, 0…=1.
    Seek(f64),
    SetSpeed(ReplaySpeed),
    SetInterval(ReplayInterval),
    DismissNotice,
}

// MARK: Pure helpers

pub fn tint(status: ReplayStatus, preparing: bool) -> Color32 {
    if preparing {
        return Color32::from_gray(150);
    }
    match status {
        ReplayStatus::Playing => theme::UP,
        ReplayStatus::Completed => Color32::from_gray(150),
        _ => theme::REPLAY,
    }
}

pub fn title(status: ReplayStatus, preparing: bool) -> &'static str {
    if preparing {
        return "Loading history";
    }
    match status {
        ReplayStatus::Inactive => "Off",
        ReplayStatus::Ready => "Choose a start",
        ReplayStatus::SelectingStart => "Pick a start",
        ReplayStatus::Paused => "Paused",
        ReplayStatus::Playing => "Playing",
        ReplayStatus::Completed => "Ended",
    }
}

fn last_index(engine: &ReplayEngine) -> usize {
    engine.timeline.len().saturating_sub(1)
}

/// Cursor position along the timeline, 0…=1 (0 for a single bar).
pub fn progress(engine: &ReplayEngine) -> f64 {
    let Some(session) = &engine.session else {
        return 0.0;
    };
    match last_index(engine) {
        0 => 0.0,
        last => (session.bar_index.min(last) as f64) / last as f64,
    }
}

/// Where the replay started along the timeline, 0…=1.
pub fn start_fraction(engine: &ReplayEngine) -> f64 {
    let Some(session) = &engine.session else {
        return 0.0;
    };
    let last = last_index(engine);
    if last == 0 {
        return 0.0;
    }
    let index = engine
        .timeline
        .partition_point(|time| *time <= session.start_ms)
        .saturating_sub(1);
    index as f64 / last as f64
}

/// 1-based step number and total, for "Step 3 of 40".
pub fn step_of(engine: &ReplayEngine) -> (usize, usize) {
    let number = engine
        .session
        .as_ref()
        .map_or(0, |session| session.bar_index + 1);
    (number, engine.timeline.len())
}

pub fn can_step_back(engine: &ReplayEngine) -> bool {
    engine
        .session
        .as_ref()
        .is_some_and(|session| session.bar_index > 0)
}

pub fn can_advance(engine: &ReplayEngine) -> bool {
    engine
        .session
        .as_ref()
        .is_some_and(|session| session.bar_index + 1 < engine.timeline.len())
}

fn can_toggle(engine: &ReplayEngine, preparing: bool) -> bool {
    !preparing
        && matches!(
            engine.status,
            ReplayStatus::Paused | ReplayStatus::Playing | ReplayStatus::Completed
        )
}

fn split_utc(epoch_ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    (year, month, day, rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// "Oct 9, 2026"
pub fn date_text(epoch_ms: i64) -> String {
    let (year, month, day, ..) = split_utc(epoch_ms);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

/// "14:03:07"
pub fn time_text(epoch_ms: i64) -> String {
    let (_, _, _, hour, minute, second) = split_utc(epoch_ms);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// "Oct 9, 14:03"
pub fn compact_text(epoch_ms: i64) -> String {
    let local_ms = epoch_ms + local_utc_offset_secs(epoch_ms) * 1_000;
    let (_, month, day, hour, minute, _) = split_utc(local_ms);
    format!(
        "{} {day}, {hour:02}:{minute:02}",
        MONTHS[(month - 1) as usize]
    )
}

fn interval_text(engine: &ReplayEngine, resolved: Option<ReplayInterval>) -> String {
    let chosen = engine
        .session
        .as_ref()
        .map_or(ReplayInterval::Automatic, |session| session.interval);
    match (chosen, resolved) {
        (ReplayInterval::Automatic, Some(resolved)) => format!("Auto · {}", resolved.label()),
        _ => chosen.label().to_owned(),
    }
}

// MARK: Metrics

/// The height of one row of the bar.
const ROW: f32 = 40.0;
/// The second row a narrow bar wraps the timeline onto.
const SECOND_ROW: f32 = 30.0;
/// Chips, pills and icon buttons.
const CONTROL: f32 = 24.0;
/// The primary action of a start row.
const ACTION: f32 = 26.0;
const GAP: f32 = 8.0;
/// Width of the speed and resolution menus.
const MENU_WIDTH: f32 = 148.0;
/// Space the dismissible notice adds below the row.
const NOTICE: f32 = 38.0;

// MARK: Bar

pub fn show(ui: &mut egui::Ui, input: &ReplayBarInput) -> Vec<ReplayBarAction> {
    let mut actions = Vec::new();
    let engine = input.engine;

    let frame = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            match engine.status {
                ReplayStatus::Ready => ready_row(ui, input, &mut actions),
                ReplayStatus::SelectingStart => selecting_row(ui, input, &mut actions),
                _ if ui.available_width() >= WIDE_ROW_MIN_WIDTH => {
                    wide_row(ui, input, &mut actions);
                }
                _ => narrow_rows(ui, input, &mut actions),
            }
            if let Some(message) = input.notice {
                notice_chip(ui, message, &mut actions);
            }
        });
    if input.is_preparing {
        loading_sweep(ui, frame.response.rect);
    }

    shortcuts(ui, input, &mut actions);
    actions
}

fn shortcuts(ui: &egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    if !can_toggle(input.engine, input.is_preparing) || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    ui.input_mut(|state| {
        if state.consume_key(Modifiers::SHIFT, Key::ArrowLeft) && can_step_back(input.engine) {
            actions.push(ReplayBarAction::StepBack);
        }
        if state.consume_key(Modifiers::SHIFT, Key::ArrowRight) && can_advance(input.engine) {
            actions.push(ReplayBarAction::StepForward);
        }
        if state.consume_key(Modifiers::SHIFT, Key::ArrowDown) {
            toggle_actions(input.engine, actions);
        }
    });
}

/// At the end of the data play becomes "replay again".
fn toggle_actions(engine: &ReplayEngine, actions: &mut Vec<ReplayBarAction>) {
    if engine.status == ReplayStatus::Completed {
        actions.push(ReplayBarAction::Restart);
    }
    actions.push(ReplayBarAction::TogglePlayback);
}

// MARK: Rows

/// One row `height` tall: `left` fills from the left edge, `right` hugs the right edge and is laid
/// out first, so `left` gets what it leaves (and a flexible control in `left` can take the rest).
fn split_row(
    ui: &mut egui::Ui,
    height: f32,
    actions: &mut Vec<ReplayBarAction>,
    left: impl FnOnce(&mut egui::Ui, &mut Vec<ReplayBarAction>),
    right: impl FnOnce(&mut egui::Ui, &mut Vec<ReplayBarAction>),
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let mut right_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    right_ui.spacing_mut().item_spacing.x = GAP;
    right(&mut right_ui, actions);
    let taken = right_ui.min_rect().left() - GAP * 1.5;
    let left_rect = Rect::from_min_max(rect.min, Pos2::new(taken.max(rect.left()), rect.max.y));
    let mut left_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    left_ui.spacing_mut().item_spacing.x = GAP;
    left(&mut left_ui, actions);
}

fn wide_row(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    split_row(
        ui,
        ROW,
        actions,
        |ui, actions| {
            header(ui, input, true);
            divider(ui);
            transport(ui, input, actions);
            divider(ui);
            pickers(ui, input, actions);
            let timeline = (ui.available_width() - GAP).max(80.0);
            scrubber(ui, input, timeline, actions);
        },
        |ui, actions| {
            live_button(ui, actions);
            change_start_button(ui, actions);
            clock(ui, input.engine, false);
        },
    );
}

fn narrow_rows(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    let compact = ui.available_width() < 560.0;
    split_row(
        ui,
        ROW,
        actions,
        |ui, actions| {
            header(ui, input, !compact);
            transport(ui, input, actions);
        },
        |ui, actions| {
            live_button(ui, actions);
            change_start_button(ui, actions);
            clock(ui, input.engine, true);
        },
    );
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), SECOND_ROW), Sense::hover());
    let mut lane = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    lane.spacing_mut().item_spacing.x = GAP;
    pickers(&mut lane, input, actions);
    let timeline = (lane.available_width() - GAP).max(60.0);
    scrubber(&mut lane, input, timeline, actions);
}

fn ready_row(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    split_row(
        ui,
        ROW,
        actions,
        |ui, actions| {
            header(ui, input, ui.available_width() > 360.0);
            divider(ui);
            ui.add_enabled_ui(!input.is_preparing, |ui| start_options(ui, actions));
        },
        live_button,
    );
}

fn selecting_row(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    split_row(
        ui,
        ROW,
        actions,
        |ui, _| {
            header(ui, input, ui.available_width() > 460.0);
            divider(ui);
            let (icon, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_glyph(
                ui.painter(),
                icon.center(),
                Glyph::Scope,
                theme::text_primary(),
            );
            ui.label(kit::text(
                "Click a candle to start the replay",
                kit::size::BODY - 1.0,
                MEDIUM,
                theme::text_primary(),
            ));
            if ui.available_width() > 130.0 {
                keycap(ui, "Esc");
                ui.label(kit::text(
                    "to cancel",
                    kit::size::SUBHEADLINE + 0.5,
                    REGULAR,
                    theme::text_tertiary(),
                ));
            }
        },
        |ui, actions| {
            if pill_button(ui, "Cancel", "Cancel (Esc)").clicked() {
                actions.push(ReplayBarAction::CancelSelection);
            }
        },
    );
}

// MARK: Pieces

/// The mode's name and its state: `⟲ Replay  ● Paused`.
fn header(ui: &mut egui::Ui, input: &ReplayBarInput, show_name: bool) {
    let name = kit::galley(
        ui,
        "Replay",
        kit::size::HEADLINE - 0.5,
        SEMIBOLD,
        theme::text_primary(),
        f32::INFINITY,
    );
    let width = 15.0 + if show_name { 6.0 + name.size().x } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CONTROL), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.left() + 7.5, rect.center().y),
            Vec2::splat(15.0),
        ),
        Icon::ClockArrow,
        theme::REPLAY,
    );
    if show_name {
        ui.painter().galley(
            Pos2::new(rect.left() + 21.0, rect.center().y - name.size().y / 2.0),
            name,
            theme::text_primary(),
        );
    }
    response.on_hover_text("Historical replay");
    state_pill(ui, input.engine.status, input.is_preparing);
}

/// The state as a slim neutral pill with its coloured dot; the dot is the only colour.
fn state_pill(ui: &mut egui::Ui, status: ReplayStatus, preparing: bool) {
    let color = tint(status, preparing);
    let label = title(status, preparing);
    let pulse = status == ReplayStatus::Playing && !preparing;
    let text = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE + 0.5,
        MEDIUM,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(10.0 + 9.0 + 6.0 + text.size().x + 11.0, 22.0),
        Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 11.0, kit::primary(0.07));
    let dot = Pos2::new(rect.left() + 14.5, rect.center().y);
    if preparing {
        let turn = ui.input(|input| input.time) as f32 * 4.5;
        let arc: Vec<Pos2> = (0..=14)
            .map(|step| {
                let angle = turn + step as f32 / 14.0 * 4.2;
                dot + Vec2::new(angle.cos(), angle.sin()) * 4.0
            })
            .collect();
        painter.add(Shape::line(arc, Stroke::new(1.5, theme::text_secondary())));
        ui.ctx().request_repaint();
    } else {
        if pulse {
            let phase = (ui.input(|input| input.time) * 1.1).fract() as f32;
            painter.circle_filled(
                dot,
                3.0 + 4.5 * phase,
                color.gamma_multiply((1.0 - phase) * 0.35),
            );
            ui.ctx().request_repaint();
        }
        painter.circle_filled(dot, 3.0, color);
    }
    painter.galley(
        Pos2::new(rect.left() + 25.0, rect.center().y - text.size().y / 2.0),
        text,
        theme::text_secondary(),
    );
    response.on_hover_text(format!("Historical replay, {label}"));
}

fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 14.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, kit::primary(0.14));
}

/// A key as it is named in a hint.
fn keycap(ui: &mut egui::Ui, key: &str) {
    let text = kit::galley(
        ui,
        key,
        kit::size::CAPTION + 0.5,
        MEDIUM,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let (rect, _) = ui.allocate_exact_size(Vec2::new(text.size().x + 12.0, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 5.0, kit::primary(0.07));
    ui.painter().rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, kit::primary(0.12)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        rect.center() - text.size() / 2.0,
        text,
        theme::text_secondary(),
    );
}

/// A sliver sweeping along the bottom edge while history loads.
fn loading_sweep(ui: &egui::Ui, bar: Rect) {
    let length = bar.width() * 0.22;
    let phase = (ui.input(|input| input.time) * 0.7).fract() as f32;
    let left = bar.left() - length + (bar.width() + length) * phase;
    let sweep = Rect::from_min_max(
        Pos2::new(left.max(bar.left()), bar.bottom() - 2.0),
        Pos2::new((left + length).min(bar.right()), bar.bottom()),
    );
    if sweep.width() > 0.0 {
        ui.painter().rect_filled(sweep, 1.0, theme::REPLAY);
    }
    ui.ctx().request_repaint();
}

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Restart,
    StepBack,
    StepForward,
    Scope,
    Close,
    Play,
}

/// A 24pt round icon button that washes on hover.
fn glyph_button(ui: &mut egui::Ui, glyph: Glyph, enabled: bool, tip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(CONTROL),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = accessibility::label(response, tip).on_hover_text(tip);
    if enabled {
        let response = response
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.is_pointer_button_down_on() {
            ui.painter()
                .circle_filled(rect.center(), CONTROL / 2.0, kit::primary(0.16));
        } else if response.hovered() {
            ui.painter()
                .circle_filled(rect.center(), CONTROL / 2.0, kit::primary(0.09));
        }
    }
    let base = if !enabled {
        theme::text_tertiary().gamma_multiply(0.7)
    } else if response.hovered() {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    paint_glyph(ui.painter(), rect.center(), glyph, base);
    response
}

fn triangle(center: Pos2, half: f32, pointing_right: bool) -> Vec<Pos2> {
    let sign = if pointing_right { 1.0 } else { -1.0 };
    vec![
        Pos2::new(center.x + sign * half, center.y),
        Pos2::new(center.x - sign * half * 0.8, center.y - half),
        Pos2::new(center.x - sign * half * 0.8, center.y + half),
    ]
}

fn paint_glyph(painter: &egui::Painter, center: Pos2, glyph: Glyph, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    match glyph {
        Glyph::Restart => {
            painter.line_segment(
                [
                    center + Vec2::new(-5.0, -4.5),
                    center + Vec2::new(-5.0, 4.5),
                ],
                stroke,
            );
            painter.add(Shape::convex_polygon(
                triangle(center + Vec2::new(1.5, 0.0), 4.5, false),
                color,
                Stroke::NONE,
            ));
        }
        Glyph::StepBack => {
            painter.add(Shape::convex_polygon(
                triangle(center + Vec2::new(-1.0, 0.0), 4.5, false),
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [center + Vec2::new(4.5, -4.5), center + Vec2::new(4.5, 4.5)],
                stroke,
            );
        }
        Glyph::StepForward => {
            painter.line_segment(
                [
                    center + Vec2::new(-4.5, -4.5),
                    center + Vec2::new(-4.5, 4.5),
                ],
                stroke,
            );
            painter.add(Shape::convex_polygon(
                triangle(center + Vec2::new(1.0, 0.0), 4.5, true),
                color,
                Stroke::NONE,
            ));
        }
        Glyph::Play => {
            painter.add(Shape::convex_polygon(
                triangle(center + Vec2::new(0.5, 0.0), 3.5, true),
                color,
                Stroke::NONE,
            ));
        }
        Glyph::Scope => {
            painter.circle_stroke(center, 5.0, Stroke::new(1.4, color));
            painter.circle_filled(center, 1.5, color);
            for (from, to) in [
                (Vec2::new(0.0, -7.5), Vec2::new(0.0, -4.0)),
                (Vec2::new(0.0, 4.0), Vec2::new(0.0, 7.5)),
                (Vec2::new(-7.5, 0.0), Vec2::new(-4.0, 0.0)),
                (Vec2::new(4.0, 0.0), Vec2::new(7.5, 0.0)),
            ] {
                painter.line_segment([center + from, center + to], Stroke::new(1.2, color));
            }
        }
        Glyph::Close => {
            for (a, b) in [
                (Vec2::new(-3.5, -3.5), Vec2::new(3.5, 3.5)),
                (Vec2::new(-3.5, 3.5), Vec2::new(3.5, -3.5)),
            ] {
                painter.line_segment([center + a, center + b], stroke);
            }
        }
    }
}

fn transport(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    let engine = input.engine;
    let busy = input.is_preparing;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if glyph_button(
            ui,
            Glyph::Restart,
            !busy && engine.session.is_some(),
            "Back to start",
        )
        .clicked()
        {
            actions.push(ReplayBarAction::Restart);
        }
        if glyph_button(
            ui,
            Glyph::StepBack,
            !busy && can_step_back(engine),
            "Step Back (Shift–Left Arrow)",
        )
        .clicked()
        {
            actions.push(ReplayBarAction::StepBack);
        }
        play_button(ui, input, actions);
        if glyph_button(
            ui,
            Glyph::StepForward,
            !busy && can_advance(engine),
            "Step Forward (Shift–Right Arrow)",
        )
        .clicked()
        {
            actions.push(ReplayBarAction::StepForward);
        }
    });
}

/// The one solid control on the bar: a neutral disc, inked like the label colour.
fn play_button(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    let engine = input.engine;
    let enabled = can_toggle(engine, input.is_preparing);
    let ended = engine.status == ReplayStatus::Completed;
    let playing = engine.status == ReplayStatus::Playing;
    let verb = if ended {
        "Replay Again"
    } else if playing {
        "Pause"
    } else {
        "Play"
    };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(30.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = accessibility::label(response, verb);
    let (fill, glyph) = if !enabled {
        (kit::primary(0.12), theme::text_tertiary())
    } else if response.is_pointer_button_down_on() {
        (theme::text_primary().gamma_multiply(0.78), theme::chrome())
    } else if response.hovered() {
        (theme::text_primary(), theme::chrome())
    } else {
        (theme::text_primary().gamma_multiply(0.92), theme::chrome())
    };
    let painter = ui.painter();
    if enabled {
        painter.add(
            egui::Shadow {
                offset: [0, 1],
                blur: 4,
                spread: 0,
                color: Color32::from_black_alpha(55),
            }
            .as_shape(rect.shrink(1.0), 14.0),
        );
    }
    painter.circle_filled(rect.center(), 14.0, fill);
    if ended {
        // A counter-clockwise arc, open at the top right, ending in an arrow head.
        let radius = 5.2;
        let (from, to) = (1.5_f32, 1.5 + 5.0);
        let at = |angle: f32| rect.center() + Vec2::new(angle.cos(), -angle.sin()) * radius;
        let arc: Vec<Pos2> = (0..=24)
            .map(|step| at(from + (to - from) * step as f32 / 24.0))
            .collect();
        painter.add(Shape::line(arc, Stroke::new(1.8, glyph)));
        let tip_direction = Vec2::new(-to.sin(), -to.cos());
        let side = Vec2::new(-tip_direction.y, tip_direction.x);
        let base = at(to) - tip_direction * 0.5;
        painter.add(Shape::convex_polygon(
            vec![
                base + tip_direction * 4.2,
                base + side * 3.2,
                base - side * 3.2,
            ],
            glyph,
            Stroke::NONE,
        ));
    } else if playing {
        for offset in [-3.0, 3.0] {
            painter.rect_filled(
                Rect::from_center_size(
                    rect.center() + Vec2::new(offset, 0.0),
                    Vec2::new(3.0, 10.0),
                ),
                1.0,
                glyph,
            );
        }
    } else {
        painter.add(Shape::convex_polygon(
            triangle(rect.center() + Vec2::new(1.0, 0.0), 5.5, true),
            glyph,
            Stroke::NONE,
        ));
    }
    if response
        .on_hover_text(format!("{verb} (Shift–Down Arrow)"))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && enabled
    {
        toggle_actions(engine, actions);
    }
}

fn pickers(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    let engine = input.engine;
    let speed = engine.session.as_ref().map(|session| session.speed);
    let labels: Vec<&str> = ReplaySpeed::ALL.iter().map(|s| s.label()).collect();
    if let Some(index) = picker_chip(
        ui,
        PickerChip {
            caption: "Speed",
            value: speed.unwrap_or(ReplaySpeed::Normal).label(),
            tip: "Playback speed",
            options: &labels,
            selected: speed.and_then(|s| ReplaySpeed::ALL.iter().position(|o| *o == s)),
            enabled: true,
        },
    ) {
        actions.push(ReplayBarAction::SetSpeed(ReplaySpeed::ALL[index]));
    }
    let chosen = engine.session.as_ref().map(|session| session.interval);
    let intervals = input.available_intervals;
    let labels: Vec<&str> = intervals.iter().map(|i| i.label()).collect();
    if let Some(index) = picker_chip(
        ui,
        PickerChip {
            caption: "Bars",
            value: &interval_text(engine, input.resolved_interval),
            tip: "Replay resolution. Auto picks the finest history that fits; 1 bar steps whole chart bars.",
            options: &labels,
            selected: chosen.and_then(|c| intervals.iter().position(|o| *o == c)),
            enabled: !input.is_preparing,
        },
    ) {
        actions.push(ReplayBarAction::SetInterval(intervals[index]));
    }
}

struct PickerChip<'a> {
    caption: &'a str,
    value: &'a str,
    tip: &'a str,
    options: &'a [&'a str],
    selected: Option<usize>,
    enabled: bool,
}

/// `Speed 1× ⌃⌄`: a capsule that opens the options as a menu. Returns the index chosen.
fn picker_chip(ui: &mut egui::Ui, chip: PickerChip) -> Option<usize> {
    let caption = kit::galley(
        ui,
        chip.caption,
        kit::size::CAPTION + 0.5,
        MEDIUM,
        theme::text_tertiary(),
        f32::INFINITY,
    );
    let value_color = if chip.enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let value = kit::galley(
        ui,
        chip.value,
        kit::size::SUBHEADLINE + 0.5,
        SEMIBOLD,
        value_color,
        150.0,
    );
    let width = 10.0 + caption.size().x + 5.0 + value.size().x + 5.0 + 10.0 + 8.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, CONTROL),
        if chip.enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = accessibility::label(response, format!("{}: {}", chip.caption, chip.value))
        .on_hover_text(chip.tip);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    paint_bubble(
        ui,
        rect,
        chip.enabled && response.hovered(),
        chip.enabled && (response.is_pointer_button_down_on() || open),
    );
    let painter = ui.painter();
    let mut x = rect.left() + 10.0;
    painter.galley(
        Pos2::new(x, rect.center().y - caption.size().y / 2.0),
        caption.clone(),
        theme::text_tertiary(),
    );
    x += caption.size().x + 5.0;
    painter.galley(
        Pos2::new(x, rect.center().y - value.size().y / 2.0),
        value.clone(),
        value_color,
    );
    icons::paint(
        painter,
        Rect::from_center_size(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ChevronUpDown,
        theme::text_tertiary(),
    );
    if !chip.enabled {
        return None;
    }
    response
        .clone()
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut picked = None;
    kit::menu_popup(&response).width(MENU_WIDTH).show(|ui| {
        kit::menu_scope(ui, MENU_WIDTH - kit::MENU_FRAME_MARGIN);
        kit::menu_heading(
            ui,
            match chip.caption {
                "Speed" => "Playback speed",
                _ => "Bar resolution",
            },
        );
        for (index, label) in chip.options.iter().enumerate() {
            let row = kit::menu_row(
                ui,
                kit::MenuRow {
                    current: chip.selected == Some(index),
                    reserve_icon: false,
                    ..kit::MenuRow::new(label)
                },
            );
            if row.clicked() {
                picked = Some(index);
                ui.close();
            }
        }
    });
    picked
}

/// The timeline: where the replay began, how far it has come, and (on hover) when a click lands.
fn scrubber(
    ui: &mut egui::Ui,
    input: &ReplayBarInput,
    width: f32,
    actions: &mut Vec<ReplayBarAction>,
) {
    let engine = input.engine;
    let enabled = !input.is_preparing && engine.session.is_some();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, CONTROL),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let inset = 7.0;
    let span = Rect::from_x_y_ranges(
        (rect.left() + inset)..=(rect.right() - inset),
        rect.y_range(),
    );
    let x_at = |fraction: f32| span.left() + span.width() * fraction;
    let engaged = enabled && (response.hovered() || response.dragged());
    let grow = ui
        .ctx()
        .animate_bool_with_time(response.id.with("grow"), engaged, 0.12);
    let dim = if enabled { 1.0 } else { 0.45 };

    let thickness = 4.0 + 2.0 * grow;
    let radius = (thickness / 2.0).round() as u8;
    let track = Rect::from_center_size(rect.center(), Vec2::new(span.width(), thickness));
    let painter = ui.painter();
    painter.rect_filled(track, f32::from(radius), kit::primary(0.12 * dim));
    if engine.session.is_some() {
        let now = x_at(progress(engine) as f32);
        let begin = x_at(start_fraction(engine) as f32);
        // What was already on the chart before the replay began.
        if begin > track.left() + 0.5 {
            painter.rect_filled(
                Rect::from_min_max(track.min, Pos2::new(begin, track.max.y)),
                egui::CornerRadius {
                    nw: radius,
                    sw: radius,
                    ne: 0,
                    se: 0,
                },
                kit::primary(0.28 * dim),
            );
        }
        // What has been replayed.
        if now > begin + 0.5 {
            let left_round = if begin <= track.left() + 0.5 {
                radius
            } else {
                0
            };
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(begin, track.min.y), Pos2::new(now, track.max.y)),
                egui::CornerRadius {
                    nw: left_round,
                    sw: left_round,
                    ne: radius,
                    se: radius,
                },
                theme::REPLAY.gamma_multiply(dim),
            );
        }
        painter.rect_filled(
            Rect::from_center_size(
                Pos2::new(begin, rect.center().y),
                Vec2::new(2.0, 12.0 + 2.0 * grow),
            ),
            1.0,
            kit::primary(0.55 * dim),
        );
        if let (true, Some(hover)) = (engaged && !response.dragged(), response.hover_pos()) {
            painter.vline(
                hover.x.clamp(span.left(), span.right()),
                (rect.center().y - 5.0)..=(rect.center().y + 5.0),
                Stroke::new(1.0, kit::primary(0.35)),
            );
        }
        let thumb = Pos2::new(now, rect.center().y);
        let size = 4.0 + 2.5 * grow;
        painter.add(
            egui::Shadow {
                offset: [0, 1],
                blur: 4,
                spread: 0,
                color: Color32::from_black_alpha((90.0 * dim) as u8),
            }
            .as_shape(Rect::from_center_size(thumb, Vec2::splat(size * 2.0)), size),
        );
        painter.circle(
            thumb,
            size,
            Color32::WHITE.gamma_multiply(dim),
            Stroke::new(1.0, kit::primary(0.22 * dim)),
        );
    }
    if enabled
        && (response.dragged() || response.clicked())
        && let Some(position) = response.interact_pointer_pos()
    {
        let target = ((position.x - span.left()) / span.width()).clamp(0.0, 1.0);
        actions.push(ReplayBarAction::Seek(f64::from(target)));
    }
    let response = response.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    });
    let (number, count) = step_of(engine);
    match (enabled, response.hover_pos()) {
        (true, Some(hover)) if !response.dragged() => {
            let fraction = ((hover.x - span.left()) / span.width()).clamp(0.0, 1.0);
            let index = (fraction * last_index(engine) as f32).round() as usize;
            let when = engine
                .timeline
                .get(index)
                .map_or_else(String::new, |ms| compact_text(*ms));
            response.on_hover_text_at_pointer(format!("{when}  ·  Step {} of {count}", index + 1));
        }
        _ => {
            response.on_hover_text(format!("Step {number} of {count}"));
        }
    }
}

/// `Oct 9, 2026  14:03:07`, right-aligned in a slot that does not move as the digits change.
fn clock(ui: &mut egui::Ui, engine: &ReplayEngine, compact: bool) {
    let Some(ms) = engine.current_ms() else {
        ui.label(kit::text(
            "—",
            kit::size::BODY,
            REGULAR,
            theme::text_tertiary(),
        ));
        return;
    };
    let digits = |ui: &egui::Ui, text: &str, size: f32, weight: f32, color: Color32| {
        kit::digits(ui, text, size, weight, color, f32::INFINITY)
    };
    let (date, time, reference) = if compact {
        (None, compact_text(ms), "May 28, 88:88".to_owned())
    } else {
        let local = ms + local_utc_offset_secs(ms) * 1_000;
        (
            Some(date_text(local)),
            time_text(local),
            "May 28, 2026 88:88:88".to_owned(),
        )
    };
    let time_galley = digits(ui, &time, 12.5, SEMIBOLD, theme::text_primary());
    let date_galley = date
        .as_deref()
        .map(|date| digits(ui, date, 11.5, REGULAR, theme::text_secondary()));
    let used = time_galley.size().x + date_galley.as_ref().map_or(0.0, |d| d.size().x + 8.0);
    let slot = digits(ui, &reference, 12.5, SEMIBOLD, Color32::PLACEHOLDER)
        .size()
        .x
        .max(used);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(slot, CONTROL), Sense::hover());
    let mut right = rect.right();
    let painter = ui.painter();
    right -= time_galley.size().x;
    painter.galley(
        Pos2::new(right, rect.center().y - time_galley.size().y / 2.0),
        time_galley,
        theme::text_primary(),
    );
    if let Some(date_galley) = date_galley {
        right -= 8.0 + date_galley.size().x;
        painter.galley(
            Pos2::new(right, rect.center().y - date_galley.size().y / 2.0),
            date_galley,
            theme::text_secondary(),
        );
    }
    response.on_hover_text("Time of the replay cursor, in your time zone");
}

fn change_start_button(ui: &mut egui::Ui, actions: &mut Vec<ReplayBarAction>) {
    if glyph_button(ui, Glyph::Scope, true, "Choose a new starting point").clicked() {
        actions.push(ReplayBarAction::ChangeStart);
    }
}

/// The way out. One control, named for where it goes: the green dot is the market being live.
fn live_button(ui: &mut egui::Ui, actions: &mut Vec<ReplayBarAction>) {
    let label = kit::galley(
        ui,
        "Live",
        kit::size::SUBHEADLINE + 0.5,
        SEMIBOLD,
        theme::text_primary(),
        f32::INFINITY,
    );
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(11.0 + 6.0 + 6.0 + label.size().x + 12.0, CONTROL),
        Sense::click(),
    );
    paint_bubble(
        ui,
        rect,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    let painter = ui.painter();
    let dot = Pos2::new(rect.left() + 14.0, rect.center().y);
    painter.circle_filled(dot, 4.5, theme::UP.gamma_multiply(0.22));
    painter.circle_filled(dot, 2.5, theme::UP);
    painter.galley(
        Pos2::new(rect.left() + 25.0, rect.center().y - label.size().y / 2.0),
        label,
        theme::text_primary(),
    );
    if accessibility::label(response, "Return to live market")
        .on_hover_text("Leave replay and return to the live market")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        actions.push(ReplayBarAction::ReturnToLive);
    }
}

/// A quiet text capsule.
fn pill_button(ui: &mut egui::Ui, label: &str, tip: &str) -> Response {
    let text = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE + 0.5,
        MEDIUM,
        theme::text_primary(),
        f32::INFINITY,
    );
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(text.size().x + 24.0, CONTROL), Sense::click());
    paint_bubble(
        ui,
        rect,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    ui.painter().galley(
        rect.center() - text.size() / 2.0,
        text,
        theme::text_primary(),
    );
    accessibility::label(response, label)
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The start row's main action: a solid capsule inked like the label colour, so it is the one
/// thing to press without borrowing an accent.
fn primary_button(
    ui: &mut egui::Ui,
    glyph: Glyph,
    label: &str,
    dropdown: bool,
    tip: &str,
) -> Response {
    let ink = theme::chrome();
    let text = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE + 0.5,
        SEMIBOLD,
        ink,
        f32::INFINITY,
    );
    let width = 11.0 + 15.0 + 5.0 + text.size().x + if dropdown { 7.0 + 8.0 } else { 0.0 } + 13.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ACTION), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        theme::text_primary().gamma_multiply(0.78)
    } else if response.hovered() {
        theme::text_primary()
    } else {
        theme::text_primary().gamma_multiply(0.92)
    };
    let painter = ui.painter();
    painter.add(
        egui::Shadow {
            offset: [0, 1],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(50),
        }
        .as_shape(rect.shrink(1.0), ACTION / 2.0),
    );
    painter.rect_filled(rect, ACTION / 2.0, fill);
    paint_glyph(
        painter,
        Pos2::new(rect.left() + 11.0 + 7.5, rect.center().y),
        glyph,
        ink,
    );
    let text_left = rect.left() + 11.0 + 15.0 + 5.0;
    let text_width = text.size().x;
    painter.galley(
        Pos2::new(text_left, rect.center().y - text.size().y / 2.0),
        text,
        ink,
    );
    if dropdown {
        icons::paint(
            painter,
            Rect::from_center_size(
                Pos2::new(text_left + text_width + 7.0 + 4.0, rect.center().y),
                Vec2::splat(9.0),
            ),
            Icon::ChevronDown,
            ink,
        );
    }
    accessibility::label(response, label)
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy)]
enum StartIcon {
    Calendar,
    Dice,
    FirstBar,
}

struct StartChoice {
    title: &'static str,
    tip: &'static str,
    icon: StartIcon,
    action: ReplayBarAction,
}

const START_CHOICES: [StartChoice; 3] = [
    StartChoice {
        title: "Date & Time",
        tip: "Pick a date and time to start from",
        icon: StartIcon::Calendar,
        action: ReplayBarAction::ChooseDate,
    },
    StartChoice {
        title: "Random",
        tip: "Start from a random bar, leaving room to play forward",
        icon: StartIcon::Dice,
        action: ReplayBarAction::RandomBar,
    },
    StartChoice {
        title: "First Bar",
        tip: "Start from the first bar the chart has loaded",
        icon: StartIcon::FirstBar,
        action: ReplayBarAction::FirstBar,
    },
];

/// How much room the start row has decides how much of it is spelled out.
fn start_options(ui: &mut egui::Ui, actions: &mut Vec<ReplayBarAction>) {
    let room = ui.available_width();
    if room < START_OPTIONS_MIN_WIDTH {
        let response = primary_button(
            ui,
            Glyph::Play,
            "Start",
            true,
            "Choose where the replay starts",
        );
        kit::menu_popup(&response)
            .width(MENU_WIDTH + 36.0)
            .show(|ui| {
                kit::menu_scope(ui, MENU_WIDTH + 36.0 - kit::MENU_FRAME_MARGIN);
                let mut entries = vec![("Select Bar on Chart", ReplayBarAction::SelectOnChart)];
                entries.extend(START_CHOICES.iter().map(|c| (c.title, c.action)));
                for (label, action) in entries {
                    let row = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            reserve_icon: false,
                            ..kit::MenuRow::new(label)
                        },
                    );
                    if row.clicked() {
                        actions.push(action);
                        ui.close();
                    }
                }
            });
        return;
    }
    let labels = room >= 650.0;
    if primary_button(
        ui,
        Glyph::Scope,
        if labels {
            "Select bar on chart"
        } else {
            "Select"
        },
        false,
        "Click a candle on a chart to start the replay there",
    )
    .clicked()
    {
        actions.push(ReplayBarAction::SelectOnChart);
    }
    if labels {
        ui.label(kit::text(
            "or",
            kit::size::SUBHEADLINE,
            REGULAR,
            theme::text_tertiary(),
        ));
    }
    start_group(ui, labels, actions);
}

/// Date & Time, Random and First Bar as one capsule of three segments.
fn start_group(ui: &mut egui::Ui, labels: bool, actions: &mut Vec<ReplayBarAction>) {
    let galleys: Vec<_> = START_CHOICES
        .iter()
        .map(|choice| {
            kit::galley(
                ui,
                choice.title,
                kit::size::SUBHEADLINE + 0.5,
                MEDIUM,
                theme::text_primary(),
                f32::INFINITY,
            )
        })
        .collect();
    let widths: Vec<f32> = galleys
        .iter()
        .map(|galley| {
            if labels {
                11.0 + 12.0 + 6.0 + galley.size().x + 12.0
            } else {
                32.0
            }
        })
        .collect();
    let (track, _) = ui.allocate_exact_size(
        Vec2::new(widths.iter().sum::<f32>() + 4.0, ACTION),
        Sense::hover(),
    );
    paint_bubble(ui, track, false, false);
    let mut x = track.left() + 2.0;
    for ((choice, galley), width) in START_CHOICES.iter().zip(galleys).zip(widths) {
        let segment = Rect::from_min_size(
            Pos2::new(x, track.top() + 2.0),
            Vec2::new(width, track.height() - 4.0),
        );
        x += width;
        let response = ui.interact(
            segment,
            Id::new(("replay-start", choice.title)),
            Sense::click(),
        );
        let hovered = response.hovered();
        if response.is_pointer_button_down_on() {
            ui.painter()
                .rect_filled(segment, segment.height() / 2.0, kit::primary(0.16));
        } else if hovered {
            ui.painter()
                .rect_filled(segment, segment.height() / 2.0, kit::primary(0.10));
        }
        let color = if hovered {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let icon_left = if labels {
            segment.left() + 11.0
        } else {
            segment.center().x - 6.0
        };
        paint_start_icon(
            ui.painter(),
            Rect::from_min_size(
                Pos2::new(icon_left, segment.center().y - 6.0),
                Vec2::splat(12.0),
            ),
            choice.icon,
            color,
        );
        if labels {
            ui.painter().galley(
                Pos2::new(
                    segment.left() + 11.0 + 12.0 + 6.0,
                    segment.center().y - galley.size().y / 2.0,
                ),
                galley,
                theme::text_primary(),
            );
        }
        if accessibility::label(response, choice.title)
            .on_hover_text(choice.tip)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            actions.push(choice.action);
        }
    }
}

fn paint_start_icon(painter: &egui::Painter, rect: Rect, icon: StartIcon, color: Color32) {
    match icon {
        StartIcon::Calendar => icons::paint(painter, rect, Icon::Calendar, color),
        StartIcon::Dice => {
            painter.rect_stroke(
                rect.shrink(0.8),
                2.5,
                Stroke::new(1.2, color),
                egui::StrokeKind::Middle,
            );
            for (x, y) in [(3.5, 3.5), (8.5, 3.5), (6.0, 6.0), (3.5, 8.5), (8.5, 8.5)] {
                painter.circle_filled(rect.min + Vec2::new(x, y), 0.8, color);
            }
        }
        StartIcon::FirstBar => paint_glyph(painter, rect.center(), Glyph::Restart, color),
    }
}

fn notice_chip(ui: &mut egui::Ui, message: &str, actions: &mut Vec<ReplayBarAction>) {
    ui.add_space((NOTICE - 26.0) / 2.0);
    egui::Frame::new()
        .fill(kit::primary(0.07))
        .stroke(Stroke::new(1.0, kit::primary(0.06)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                icons::paint(ui.painter(), icon, Icon::Triangle, theme::REPLAY);
                ui.label(kit::text(
                    message,
                    kit::size::SUBHEADLINE + 0.5,
                    REGULAR,
                    theme::text_secondary(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if glyph_button(ui, Glyph::Close, true, "Dismiss").clicked() {
                        actions.push(ReplayBarAction::DismissNotice);
                    }
                });
            });
        });
}

/// Small text badge used on a replaying chart's plot.
pub fn badge(painter: &egui::Painter, top_right: Pos2) {
    let text = "Replay";
    let font = FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE);
    let size = galley.size() + Vec2::new(14.0, 6.0);
    let rect = Rect::from_min_size(Pos2::new(top_right.x - size.x, top_right.y), size);
    painter.rect_filled(rect, 999.0, theme::REPLAY);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        font,
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> ReplayEngine {
        let mut engine = ReplayEngine::default();
        engine.start(1_100, "BTCUSDT", (0..5).map(|i| 1_000 + i * 100).collect());
        engine
    }

    #[test]
    fn progress_and_start_tick_follow_the_timeline() {
        let mut engine = engine();
        assert_eq!(start_fraction(&engine), 0.25);
        assert_eq!(progress(&engine), 0.25);
        engine.seek(1.0);
        assert_eq!(progress(&engine), 1.0);
        assert_eq!(start_fraction(&engine), 0.25);
    }

    #[test]
    fn single_bar_and_no_session_do_not_divide_by_zero() {
        assert_eq!(progress(&ReplayEngine::default()), 0.0);
        let mut one = ReplayEngine::default();
        one.start(5, "X", vec![5]);
        assert_eq!((progress(&one), start_fraction(&one)), (0.0, 0.0));
    }

    #[test]
    fn step_gates() {
        let mut engine = engine();
        assert!(can_step_back(&engine) && can_advance(&engine));
        assert_eq!(step_of(&engine), (2, 5));
        engine.seek(0.0);
        assert!(!can_step_back(&engine));
        engine.seek(1.0);
        assert!(!can_advance(&engine));
        assert!(!can_step_back(&ReplayEngine::default()));
    }

    #[test]
    fn titles_and_tints() {
        assert_eq!(title(ReplayStatus::Playing, false), "Playing");
        assert_eq!(title(ReplayStatus::Playing, true), "Loading history");
        assert_eq!(tint(ReplayStatus::Playing, false), theme::UP);
        assert_eq!(tint(ReplayStatus::Paused, false), theme::REPLAY);
        assert_eq!(tint(ReplayStatus::Playing, true), Color32::from_gray(150));
    }

    #[test]
    fn utc_date_and_time_text() {
        // 2023-11-14 22:13:20 UTC
        let ms = 1_700_000_000_000;
        assert_eq!(date_text(ms), "Nov 14, 2023");
        assert_eq!(time_text(ms), "22:13:20");
        assert_eq!(compact_text(ms), "Nov 14, 22:13");
        assert_eq!(date_text(0), "Jan 1, 1970");
    }

    #[test]
    fn interval_text_shows_the_resolved_auto() {
        let engine = engine();
        assert_eq!(interval_text(&engine, None), "Auto");
        assert_eq!(
            interval_text(&engine, Some(ReplayInterval::FiveMinutes)),
            "Auto · 5m"
        );
    }

    #[test]
    fn completed_toggle_restarts_first() {
        let mut engine = engine();
        engine.seek(1.0);
        let mut actions = Vec::new();
        toggle_actions(&engine, &mut actions);
        assert_eq!(
            actions,
            vec![ReplayBarAction::Restart, ReplayBarAction::TogglePlayback]
        );
    }

    #[test]
    fn wrapped_bar_reserves_a_second_row() {
        assert_eq!(preferred_height(ReplayStatus::Ready, 500.0, false), 40.0);
        assert_eq!(preferred_height(ReplayStatus::Paused, 500.0, false), 70.0);
        assert_eq!(preferred_height(ReplayStatus::Paused, 1200.0, true), 78.0);
    }

    /// Written by hand: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib replay_bar::tests::gallery -- --ignored`.
    #[test]
    #[ignore = "writes pictures"]
    fn gallery_of_the_bar() {
        use crate::ui::render_tests::Screen;
        let _restore = theme::RestoreDefaultTheme;
        let timeline: Vec<i64> = (0..400).map(|i| 1_700_000_000_000 + i * 300_000).collect();
        let started = |index: usize, status: ReplayStatus| {
            let mut engine = ReplayEngine::default();
            engine.start(timeline[40], "BTCUSDT", timeline.clone());
            engine.seek(index as f64 / 399.0);
            engine.status = status;
            engine
        };
        let mut ready = ReplayEngine::default();
        ready.open();
        let mut selecting = ReplayEngine::default();
        selecting.open();
        selecting.begin_selecting();
        let cases: Vec<(ReplayEngine, bool, Option<&str>)> = vec![
            (ready, false, None),
            (selecting, false, None),
            (started(120, ReplayStatus::Paused), false, None),
            (started(210, ReplayStatus::Playing), false, None),
            (started(399, ReplayStatus::Completed), false, None),
            (started(60, ReplayStatus::Paused), true, None),
            (
                started(80, ReplayStatus::Paused),
                false,
                Some("This chart has no 1m history; replaying whole bars."),
            ),
        ];
        let available = [ReplayInterval::Automatic, ReplayInterval::ChartBar];
        for (name, width, light) in [
            ("replay-wide-dark", 1_240.0, false),
            ("replay-wide-light", 1_240.0, true),
            ("replay-mid-dark", 820.0, false),
            ("replay-mid-light", 820.0, true),
            ("replay-narrow-dark", 460.0, false),
        ] {
            let height: f32 = cases
                .iter()
                .map(|(engine, _, notice)| {
                    preferred_height(engine.status, width, notice.is_some()) + 1.0
                })
                .sum::<f32>()
                + 10.0;
            let mut screen = Screen::new(Vec2::new(width, height));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let hover = Pos2::new(width * 0.45, 100.0);
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(vec![egui::Event::PointerMoved(hover)], |ui| {
                    theme::sync(ui.ctx());
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::background());
                    for (index, (engine, preparing, notice)) in cases.iter().enumerate() {
                        let input = ReplayBarInput {
                            engine,
                            is_preparing: *preparing,
                            notice: *notice,
                            resolved_interval: Some(ReplayInterval::FiveMinutes),
                            available_intervals: &available,
                        };
                        egui::Panel::top(egui::Id::new(("bar", index)))
                            .exact_size(preferred_height(engine.status, width, notice.is_some()))
                            .frame(egui::Frame::new().fill(theme::chrome()))
                            .show(ui, |ui| {
                                show(ui, &input);
                            });
                    }
                }));
            }
            screen.save(name, last.unwrap());
        }
    }

    /// The speed menu, open on a paused bar.
    #[test]
    #[ignore = "writes pictures"]
    fn gallery_of_the_open_menu() {
        use crate::ui::render_tests::Screen;
        let _restore = theme::RestoreDefaultTheme;
        let timeline: Vec<i64> = (0..400).map(|i| 1_700_000_000_000 + i * 300_000).collect();
        let mut engine = ReplayEngine::default();
        engine.start(timeline[40], "BTCUSDT", timeline.clone());
        engine.seek(0.3);
        let available = [ReplayInterval::Automatic, ReplayInterval::ChartBar];
        for (name, light) in [("replay-menu-dark", false), ("replay-menu-light", true)] {
            let mut screen = Screen::new(Vec2::new(1_240.0, 330.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let chip = Pos2::new(330.0, 20.0);
            let press = |pressed| egui::Event::PointerButton {
                pos: chip,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut last = None;
            for step in 0..6 {
                let events = match step {
                    0 | 1 => vec![egui::Event::PointerMoved(chip)],
                    2 => vec![press(true)],
                    3 => vec![press(false)],
                    _ => vec![],
                };
                last = Some(screen.run(events, |ui| {
                    theme::sync(ui.ctx());
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::background());
                    let input = ReplayBarInput {
                        engine: &engine,
                        is_preparing: false,
                        notice: None,
                        resolved_interval: Some(ReplayInterval::FiveMinutes),
                        available_intervals: &available,
                    };
                    egui::Panel::top("bar")
                        .exact_size(ROW)
                        .frame(egui::Frame::new().fill(theme::chrome()))
                        .show(ui, |ui| {
                            show(ui, &input);
                        });
                }));
            }
            screen.save(name, last.unwrap());
        }
    }
}
