//! The watchlist sidebar: drawing it and acting on what it reports.

use super::ChartWorkspace;
use super::alerts::AlertRequest;
use crate::model::InstrumentId;
use crate::model::alert::AlertAsset;
use crate::model::watchlist::{WatchlistError, WatchlistInstrument, WatchlistNotice};
use crate::ui::watchlist_sidebar::{self, WatchlistAction, WatchlistSidebarInput};
use eframe::egui;
use std::sync::{Arc, mpsc};

/// Shows why a change was refused. A failed save already shows the store's own message.
pub(super) fn report_watchlist(
    slot: &mut Option<WatchlistNotice>,
    result: Result<(), WatchlistError>,
) {
    match result {
        Ok(()) | Err(WatchlistError::SaveFailed) => {}
        Err(error) => *slot = Some(error.notice()),
    }
}

impl ChartWorkspace {
    pub(super) fn show_watchlist(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(receiver) = &self.watchlist_file_result {
            match receiver.try_recv() {
                Ok(Err(error)) => {
                    self.watchlist_error = Some(WatchlistNotice::new("File Error", error));
                    self.watchlist_file_result = None;
                }
                Ok(Ok(())) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.watchlist_file_result = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let actions = {
            let watchlist = Arc::clone(&self.watchlist);
            let store = watchlist.lock().expect("watchlist lock poisoned");
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
                error: self.watchlist_error.as_ref().or(store_notice.as_ref()),
                import_report: self.import_report.as_ref(),
            };
            watchlist_sidebar::show(ui, &mut self.watchlist_sidebar, &input)
        };
        // Only the rows on screen ask for prices, not every list in the library.
        self.quote_coordinator.set_sidebar_demand(
            self.id,
            self.watchlist_sidebar.quote_demand().to_vec(),
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
            let mut store = self.watchlist.lock().expect("watchlist lock poisoned");
            for action in actions {
                match action {
                    WatchlistAction::Open(item) => opens.push((true, item)),
                    WatchlistAction::AddChart(item) => opens.push((false, item)),
                    WatchlistAction::OpenInNewTab(item) => {
                        // Hand off after the store lock is released.
                        self.open_new_tab_request = Some(item);
                    }
                    WatchlistAction::AddSymbol { list } => {
                        if let Some(list_id) = list
                            && let Some(name) = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list_id)
                                .map(|candidate| candidate.name.clone())
                        {
                            self.add_to_watchlist = Some(list_id);
                            self.add_chart.open_for_watchlist(&name);
                        }
                    }
                    WatchlistAction::CreateAlert(item) => {
                        self.new_alert_request = Some(AlertRequest::market(AlertAsset::new(
                            item.instrument.clone(),
                            crate::ui::watchlist_rows::icon_base_symbol(&item),
                            item.name.clone(),
                        )));
                    }
                    WatchlistAction::CreateList { name, with } => {
                        let result = store.create_list(&name, with).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::RenameList { id, name } => {
                        let result = store.rename_list(id, &name).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::DuplicateList { id, name } => {
                        let result = store.duplicate_list(id, &name).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::DeleteList(id) => {
                        let result = store.delete_list(id).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::AddToList { list, item } => {
                        let result = store.add_to_list(list, item).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::RemoveFromList { list, instrument } => {
                        let result = store.remove_from_list(list, &instrument).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::RemoveEntry { list, entry } => {
                        let result = store.remove_entry(list, entry).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::MoveToSection {
                        list,
                        entry,
                        section,
                    } => {
                        let result = store.move_instruments(list, &[entry], section).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::Move { list, mv } => {
                        let result = store.move_entries(list, &mv.ids, mv.before).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::MoveToSectionByDrop { list, mv } => {
                        let result = store
                            .move_instruments(list, &mv.ids, mv.section)
                            .map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::AddSection { list, title } => {
                        let result = store.add_section(list, &title).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::RenameSection {
                        list,
                        section,
                        title,
                    } => {
                        let result = store.rename_section(list, section, &title).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::DeleteSection { list, section } => {
                        let result = store.delete_section(list, section).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::SetSectionCollapsed {
                        list,
                        section,
                        collapsed,
                    } => {
                        let result = store
                            .set_section_collapsed(list, section, collapsed)
                            .map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::SetSort { list, sort } => {
                        let result = store.set_sort(list, sort).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::SetColumn {
                        list,
                        column,
                        visible,
                    } => {
                        let result = store.set_column(list, column, visible).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::SetShowsDescription { list, shows } => {
                        let result = store.set_shows_description(list, shows).map(|_| ());
                        report_watchlist(&mut self.watchlist_error, result);
                    }
                    WatchlistAction::SetFlag { instrument, flag } => {
                        store.set_flag(&instrument, flag);
                    }
                    WatchlistAction::Import { list, text } => {
                        match store.import_into(list, &text) {
                            Ok(report) => {
                                self.import_report = Some(report);
                                self.watchlist_error = None;
                            }
                            Err(error) => report_watchlist(&mut self.watchlist_error, Err(error)),
                        }
                    }
                    WatchlistAction::BeginImport => self.import_report = None,
                    WatchlistAction::Export { list } => {
                        if let Some(text) = store.export(list) {
                            let name = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list)
                                .map(|candidate| candidate.name.clone())
                                .unwrap_or_else(|| "Watchlist".to_owned());
                            let (sender, receiver) = mpsc::channel();
                            self.watchlist_file_result = Some(receiver);
                            std::thread::spawn(move || {
                                let result = crate::service::file_dialog::choose_save_path(
                                    "Export watchlist",
                                    &crate::model::watchlist_text::export_file_name(&name),
                                )
                                .and_then(|path| {
                                    path.map(|path| {
                                        crate::service::file_dialog::write_text(&path, &text)
                                    })
                                    .transpose()
                                })
                                .map(|_| ());
                                let _ = sender.send(result);
                            });
                        }
                    }
                    WatchlistAction::CopyText(text) => ctx.copy_text(text),
                    WatchlistAction::DismissError => {
                        self.watchlist_error = None;
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
            self.switch_chart(focused, item, ctx);
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
