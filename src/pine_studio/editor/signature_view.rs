//! The signature panel over the caret line while the caret is inside a call.

use super::{EditorState, signature};
use crate::model::editor_theme::{SurfaceColor, SyntaxRole};
use crate::ui::editor_appearance::{Appearance, EditorFont};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Margin, Stroke, Ui, Vec2, text_edit::TextEditState};

/// The signature panel over the caret line: `ta.sma(source, length) → series float`, the
/// argument under the caret underlined. Quiet while the suggestion list is open.
pub(super) fn draw_signature(
    ui: &mut Ui,
    state: &mut EditorState,
    look: &Appearance,
    code: &EditorFont,
) {
    let ctx = ui.ctx().clone();
    if state.completion.is_some()
        || state.caret_hidden
        || !ctx.memory(|memory| memory.has_focus(state.id))
    {
        return;
    }
    let Some(caret_rect) = state.caret_rect else {
        return;
    };
    let Some(range) = TextEditState::load(&ctx, state.id).and_then(|edit| edit.cursor.char_range())
    else {
        return;
    };
    if range.primary != range.secondary {
        return;
    }
    let table = state.symbols();
    let Some(help) = signature::help_with(&state.source, range.primary.index.0, &table) else {
        return;
    };
    let text = look.role(SyntaxRole::Default);
    let dim = look.surface(SurfaceColor::LineNumber);
    let mono = code.at(&ctx, 12.0 * code.chrome_scale());
    let plain = |color: Color32| egui::TextFormat::simple(mono.clone(), color);
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("{}(", help.name), 0.0, plain(text));
    for (index, parameter) in help.parameters.iter().enumerate() {
        if index > 0 {
            job.append(", ", 0.0, plain(text));
        }
        let mut format = plain(if Some(index) == help.active {
            Color32::WHITE
        } else {
            text
        });
        if Some(index) == help.active {
            format.underline = Stroke::new(1.5, look.surface(SurfaceColor::PopupSelection));
        }
        job.append(parameter, 0.0, format);
    }
    job.append(")", 0.0, plain(text));
    if !help.returns.is_empty() {
        job.append(&format!(" → {}", help.returns), 0.0, plain(dim));
    }
    if help.overloads > 0 {
        let plural = if help.overloads == 1 { "" } else { "s" };
        job.append(
            &format!("  (+{} overload{plural})", help.overloads),
            0.0,
            plain(dim),
        );
    }
    // The panel is as wide as the signature needs, within the window: it slides left to fit
    // rather than wrapping while there is room, and only wraps once the whole window is too
    // narrow. It sits above the caret line, or below it when there is no room above.
    const MARGIN: Margin = Margin::symmetric(10, 6);
    const EDGE: f32 = 8.0;
    const GAP: f32 = 4.0;
    // Inside the editor's text, never over what is around it (the find bar, the panel below).
    let bounds = state
        .text_visible
        .unwrap_or_else(|| ctx.content_rect())
        .shrink(EDGE);
    // The frame's margin plus its one-point border on each side.
    let chrome = MARGIN.sum() + Vec2::splat(2.0);
    job.wrap.max_width = f32::INFINITY;
    let natural = ctx
        .fonts_mut(|fonts| fonts.layout_job(job.clone()))
        .size()
        .x;
    job.wrap.max_width = natural.min((bounds.width() - chrome.x).max(1.0)) + 0.5;
    let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
    let size = galley.size() + chrome;
    let x = caret_rect
        .left()
        .min(bounds.right() - size.x)
        .max(bounds.left());
    let above = caret_rect.top() - GAP - size.y;
    let below = caret_rect.bottom() + GAP;
    let y = if above >= bounds.top() {
        above
    } else {
        below.min((bounds.bottom() - size.y).max(bounds.top()))
    };
    egui::Area::new(Id::new("script-signature"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .fixed_pos(egui::pos2(x, y))
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(look.surface(SurfaceColor::PopupBackground))
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(6.0)
                .inner_margin(MARGIN)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(galley).selectable(false));
                });
        });
}
