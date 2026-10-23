//! The watchlist sidebar: drawing it and acting on what it reports.

use super::ChartWorkspace;
use super::alerts::AlertRequest;
use crate::model::InstrumentId;
use crate::model::alert::AlertAsset;
use crate::model::watchlist::WatchlistInstrument;
use crate::ui::watchlist_sidebar::{self, WatchlistAction, WatchlistSidebarInput};
use eframe::egui;
use std::sync::{Arc, mpsc};

impl ChartWorkspace {
    pub(super) fn show_watchlist(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(receiver) = &self.watchlist_file_result {
            match receiver.try_recv() {
                Ok(Err(error)) => {
                    self.watchlist_error = Some(error);
                    self.watchlist_file_result = None;
                }
                Ok(Ok(())) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.watchlist_file_result = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let (actions, instruments) = {
            let watchlist = Arc::clone(&self.watchlist);
            let store = watchlist.lock().expect("watchlist lock poisoned");
            let lists = store.lists();
            let flags = store.flags();
            let instruments: Vec<InstrumentId> = lists
                .iter()
                .flat_map(|list| list.instruments())
                .map(|item| item.instrument.clone())
                .collect();
            let quote = |id: &InstrumentId| self.quote_book.get(id);
            let focused_market = self
                .resolved_focused_id()
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map(|chart| InstrumentId::new(chart.source, &chart.symbol));
            let input = WatchlistSidebarInput {
                lists,
                flags,
                quote: &quote,
                focused_market: focused_market.as_ref(),
                load_failed: store.load_failed(),
                error: self
                    .watchlist_error
                    .as_deref()
                    .or_else(|| store.last_error()),
                import_report: self.import_report.as_ref(),
            };
            let actions = watchlist_sidebar::show(ui, &mut self.watchlist_sidebar, &input);
            (actions, instruments)
        };
        self.quote_coordinator
            .set_demand(self.id, instruments, self.watchlist_visible);
        self.apply_watchlist_actions(actions, ctx);
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
                        store.create_list(&name, with);
                    }
                    WatchlistAction::RenameList { id, name } => store.rename_list(id, &name),
                    WatchlistAction::DuplicateList { id, name } => {
                        store.duplicate_list(id, &name);
                    }
                    WatchlistAction::DeleteList(id) => store.delete_list(id),
                    WatchlistAction::AddToList { list, item } => store.add_to_list(list, item),
                    WatchlistAction::RemoveFromList { list, instrument } => {
                        store.remove_from_list(list, &instrument);
                    }
                    WatchlistAction::RemoveEntry { list, entry } => {
                        store.remove_entry(list, entry);
                    }
                    WatchlistAction::MoveToSection {
                        list,
                        entry,
                        section,
                    } => store.move_instruments(list, &[entry], section),
                    WatchlistAction::Move { list, mv } => {
                        store.move_entries(list, &mv.ids, mv.before);
                    }
                    WatchlistAction::MoveToSectionByDrop { list, mv } => {
                        store.move_instruments(list, &mv.ids, mv.section);
                    }
                    WatchlistAction::AddSection { list, title } => store.add_section(list, &title),
                    WatchlistAction::RenameSection {
                        list,
                        section,
                        title,
                    } => store.rename_section(list, section, &title),
                    WatchlistAction::DeleteSection { list, section } => {
                        store.delete_section(list, section);
                    }
                    WatchlistAction::SetSectionCollapsed {
                        list,
                        section,
                        collapsed,
                    } => store.set_section_collapsed(list, section, collapsed),
                    WatchlistAction::SetSort { list, sort } => store.set_sort(list, sort),
                    WatchlistAction::SetColumn {
                        list,
                        column,
                        visible,
                    } => store.set_column(list, column, visible),
                    WatchlistAction::SetShowsDescription { list, shows } => {
                        store.set_shows_description(list, shows);
                    }
                    WatchlistAction::SetFlag { instrument, flag } => {
                        store.set_flag(&instrument, flag);
                    }
                    WatchlistAction::Import { list, text } => {
                        match store.import_into(list, &text) {
                            Some(report) => {
                                self.import_report = Some(report);
                                self.watchlist_error = None;
                            }
                            None => {
                                self.watchlist_error = Some("Import failed".to_owned());
                            }
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
                                    &format!("{name}.txt"),
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

    fn open_watchlist_instrument(&mut self, item: &WatchlistInstrument, ctx: &egui::Context) {
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
            return;
        }
        if let Some(focused) = self.resolved_focused_id() {
            self.switch_chart(focused, item, ctx);
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
