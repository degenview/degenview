//! What the workspace does for Post and Idea cards: asks for the link, places the card, and (only
//! when the user asks) fetches the preview.
//!
//! Arming Post or Idea opens a sheet for the link; nothing is created until a link is accepted,
//! and nothing is contacted until the user presses Preview on a card and, once per service, agrees
//! to it. Opening a link is always a deliberate click. The fetch runs on a worker of its own and
//! its answer is written to the card as one undoable change, so it syncs like any edit.

use super::ChartWorkspace;
use crate::model::image_content::Content;
use crate::model::link_card::{ContentPrefs, Link, LinkCard, LinkKind, LinkMeta};
use crate::model::{DrawingCommand, Geometry, Tool};
use crate::service::link_fetch::{self, FetchError};
use crate::service::work_queue;
use crate::ui::LinkRequest;
use crate::ui::content_sheet::{self, ConsentAction, LinkSheet, LinkSheetAction};
use eframe::egui;
use std::sync::mpsc::{self, Receiver};

/// The sheet that asks for a link: what is typed, and which card it is for.
pub(super) struct Sheet {
    kind: LinkKind,
    text: String,
    /// The card whose link is being replaced: `(chart, drawing)`.
    replace: Option<(u64, u64)>,
}

/// A preview waiting for the user's agreement to contact the service.
pub(super) struct Ask {
    chart_id: u64,
    drawing_id: u64,
    kind: LinkKind,
}

/// What a fetch found, with the card it was for.
pub(super) struct Fetched {
    pub chart_id: u64,
    pub drawing_id: u64,
    pub link: Link,
    pub result: Result<LinkMeta, FetchError>,
}

#[derive(Default)]
pub(super) struct LinkFlow {
    pub(super) sheet: Option<Sheet>,
    pub(super) consent: Option<Ask>,
    pub(super) inbox: Option<Receiver<Fetched>>,
}

impl LinkFlow {
    pub(super) fn open(&self) -> bool {
        self.sheet.is_some() || self.consent.is_some()
    }

    pub(super) fn close_sheet(&mut self) {
        self.sheet = None;
    }

    pub(super) fn start(&mut self, kind: LinkKind) {
        self.sheet = Some(Sheet {
            kind,
            text: String::new(),
            replace: None,
        });
    }
}

/// The tool that places a card of this kind.
pub(super) fn tool_of(kind: LinkKind) -> Tool {
    match kind {
        LinkKind::Post => Tool::Post,
        LinkKind::Idea => Tool::Idea,
    }
}

impl ChartWorkspace {
    /// The sheets, and the answer of a fetch that finished.
    pub(super) fn tick_links(&mut self, ctx: &egui::Context) {
        self.take_fetched();
        if let Some(mut sheet) = self.content.links.sheet.take() {
            let action = content_sheet::show_link(
                ctx,
                LinkSheet {
                    kind: sheet.kind,
                    text: &mut sheet.text,
                    replacing: sheet.replace.is_some(),
                },
            );
            match action {
                LinkSheetAction::None => self.content.links.sheet = Some(sheet),
                LinkSheetAction::Confirm(link) => self.link_chosen(link, sheet.replace),
                LinkSheetAction::Close => {
                    if sheet.replace.is_none() {
                        self.active_tool = Tool::Cursor;
                    }
                }
            }
        }
        if let Some(ask) = self.content.links.consent.take() {
            match content_sheet::show_consent(ctx, ask.kind) {
                ConsentAction::None => self.content.links.consent = Some(ask),
                ConsentAction::Allow => {
                    self.allow_previews(ask.kind);
                    self.start_fetch(ctx, ask.chart_id, ask.drawing_id);
                }
                ConsentAction::Cancel => {}
            }
        }
    }

    /// A link was accepted: a new card waits for a click, or an existing card takes it.
    pub(super) fn link_chosen(&mut self, link: Link, replace: Option<(u64, u64)>) {
        let Some((chart_id, drawing_id)) = replace else {
            let tool = tool_of(link.kind());
            self.content.pending = Some(Content::Link(LinkCard::new(link)));
            self.content.place_now = false;
            self.active_tool = tool;
            self.content.last_tool = tool;
            return;
        };
        let Some(mut drawing) = self.card_drawing(chart_id, drawing_id) else {
            return;
        };
        if let Some(Geometry::Card { card, .. }) = drawing.geometry_mut() {
            // What was fetched for the old link says nothing of the new one.
            card.link = link;
            card.meta = None;
        }
        self.apply_drawing_commands(
            chart_id,
            vec![DrawingCommand::Replace {
                id: drawing_id,
                drawing,
            }],
        );
    }

    /// A link pasted on a chart: a card for it is placed in the middle of the focused chart.
    pub(super) fn place_pasted_link(&mut self, link: Link) {
        let tool = tool_of(link.kind());
        self.content.pending = Some(Content::Link(LinkCard::new(link)));
        self.content.place_now = true;
        self.active_tool = tool;
        self.content.last_tool = tool;
        self.content.links.close_sheet();
    }

    fn card_drawing(&self, chart_id: u64, drawing_id: u64) -> Option<crate::model::Drawing> {
        self.charts
            .iter()
            .find(|chart| chart.id == chart_id)?
            .drawings
            .iter()
            .find(|drawing| {
                drawing.id() == drawing_id
                    && !drawing.locked()
                    && matches!(drawing.geometry(), Some(Geometry::Card { .. }))
            })
            .cloned()
    }

    /// What the user asked of a card: open its link, copy it, choose another, fetch a preview.
    pub(super) fn link_requested(
        &mut self,
        ctx: &egui::Context,
        chart_id: u64,
        drawing_id: u64,
        request: LinkRequest,
    ) {
        let Some(card) = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .and_then(|chart| chart.drawings.iter().find(|d| d.id() == drawing_id))
            .and_then(|drawing| match drawing.geometry() {
                Some(Geometry::Card { card, .. }) => Some(card.clone()),
                _ => None,
            })
        else {
            return;
        };
        match request {
            LinkRequest::Open => ctx.open_url(egui::OpenUrl::new_tab(card.link.url())),
            LinkRequest::Copy => {
                ctx.copy_text(card.link.url());
                self.say("Copied the link.", false);
            }
            LinkRequest::Replace => {
                self.content.links.sheet = Some(Sheet {
                    kind: card.link.kind(),
                    text: card.link.url(),
                    replace: Some((chart_id, drawing_id)),
                });
            }
            LinkRequest::Fetch => {
                if self.previews_allowed(card.link.kind()) {
                    self.start_fetch(ctx, chart_id, drawing_id);
                } else {
                    self.content.links.consent = Some(Ask {
                        chart_id,
                        drawing_id,
                        kind: card.link.kind(),
                    });
                }
            }
        }
    }

    pub(super) fn previews_allowed(&self, kind: LinkKind) -> bool {
        self.store
            .lock()
            .ok()
            .and_then(|store| store.setting::<ContentPrefs>(ContentPrefs::KEY))
            .is_some_and(|prefs| prefs.allows(kind))
    }

    pub(super) fn allow_previews(&mut self, kind: LinkKind) {
        let saved = self
            .store
            .lock()
            .map(|store| {
                let mut prefs = store
                    .setting::<ContentPrefs>(ContentPrefs::KEY)
                    .unwrap_or_default();
                prefs.allow(kind);
                store.set_setting(ContentPrefs::KEY, &prefs).is_ok()
            })
            .unwrap_or(false);
        if !saved {
            self.say("The choice could not be saved.", true);
        }
    }

    /// Asks the service for the preview of one card, on a worker. One at a time.
    fn start_fetch(&mut self, ctx: &egui::Context, chart_id: u64, drawing_id: u64) {
        if self.content.links.inbox.is_some() {
            self.say("A preview is already being fetched.", true);
            return;
        }
        let Some(drawing) = self.card_drawing(chart_id, drawing_id) else {
            return;
        };
        let Some(Geometry::Card { card, .. }) = drawing.geometry() else {
            return;
        };
        let link = card.link.clone();
        let (sender, receiver) = mpsc::channel();
        self.content.links.inbox = Some(receiver);
        self.say("Fetching the preview…", false);
        let ctx = ctx.clone();
        work_queue::LINKS.submit(move || {
            let result = link_fetch::fetch(&link, crate::alert_runtime::now_ms());
            let _ = sender.send(Fetched {
                chart_id,
                drawing_id,
                link,
                result,
            });
            ctx.request_repaint();
        });
    }

    /// The answer of a fetch, written to the card it was for if that card still has that link.
    pub(super) fn take_fetched(&mut self) {
        let Some(inbox) = &self.content.links.inbox else {
            return;
        };
        let fetched = match inbox.try_recv() {
            Ok(fetched) => fetched,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.content.links.inbox = None;
                self.say(FetchError::Network.message(), true);
                return;
            }
        };
        self.content.links.inbox = None;
        let meta = match fetched.result {
            Ok(meta) => meta,
            Err(error) => {
                self.say(error.message(), true);
                return;
            }
        };
        let Some(mut drawing) = self.card_drawing(fetched.chart_id, fetched.drawing_id) else {
            return;
        };
        let Some(Geometry::Card { card, .. }) = drawing.geometry_mut() else {
            return;
        };
        if card.link != fetched.link {
            return;
        }
        card.meta = Some(meta);
        self.apply_drawing_commands(
            fetched.chart_id,
            vec![DrawingCommand::Replace {
                id: fetched.drawing_id,
                drawing,
            }],
        );
        self.say("Fetched the preview.", false);
    }
}
