//! Renaming a tab in place: the field over the tab and what has been typed.

use super::{TabBar, TabBarEvent};
use crate::model::tab::title::TabChart;
use crate::model::tab::windows::TabId;
use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{self, Id, Pos2, Rect, Ui, vec2};

impl TabBar<'_> {
    fn rename_id(&self) -> Id {
        self.id("rename")
    }

    pub(super) fn begin_rename(
        &self,
        ctx: &egui::Context,
        rect: Rect,
        tab: TabId,
        chart: &TabChart,
    ) {
        let text = chart
            .custom_title
            .clone()
            .unwrap_or_else(|| chart.title.clone());
        let state = Rename {
            tab,
            pos: rect.left_bottom() + vec2(0.0, 4.0),
            text,
            focused: false,
        };
        ctx.data_mut(|data| data.insert_temp(self.rename_id(), state));
    }

    /// The small field under a tab that takes a title for it: Enter keeps it (empty clears it),
    /// Escape or a click elsewhere leaves the tab as it was.
    pub(super) fn show_rename(&self, ui: &mut Ui, events: &mut Vec<TabBarEvent>) {
        let ctx = ui.ctx().clone();
        let Some(mut state) = ctx.data(|data| data.get_temp::<Rename>(self.rename_id())) else {
            return;
        };
        if !self.tabs.iter().any(|tab| tab.id == state.tab) {
            ctx.data_mut(|data| data.remove::<Rename>(self.rename_id()));
            return;
        }
        let mut done = None;
        egui::Area::new(self.id("rename-area"))
            .order(egui::Order::Foreground)
            .fixed_pos(state.pos)
            .show(&ctx, |ui| {
                kit::menu_frame().show(ui, |ui| {
                    ui.set_width(240.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut state.text)
                            .hint_text("Tab title")
                            .char_limit(crate::model::tab::title::MAX_CUSTOM_TITLE_CHARS)
                            .desired_width(f32::INFINITY),
                    );
                    if !state.focused {
                        edit.request_focus();
                        state.focused = true;
                    }
                    ui.add_space(4.0);
                    ui.label(kit::text(
                        "Return to save, Esc to cancel. Empty restores the default.",
                        kit::size::CAPTION,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    if edit.lost_focus() {
                        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                        done = Some(enter);
                    }
                });
            });
        match done {
            Some(true) => {
                events.push(TabBarEvent::SetCustomTitle {
                    tab: state.tab,
                    title: Some(state.text.clone()),
                });
                ctx.data_mut(|data| data.remove::<Rename>(self.rename_id()));
            }
            Some(false) => ctx.data_mut(|data| data.remove::<Rename>(self.rename_id())),
            None => ctx.data_mut(|data| {
                data.insert_temp(self.rename_id(), state);
            }),
        }
    }
}

/// An open rename: the tab, where its field sits and what has been typed.
#[derive(Clone)]
struct Rename {
    tab: TabId,
    pos: Pos2,
    text: String,
    focused: bool,
}
