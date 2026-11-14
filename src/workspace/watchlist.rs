//! The watchlist sidebar: drawing it and acting on what it reports.

use super::ChartWorkspace;
use super::alerts::AlertRequest;
use crate::lock::lock;
use crate::model::InstrumentId;
use crate::model::alert::AlertAsset;
use crate::model::watchlist::text::ImportReport;
use crate::model::watchlist::{EntryId, WatchlistError, WatchlistInstrument, WatchlistNotice};
use crate::service::system::file_dialog;
use crate::service::work_queue::{self, Pending};
use crate::ui::watchlist::sidebar::{
    self, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState,
};
use eframe::egui;
use std::sync::Arc;

/// Shows why a change was refused. A failed save already shows the store's own message.
fn report_watchlist(slot: &mut Option<WatchlistNotice>, result: Result<(), WatchlistError>) {
    match result {
        Ok(()) | Err(WatchlistError::SaveFailed) => {}
        Err(error) => *slot = Some(error.notice()),
    }
}

/// Asks for a file of watchlist symbols and reads it, on the workers that wait on the user. The
/// answer is `None` when the user cancels.
fn read_import_file(ctx: &egui::Context) -> Pending<Result<Option<String>, String>> {
    work_queue::SNAPSHOTS.start(Some(ctx), || {
        file_dialog::choose_csv("Choose watchlist symbols").and_then(|path| {
            path.map(|path| {
                file_dialog::read_text_up_to(&path, crate::model::watchlist::text::MAX_IMPORT_BYTES)
            })
            .transpose()
        })
    })
}

/// Asks where to save the watchlist `name` and writes `text` there, on the workers that wait on
/// the user.
fn export_to_file(ctx: &egui::Context, name: &str, text: String) -> Pending<Result<(), String>> {
    let file_name = crate::model::watchlist::text::export_file_name(name);
    work_queue::SNAPSHOTS.start(Some(ctx), move || {
        file_dialog::choose_save_path("Export watchlist", &file_name)
            .and_then(|path| {
                path.map(|path| file_dialog::write_text(&path, &text))
                    .transpose()
            })
            .map(|_| ())
    })
}

/// The watchlist sidebar's side of a tab: whether it shows, its own view state, what it last had
/// to say, and the requests it hands to the rest of the tab (a list for the symbol picker to add
/// to, a market to open in a new tab).
#[derive(Default)]
pub(super) struct WatchlistPane {
    /// Saved with the tab and its layouts.
    visible: bool,
    sidebar: WatchlistSidebarState,
    import_report: Option<ImportReport>,
    /// Why the last change was refused, until dismissed.
    notice: Option<WatchlistNotice>,
    /// The export being written.
    file_result: Option<Pending<Result<(), String>>>,
    /// The list a list's plus opened the symbol picker for; a pick is added to it.
    picker_target: Option<EntryId>,
    /// A row asked to open in a new tab; the app opens it after the frame.
    open_in_new_tab: Option<WatchlistInstrument>,
}

impl WatchlistPane {
    pub(super) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub(super) fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Shows why a change was refused (a failed save shows the store's own message).
    pub(super) fn report(&mut self, result: Result<(), WatchlistError>) {
        report_watchlist(&mut self.notice, result);
    }

    /// The list the open symbol picker adds to, if it was opened for one.
    pub(super) fn picker_target(&self) -> Option<EntryId> {
        self.picker_target
    }

    /// The picker closed: a list is only its target while it is open.
    pub(super) fn clear_picker_target(&mut self) {
        self.picker_target = None;
    }

    #[cfg(test)]
    pub(super) fn set_picker_target(&mut self, list: Option<EntryId>) {
        self.picker_target = list;
    }

    /// The market a row asked to open in a new tab, once.
    pub(super) fn take_new_tab_request(&mut self) -> Option<WatchlistInstrument> {
        self.open_in_new_tab.take()
    }
}

impl ChartWorkspace {
    /// The sidebar at the right while it is shown; hidden, its quote demand stops.
    pub(super) fn show_watchlist_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.watchlist_pane.visible {
            self.suspend_watchlist_demand();
            return;
        }
        let minimum_width = self
            .watchlist
            .lock()
            .ok()
            .map(|store| self.watchlist_pane.sidebar.minimum_width(store.lists()))
            .unwrap_or(286.0);
        egui::Panel::right("watchlist")
            .default_size(minimum_width.max(270.0))
            .min_size(minimum_width)
            .max_size(640.0)
            .frame(
                egui::Frame::new()
                    .fill(crate::ui::theme::chrome())
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ui, |ui| self.show_watchlist(ui, ctx));
    }

    fn show_watchlist(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(export) = &mut self.watchlist_pane.file_result {
            if let Some(Err(error)) = export.poll() {
                self.watchlist_pane.notice = Some(WatchlistNotice::new("File Error", error));
            }
            if !export.is_pending() {
                self.watchlist_pane.file_result = None;
            }
        }
        let actions = {
            let watchlist = Arc::clone(&self.watchlist);
            let store = lock(&watchlist);
            let lists = store.lists();
            let flags = store.flags();
            let quote = |id: &InstrumentId| self.quote_book.get(id);
            let focused_market = self
                .resolved_focused_id()
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map(|chart| InstrumentId::new(chart.source, &chart.symbol));
            let store_notice = store
                .last_error()
                .map(|message| WatchlistNotice::new("Couldn\u{2019}t Save", message));
            let input = WatchlistSidebarInput {
                lists,
                flags,
                quote: &quote,
                focused_market: focused_market.as_ref(),
                load_failed: store.load_failed(),
                error: self
                    .watchlist_pane
                    .notice
                    .as_ref()
                    .or(store_notice.as_ref()),
                import_report: self.watchlist_pane.import_report.as_ref(),
            };
            sidebar::show(ui, &mut self.watchlist_pane.sidebar, &input)
        };
        // Only the rows on screen ask for prices, not every list in the library.
        self.quote_coordinator.set_sidebar_demand(
            self.id,
            self.watchlist_pane.sidebar.quote_demand().to_vec(),
            true,
        );
        self.apply_watchlist_actions(actions, ctx);
    }

    /// Closing the sidebar stops its quote demand at once; a background tab or a covered
    /// window never reaches here and is covered by the demand's lease.
    pub(super) fn suspend_watchlist_demand(&self) {
        self.quote_coordinator
            .set_sidebar_demand(self.id, Vec::new(), false);
    }

    fn apply_watchlist_actions(&mut self, actions: Vec<WatchlistAction>, ctx: &egui::Context) {
        let mut opens: Vec<(bool, WatchlistInstrument)> = Vec::new();
        {
            let mut store = lock(&self.watchlist);
            for action in actions {
                match action {
                    WatchlistAction::Open(item) => opens.push((true, item)),
                    WatchlistAction::AddChart(item) => opens.push((false, item)),
                    WatchlistAction::OpenInNewTab(item) => {
                        // Hand off after the store lock is released.
                        self.watchlist_pane.open_in_new_tab = Some(item);
                    }
                    WatchlistAction::AddSymbol { list } => {
                        if let Some(list_id) = list
                            && let Some(name) = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list_id)
                                .map(|candidate| candidate.name.clone())
                        {
                            self.watchlist_pane.picker_target = Some(list_id);
                            self.symbol_picker.open_for_watchlist(&name);
                        }
                    }
                    WatchlistAction::CreateAlert(item) => {
                        self.alerts.request(AlertRequest::market(AlertAsset::new(
                            item.instrument.clone(),
                            crate::ui::watchlist::rows::icon_base_symbol(&item),
                            item.name.clone(),
                        )));
                    }
                    WatchlistAction::CreateList { name, with } => {
                        let result = store.create_list(&name, with).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::RenameList { id, name } => {
                        let result = store.rename_list(id, &name).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::DuplicateList { id, name } => {
                        let result = store.duplicate_list(id, &name).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::DeleteList(id) => {
                        let result = store.delete_list(id).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::AddToList { list, item } => {
                        let result = store.add_to_list(list, item).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::RemoveFromList { list, instrument } => {
                        let result = store.remove_from_list(list, &instrument).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::RemoveEntry { list, entry } => {
                        let result = store.remove_entry(list, entry).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::MoveToSection {
                        list,
                        entry,
                        section,
                    } => {
                        let result = store.move_instruments(list, &[entry], section).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::Move { list, mv } => {
                        let result = store.move_entries(list, &mv.ids, mv.before).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::MoveToSectionByDrop { list, mv } => {
                        let result = store
                            .move_instruments(list, &mv.ids, mv.section)
                            .map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::AddSection { list, title } => {
                        let result = store.add_section(list, &title).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::RenameSection {
                        list,
                        section,
                        title,
                    } => {
                        let result = store.rename_section(list, section, &title).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::DeleteSection { list, section } => {
                        let result = store.delete_section(list, section).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::SetSectionCollapsed {
                        list,
                        section,
                        collapsed,
                    } => {
                        let result = store
                            .set_section_collapsed(list, section, collapsed)
                            .map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::SetSort { list, sort } => {
                        let result = store.set_sort(list, sort).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::SetColumn {
                        list,
                        column,
                        visible,
                    } => {
                        let result = store.set_column(list, column, visible).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::SetShowsDescription { list, shows } => {
                        let result = store.set_shows_description(list, shows).map(|_| ());
                        report_watchlist(&mut self.watchlist_pane.notice, result);
                    }
                    WatchlistAction::SetFlag { instrument, flag } => {
                        store.set_flag(&instrument, flag);
                    }
                    WatchlistAction::Import { list, text } => {
                        match store.import_into(list, &text) {
                            Ok(report) => {
                                self.watchlist_pane.import_report = Some(report);
                                self.watchlist_pane.notice = None;
                            }
                            Err(error) => {
                                report_watchlist(&mut self.watchlist_pane.notice, Err(error))
                            }
                        }
                    }
                    WatchlistAction::BeginImport => self.watchlist_pane.import_report = None,
                    WatchlistAction::ChooseImportFile => {
                        self.watchlist_pane
                            .sidebar
                            .read_import_file(read_import_file(ctx));
                    }
                    WatchlistAction::Export { list } => {
                        if let Some(text) = store.export(list) {
                            let name = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list)
                                .map(|candidate| candidate.name.clone())
                                .unwrap_or_else(|| "Watchlist".to_owned());
                            self.watchlist_pane.file_result =
                                Some(export_to_file(ctx, &name, text));
                        }
                    }
                    WatchlistAction::CopyText(text) => ctx.copy_text(text),
                    WatchlistAction::DismissError => {
                        self.watchlist_pane.notice = None;
                        store.clear_error();
                    }
                }
            }
        }
        for (open, item) in opens {
            if open {
                self.open_watchlist_instrument(&item, ctx);
            } else {
                self.add_chart_from_instrument(&item, ctx);
            }
        }
    }

    pub(super) fn open_watchlist_instrument(
        &mut self,
        item: &WatchlistInstrument,
        ctx: &egui::Context,
    ) {
        // A card is not a market: show the one already here, else add it beside the others
        // (never replace the focused chart's market with it).
        if let Some(card) = item.card {
            self.add_card(card);
            return;
        }
        let instrument = &item.instrument;
        if let Some(existing) = self.charts.iter().position(|chart| {
            InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument)
        }) {
            let chart_id = self.charts[existing].id;
            self.charts[existing].apply_watchlist_metadata(item);
            self.focus_chart(chart_id);
            // The focused market is the tab's leader: this is a change of it like any other.
            self.announce_market(item);
            return;
        }
        if let Some(focused) = self.resolved_focused_id() {
            self.replace_chart_market(focused, item, ctx);
            self.announce_market(item);
        } else {
            self.add_chart_from_instrument(item, ctx);
        }
    }

    pub(super) fn add_chart_from_instrument(
        &mut self,
        item: &WatchlistInstrument,
        ctx: &egui::Context,
    ) {
        if let Some(card) = item.card {
            self.add_card(card);
            return;
        }
        let outcome = self.add_chart_for(&item.instrument, ctx);
        if let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == outcome.chart_id)
        {
            chart.apply_watchlist_metadata(item);
        }
    }
}
