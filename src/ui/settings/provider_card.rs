//! One row of Settings ▸ Snapshots: a snapshot provider, drawn from its descriptor.
//!
//! The row is a card whose header stays when it is folded: the provider's mark, its name, how it
//! is doing, a switch that turns it on (one provider at a time) and a chevron. Unfolded, it shows
//! the setup steps, the links, what the user should know before an image goes there, and a widget
//! per field of the descriptor. Nothing here knows which provider it is drawing, so a new provider
//! needs no code on this page.

use crate::model::chart::snapshot::{FieldValues, SnapshotPrefs};
use crate::service::keychain_snapshot_secrets::{SecretState, SnapshotSecrets};
use crate::service::snapshot_providers::{
    Destination, FieldKind, FieldSpec, Glyph, Logo, ProviderDescriptor, Readiness, SecretPresence,
    SnapshotProvider,
};
use crate::service::snapshot_upload::{self, UreqTransport};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::Tone;
use crate::ui::paper::form;
use crate::ui::settings::api_keys::{Autosave, StatusMessage, destructive_button, field};
use crate::ui::settings::brokers::{fitted, gray_link, muted, step_row, titled_group};
use crate::ui::settings::snapshots::SnapshotsAction;
use crate::ui::settings::window::{divider, status_message};
use crate::ui::{kit, theme};
use eframe::egui::{self, vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// The height of a card's header row.
const HEADER_HEIGHT: f32 = 36.0;
/// Under this much room beside the title, the header shows the state as a dot instead of a badge.
const COMPACT_ROOM: f32 = 250.0;

/// What a row needs to know besides its own form.
pub struct CardInput<'a> {
    pub prefs: &'a SnapshotPrefs,
    pub secrets: &'a Arc<SnapshotSecrets>,
}

/// What the user is doing in one row.
#[derive(Default)]
pub struct ProviderForm {
    /// The fields as typed: text and choices as they are saved, and secrets until they are.
    values: BTreeMap<String, String>,
    /// The fields the user has changed and that are not saved yet. Only these are ever sent: a
    /// field left alone must not overwrite what another Mac has since synced in.
    dirty: BTreeSet<String>,
    /// The saved settings the fields were last filled from.
    loaded_from: Option<FieldValues>,
    autosave: Autosave,
    test: Option<Receiver<Result<(), String>>>,
    /// What the last test said.
    test_status: Option<StatusMessage>,
    /// What the last save said.
    pub status: Option<StatusMessage>,
}

impl ProviderForm {
    /// Puts what is saved in the fields: when the row is first drawn, and again when what is saved
    /// changes under it (iCloud Sync) while the user has nothing of their own half typed.
    fn sync_with(&mut self, descriptor: &ProviderDescriptor, settings: &FieldValues) {
        let unchanged = self.loaded_from.as_ref() == Some(settings);
        let editing = !self.dirty.is_empty() || self.autosave.is_pending();
        if unchanged || (self.loaded_from.is_some() && editing) {
            if !unchanged {
                // What the user is typing stays; it is compared with the new saved value.
                self.loaded_from = Some(settings.clone());
            }
            return;
        }
        self.loaded_from = Some(settings.clone());
        for field in descriptor.fields.iter().filter(|field| !field.is_secret()) {
            let value = match field.kind {
                FieldKind::Choice(_) => field
                    .choice(settings.get(field.key).map(String::as_str))
                    .map_or("", |choice| choice.value),
                _ => settings.get(field.key).map_or("", String::as_str),
            };
            self.values.insert(field.key.to_owned(), value.to_owned());
        }
    }

    fn typed(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }

    /// What is wrong with a typed value, if anything. An empty one is never wrong: it clears.
    fn problem(&self, field: &FieldSpec) -> Option<&'static str> {
        let text = self.typed(field.key).trim();
        match field.kind {
            FieldKind::Choice(_) => None,
            _ => (!text.is_empty()).then(|| (field.validate)(text)).flatten(),
        }
    }

    fn has_problem(&self, descriptor: &ProviderDescriptor) -> bool {
        descriptor
            .fields
            .iter()
            .any(|field| self.problem(field).is_some())
    }

    /// The non-secret fields the user changed that are right and differ from what is saved. One
    /// that is wrong stays out (its problem is under it) without keeping the others from being
    /// saved.
    fn changed_fields(
        &self,
        descriptor: &ProviderDescriptor,
        settings: &FieldValues,
    ) -> BTreeMap<String, String> {
        let mut changed = BTreeMap::new();
        for field in descriptor
            .fields
            .iter()
            .filter(|field| !field.is_secret() && self.dirty.contains(field.key))
            .filter(|field| self.problem(field).is_none())
        {
            let typed = self.typed(field.key).trim();
            let saved = settings.get(field.key).map_or("", String::as_str);
            let unchanged = match field.kind {
                // Never picked and nothing saved: the default is already in force.
                FieldKind::Choice(_) => {
                    typed == saved || (saved.is_empty() && typed == field.default)
                }
                _ => typed == saved,
            };
            if !unchanged {
                changed.insert(field.key.to_owned(), typed.to_owned());
            }
        }
        changed
    }

    /// The secrets typed and not yet saved (a wrong one is left in its field with its problem).
    fn typed_secrets(&self, descriptor: &ProviderDescriptor) -> BTreeMap<String, String> {
        descriptor
            .secret_fields()
            .filter(|field| self.problem(field).is_none())
            .filter_map(|field| {
                let typed = self.typed(field.key).trim();
                (!typed.is_empty()).then(|| (field.key.to_owned(), typed.to_owned()))
            })
            .collect()
    }

    /// A save went through: what was typed in a secret field is the secure storage's now.
    pub fn saved(&mut self, descriptor: &ProviderDescriptor) {
        for field in descriptor.secret_fields() {
            self.values.remove(field.key);
        }
    }

    /// The settings as typed, over what is saved.
    fn settings_as_typed(
        &self,
        descriptor: &ProviderDescriptor,
        saved: &FieldValues,
    ) -> FieldValues {
        let mut settings = saved.clone();
        for field in descriptor.fields.iter().filter(|field| !field.is_secret()) {
            if let Some(typed) = self.values.get(field.key) {
                let typed = typed.trim();
                if typed.is_empty() {
                    settings.remove(field.key);
                } else {
                    settings.insert(field.key.to_owned(), typed.to_owned());
                }
            }
        }
        settings
    }

    /// Whether the provider could upload with what is saved and typed.
    fn ready_as_typed(
        &self,
        provider: &dyn SnapshotProvider,
        saved: &FieldValues,
        mut present: SecretPresence,
    ) -> bool {
        let descriptor = provider.descriptor();
        for key in self.typed_secrets(descriptor).keys() {
            present.insert(key);
        }
        !self.has_problem(descriptor)
            && provider.readiness(&self.settings_as_typed(descriptor, saved), &present)
                == Readiness::Ready
    }

    /// Uploads a one-pixel image with what is typed (else saved), off the UI thread.
    fn start_test(
        &mut self,
        provider: &'static dyn SnapshotProvider,
        saved: &FieldValues,
        secrets: &Arc<SnapshotSecrets>,
    ) {
        if self.test.is_some() {
            return;
        }
        let descriptor = provider.descriptor();
        let destination = Destination::new(provider, self.settings_as_typed(descriptor, saved));
        let typed = self.typed_secrets(descriptor);
        let store = Arc::clone(secrets);
        let (sender, receiver) = std::sync::mpsc::channel();
        self.test_status = None;
        self.test = Some(receiver);
        std::thread::spawn(move || {
            // Read here, on the worker: the secure storage may prompt.
            let mut secrets = store.read_all(provider);
            for (key, value) in typed {
                secrets.insert(&key, value);
            }
            let label = destination.label();
            let result = snapshot_upload::test(&UreqTransport::default(), &destination, &secrets)
                .map_err(|error| error.message(&label));
            let _ = sender.send(result);
        });
    }

    fn poll_test(&mut self, label: &str) {
        let Some(receiver) = &self.test else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The test stopped unexpectedly.".to_owned()),
        };
        self.test = None;
        self.test_status = Some(match outcome {
            Ok(()) => StatusMessage::ok(format!("{label} accepted a test image.")),
            Err(message) => StatusMessage::error(message),
        });
    }

    pub fn is_testing(&self) -> bool {
        self.test.is_some()
    }
}

/// How a provider is doing, for the header. `checking`: the secure storage has not said yet
/// whether its key is saved, so "setup needed" would only be a guess.
fn state_badge(
    enabled: bool,
    readiness: &Readiness,
    checking: bool,
) -> Option<(&'static str, Tone)> {
    match (enabled, readiness) {
        (false, _) => None,
        (true, _) if checking => Some(("Checking", Tone::Neutral)),
        (true, Readiness::Ready) => Some(("Active", Tone::Good)),
        (true, Readiness::Needs(_)) => Some(("Setup needed", Tone::Warning)),
    }
}

/// One provider: a header that folds the rest. A provider that is on but not ready starts
/// unfolded, the others folded, until the user says otherwise.
pub fn provider_card(
    ui: &mut egui::Ui,
    provider: &'static dyn SnapshotProvider,
    input: &CardInput,
    form: &mut ProviderForm,
    open: &mut Option<bool>,
    actions: &mut Vec<SnapshotsAction>,
) {
    let descriptor = provider.descriptor();
    let settings = input.prefs.settings_of(descriptor.id);
    form.sync_with(descriptor, &settings);
    form.poll_test(&provider.label(&settings));
    if form.is_testing() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    let enabled = input.prefs.provider.as_deref() == Some(descriptor.id);
    let present = input.secrets.presence(provider);
    let readiness = provider.readiness(&settings, &present);
    let checking = present.is_pending();
    let needs_setup = enabled && !checking && matches!(readiness, Readiness::Needs(_));
    let is_open = open.unwrap_or(needs_setup);

    let width = ui.available_width();
    let mut prepared = kit::card_frame(false).begin(ui);
    let inner = &mut prepared.content_ui;
    inner.set_width(width - 28.0 - 2.0);
    let (head, toggled) = header(
        inner,
        provider,
        &settings,
        (enabled, &readiness, checking),
        is_open,
    );
    if toggled {
        // Turning one on turns the other off. It opens, so what it needs is in front of the user.
        actions.push(SnapshotsAction::SetProvider(
            (!enabled).then_some(descriptor.id),
        ));
        if !enabled {
            *open = Some(true);
        }
    } else if head.clicked() {
        *open = Some(!is_open);
    }
    if is_open {
        inner.add_space(12.0);
        divider(inner);
        inner.add_space(14.0);
        inner.spacing_mut().item_spacing.y = 10.0;
        body(inner, provider, input, form, &settings, &present, actions);
    }
    prepared.frame = kit::card_frame(head.hovered());
    prepared.end(ui);

    // Saved as it is typed: text when the typing pauses, a secret when its field is left, a
    // choice at once.
    if form.autosave.due(ui.ctx()) {
        let fields = form.changed_fields(descriptor, &settings);
        let secrets = form.typed_secrets(descriptor);
        if fields.is_empty() && secrets.is_empty() {
            // Nothing that is right to save (what was typed is wrong, or is what is saved).
            form.autosave.cancel();
        } else {
            for key in fields.keys() {
                form.dirty.remove(key);
            }
            actions.push(SnapshotsAction::Save {
                provider: descriptor.id,
                fields,
                secrets,
            });
        }
    }
}

/// Everything under the header.
fn body(
    ui: &mut egui::Ui,
    provider: &'static dyn SnapshotProvider,
    input: &CardInput,
    form: &mut ProviderForm,
    settings: &FieldValues,
    present: &SecretPresence,
    actions: &mut Vec<SnapshotsAction>,
) {
    let descriptor = provider.descriptor();
    if !descriptor.setup_steps.is_empty() {
        titled_group(ui, "Set up", |ui| {
            for (index, step) in descriptor.setup_steps.iter().enumerate() {
                step_row(ui, index + 1, step);
            }
        });
    }
    muted(ui, descriptor.notice);
    for link in descriptor.links {
        gray_link(ui, link.label, link.url);
    }
    if !descriptor.fields.is_empty() {
        ui.add_space(4.0);
    }
    for spec in descriptor.fields {
        // Scoped: a field squeezes the spacing it is drawn in.
        ui.scope(|ui| match spec.kind {
            FieldKind::Text => {
                let response = field(
                    ui,
                    spec.title,
                    Some(spec.hint),
                    spec.placeholder,
                    form.values.entry(spec.key.to_owned()).or_default(),
                    false,
                );
                if response.changed() {
                    form.dirty.insert(spec.key.to_owned());
                }
                form.autosave.watch(&response, false);
                if let Some(problem) = form.problem(spec) {
                    problem_line(ui, problem);
                }
            }
            FieldKind::Secret => {
                let state = input.secrets.state(descriptor.id, spec);
                let hint = secret_hint(spec, state);
                let response = field(
                    ui,
                    spec.title,
                    Some(&hint),
                    spec.placeholder,
                    form.values.entry(spec.key.to_owned()).or_default(),
                    true,
                );
                form.autosave.watch(&response, true);
                if let Some(problem) = form.problem(spec) {
                    problem_line(ui, problem);
                }
            }
            FieldKind::Choice(choices) => {
                let value = form.values.entry(spec.key.to_owned()).or_default();
                if choice_field(ui, spec, choices, value) {
                    form.dirty.insert(spec.key.to_owned());
                    form.autosave.touch();
                }
            }
        });
    }
    let removable = descriptor
        .secret_fields()
        .any(|spec| input.secrets.state(descriptor.id, spec) == SecretState::Saved);
    if descriptor.testable || removable {
        ui.add_space(4.0);
        actions_row(ui, provider, input, form, settings, present, actions);
    }
    for status in [&form.test_status, &form.status].into_iter().flatten() {
        status_message(ui, &status.text, status.is_error);
    }
}

/// Test, and Remove for each secret that is saved, in the card of the thing they act on.
fn actions_row(
    ui: &mut egui::Ui,
    provider: &'static dyn SnapshotProvider,
    input: &CardInput,
    form: &mut ProviderForm,
    settings: &FieldValues,
    present: &SecretPresence,
    actions: &mut Vec<SnapshotsAction>,
) {
    let descriptor = provider.descriptor();
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.spacing_mut().item_spacing.x = 10.0;
        if descriptor.testable {
            let testing = form.is_testing();
            let can_test = !testing && form.ready_as_typed(provider, settings, present.clone());
            if kit::sheet_button(ui, "Test", false, can_test)
                .on_hover_text("Sends a one-pixel test image to check the settings.")
                .clicked()
            {
                form.start_test(provider, settings, input.secrets);
            }
            if testing {
                ui.add(
                    egui::Spinner::new()
                        .size(14.0)
                        .color(theme::text_secondary()),
                );
                muted(ui, "Checking…");
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for spec in descriptor.secret_fields() {
                if input.secrets.state(descriptor.id, spec) == SecretState::Saved
                    && destructive_button(ui, spec.remove_label).clicked()
                {
                    actions.push(SnapshotsAction::RemoveSecret {
                        provider: descriptor.id,
                        field: spec.key,
                    });
                }
            }
        });
    });
}

/// The line under a secret's field: what it is, or that one is already there.
fn secret_hint(spec: &FieldSpec, state: SecretState) -> String {
    match (state, spec.env) {
        (SecretState::Saved, _) => {
            "Saved in your system's secure storage. Type a new one to replace it.".to_owned()
        }
        (SecretState::FromEnvironment, Some(variable)) => {
            format!("Taken from {variable}. Type one to use it instead.")
        }
        _ => spec.hint.to_owned(),
    }
}

/// A pull-down of fixed choices under its title. Returns whether one was picked.
fn choice_field(
    ui: &mut egui::Ui,
    spec: &FieldSpec,
    choices: &[crate::service::snapshot_providers::Choice],
    value: &mut String,
) -> bool {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(kit::text(
        spec.title,
        kit::size::SUBHEADLINE + 1.0,
        kit::MEDIUM,
        theme::text_primary(),
    ));
    ui.add_space(6.0);
    let labels: Vec<String> = choices
        .iter()
        .map(|choice| choice.label.to_owned())
        .collect();
    let selected = choices
        .iter()
        .position(|choice| choice.value == value.as_str())
        .or_else(|| {
            choices
                .iter()
                .position(|choice| choice.value == spec.default)
        })
        .unwrap_or(0);
    let picked = form::dropdown_field(ui, &labels, selected, true);
    if !spec.hint.is_empty() {
        ui.add_space(6.0);
        ui.label(kit::text(
            spec.hint,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    }
    match picked {
        Some(index) if choices[index].value != value.as_str() => {
            *value = choices[index].value.to_owned();
            true
        }
        _ => false,
    }
}

/// An inline reason a typed value is not saved.
pub(in crate::ui) fn problem_line(ui: &mut egui::Ui, problem: &str) {
    ui.label(kit::text(
        problem,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        kit::RED,
    ));
}

/// The row that stays when the card is folded. Returns the header's response and whether the
/// switch was flipped this frame.
fn header(
    ui: &mut egui::Ui,
    provider: &dyn SnapshotProvider,
    settings: &FieldValues,
    (enabled, readiness, checking): (bool, &Readiness, bool),
    open: bool,
) -> (egui::Response, bool) {
    let descriptor = provider.descriptor();
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), HEADER_HEIGHT),
        egui::Sense::click(),
    );
    // Said in words as well: at a narrow width the state is only a coloured dot.
    let word = match (enabled, readiness) {
        (false, _) => "off",
        (true, _) if checking => "on, checking",
        (true, Readiness::Ready) => "on",
        (true, Readiness::Needs(_)) => "on, setup needed",
    };
    let response = crate::ui::accessibility::label(
        response,
        format!(
            "{}, {word}, {}",
            descriptor.name,
            if open { "expanded" } else { "collapsed" }
        ),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand);

    let tile = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - 15.0),
        vec2(30.0, 30.0),
    );
    paint_logo(ui, descriptor.logo, tile);
    let chevron = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        vec2(16.0, 16.0),
    );
    icons::paint(
        ui.painter(),
        chevron,
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        theme::text_secondary(),
    );
    // The switch sits on top of the header, so a click on it does not fold the card.
    let slot = egui::Rect::from_center_size(
        egui::pos2(chevron.left() - 12.0 - 16.0, rect.center().y),
        vec2(32.0, 18.0),
    );
    let mut on = enabled;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(slot));
    let flipped = kit::switch(&mut child, &mut on, &format!("Use {}", descriptor.name)).clicked();

    let left = tile.right() + 12.0;
    let right = slot.left() - 12.0;
    let badge = state_badge(enabled, readiness, checking);
    // A narrow page gives the words the room and leaves the state to a dot.
    let compact = right - left < COMPACT_ROOM;
    let state_left = match badge {
        Some((_, tone)) if compact => {
            let dot = egui::pos2(right - 4.0, rect.center().y);
            ui.painter().circle_filled(dot, 4.0, tone.dot());
            right - 8.0
        }
        Some((label, tone)) => {
            kit::paint_status_badge(ui, right, rect.center().y, label, tone).left()
        }
        None => right + 4.0,
    };
    let room = state_left - 12.0 - left;
    let summary_text = if enabled && !checking && matches!(readiness, Readiness::Ready) {
        provider.status_line(settings)
    } else {
        descriptor.summary.to_owned()
    };
    let name = fitted(
        ui,
        descriptor.name,
        kit::size::HEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        room,
    );
    let summary = fitted(
        ui,
        &summary_text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
        room,
    );
    let top = rect.center().y - (name.size().y + 2.0 + summary.size().y) / 2.0;
    let name_height = name.size().y;
    ui.painter()
        .galley(egui::pos2(left, top), name, theme::text_primary());
    ui.painter().galley(
        egui::pos2(left, top + name_height + 2.0),
        summary,
        theme::text_secondary(),
    );
    (response, flipped)
}

/// A provider's mark: its own logo with a hairline, or a bare glyph.
fn paint_logo(ui: &egui::Ui, logo: Logo, tile: egui::Rect) {
    match logo {
        Logo::Glyph(glyph) => kit::paint_card_badge(
            ui,
            tile,
            match glyph {
                Glyph::Server => Icon::Code,
            },
        ),
        Logo::Image(source) => {
            egui::Image::new(source())
                .corner_radius(tile.width() * 0.22)
                .paint_at(ui, tile);
            ui.painter().rect_stroke(
                tile,
                tile.width() * 0.22,
                egui::Stroke::new(1.0, theme::ink(14)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::snapshot::SELF_HOSTED;
    use crate::service::snapshot_providers::{imgbb, self_hosted};

    fn imgbb_descriptor() -> &'static ProviderDescriptor {
        imgbb::IMGBB.descriptor()
    }

    fn host_descriptor() -> &'static ProviderDescriptor {
        self_hosted::SELF_HOSTED_PROVIDER.descriptor()
    }

    /// A form whose fields hold `pairs`, all of them typed by the user.
    fn form(pairs: &[(&str, &str)]) -> ProviderForm {
        ProviderForm {
            values: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            dirty: pairs.iter().map(|(k, _)| (*k).to_owned()).collect(),
            loaded_from: Some(FieldValues::new()),
            ..ProviderForm::default()
        }
    }

    fn saved(pairs: &[(&str, &str)]) -> FieldValues {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn the_fields_start_with_what_is_saved_and_a_choice_with_its_default() {
        let mut empty = ProviderForm::default();
        empty.sync_with(imgbb_descriptor(), &FieldValues::new());
        assert_eq!(empty.typed("expiry"), "30d");
        assert_eq!(empty.typed("key"), "", "a secret is never loaded");
        let mut kept = ProviderForm::default();
        kept.sync_with(imgbb_descriptor(), &saved(&[("expiry", "7d")]));
        assert_eq!(kept.typed("expiry"), "7d");
        let mut host = ProviderForm::default();
        host.sync_with(
            host_descriptor(),
            &saved(&[("host", "https://s.example/up")]),
        );
        assert_eq!(host.typed("host"), "https://s.example/up");
    }

    #[test]
    fn a_field_the_user_did_not_touch_is_never_sent() {
        // The expiry shown is only a default, or a value another version saved that this one
        // does not know: neither may be written back because the user typed a key.
        let imgbb = imgbb_descriptor();
        let mut untouched = ProviderForm::default();
        untouched.sync_with(imgbb, &saved(&[("expiry", "365d")]));
        assert_eq!(untouched.typed("expiry"), "30d", "shown as the default");
        untouched
            .values
            .insert("key".to_owned(), "abc123".to_owned());
        assert!(
            untouched
                .changed_fields(imgbb, &saved(&[("expiry", "365d")]))
                .is_empty(),
            "nothing of the saved settings is rewritten"
        );
        assert!(untouched.typed_secrets(imgbb).contains_key("key"));
    }

    #[test]
    fn what_syncs_in_while_nothing_is_being_typed_reaches_the_fields() {
        let descriptor = imgbb_descriptor();
        let mut form = ProviderForm::default();
        form.sync_with(descriptor, &saved(&[("expiry", "30d")]));
        assert_eq!(form.typed("expiry"), "30d");
        // Another Mac changed it.
        form.sync_with(descriptor, &saved(&[("expiry", "7d")]));
        assert_eq!(form.typed("expiry"), "7d");
        // The user is in the middle of changing it: their choice stands, and is the one saved.
        form.values.insert("expiry".to_owned(), "1d".to_owned());
        form.dirty.insert("expiry".to_owned());
        form.sync_with(descriptor, &saved(&[("expiry", "90d")]));
        assert_eq!(form.typed("expiry"), "1d");
        assert_eq!(
            form.changed_fields(descriptor, &saved(&[("expiry", "90d")]))
                .get("expiry")
                .map(String::as_str),
            Some("1d")
        );
    }

    #[test]
    fn a_wrong_field_does_not_stop_a_right_one_from_being_saved() {
        let descriptor = imgbb_descriptor();
        // A key with a space is wrong; the new lifetime is a choice and fine.
        let form = form(&[("key", "has space"), ("expiry", "7d")]);
        assert!(
            form.typed_secrets(descriptor).is_empty(),
            "the wrong key stays in its field"
        );
        let changed = form.changed_fields(descriptor, &FieldValues::new());
        assert_eq!(changed.get("expiry").map(String::as_str), Some("7d"));
        assert!(
            form.has_problem(descriptor),
            "and its problem is still shown"
        );
    }

    #[test]
    fn only_what_differs_from_what_is_saved_is_sent() {
        let descriptor = host_descriptor();
        let settings = saved(&[("host", "https://s.example/up")]);
        assert!(
            form(&[("host", "https://s.example/up")])
                .changed_fields(descriptor, &settings)
                .is_empty()
        );
        assert!(
            form(&[("host", " https://s.example/up ")])
                .changed_fields(descriptor, &settings)
                .is_empty(),
            "spaces are not a change"
        );
        let changed =
            form(&[("host", "https://other.example/up")]).changed_fields(descriptor, &settings);
        assert_eq!(
            changed.get("host").map(String::as_str),
            Some("https://other.example/up")
        );
        // Clearing the field is a change: it clears the setting.
        let cleared = form(&[("host", "")]).changed_fields(descriptor, &settings);
        assert_eq!(cleared.get("host").map(String::as_str), Some(""));
        assert!(
            form(&[("host", "")])
                .changed_fields(descriptor, &FieldValues::new())
                .is_empty()
        );
    }

    #[test]
    fn a_choice_left_on_its_default_is_not_a_change_and_another_is() {
        let descriptor = imgbb_descriptor();
        assert!(
            form(&[("expiry", "30d")])
                .changed_fields(descriptor, &FieldValues::new())
                .is_empty()
        );
        let picked = form(&[("expiry", "7d")]).changed_fields(descriptor, &FieldValues::new());
        assert_eq!(picked.get("expiry").map(String::as_str), Some("7d"));
        // Back to the default after another was saved is a change.
        let back =
            form(&[("expiry", "30d")]).changed_fields(descriptor, &saved(&[("expiry", "7d")]));
        assert_eq!(back.get("expiry").map(String::as_str), Some("30d"));
    }

    #[test]
    fn a_secret_is_sent_only_when_typed_and_trimmed() {
        let descriptor = imgbb_descriptor();
        assert!(form(&[]).typed_secrets(descriptor).is_empty());
        assert!(form(&[("key", "   ")]).typed_secrets(descriptor).is_empty());
        let typed = form(&[("key", " abc123 ")]).typed_secrets(descriptor);
        assert_eq!(typed.get("key").map(String::as_str), Some("abc123"));
        // After a save, what was typed is gone from the field.
        let mut after = form(&[("key", "abc123"), ("expiry", "7d")]);
        after.saved(descriptor);
        assert_eq!(after.typed("key"), "");
        assert_eq!(after.typed("expiry"), "7d");
    }

    #[test]
    fn a_value_that_is_wrong_is_named_and_blocks_the_save() {
        let host = host_descriptor();
        assert!(form(&[("host", "s.example")]).has_problem(host));
        assert!(!form(&[("host", "https://s.example/up")]).has_problem(host));
        assert!(!form(&[("host", "")]).has_problem(host), "empty clears it");
        assert!(form(&[("token", "has space")]).has_problem(host));
        assert!(form(&[("key", "has space")]).has_problem(imgbb_descriptor()));
        assert!(!form(&[("expiry", "whatever")]).has_problem(imgbb_descriptor()));
    }

    #[test]
    fn the_test_waits_for_what_the_provider_needs() {
        let imgbb = &imgbb::IMGBB;
        let none = SecretPresence::default();
        assert!(!form(&[]).ready_as_typed(imgbb, &FieldValues::new(), none.clone()));
        assert!(
            form(&[("key", "abc123")]).ready_as_typed(imgbb, &FieldValues::new(), none.clone()),
            "a typed key is enough"
        );
        let mut saved_key = SecretPresence::default();
        saved_key.insert("key");
        assert!(form(&[]).ready_as_typed(imgbb, &FieldValues::new(), saved_key.clone()));
        assert!(
            !form(&[("key", "has space")]).ready_as_typed(imgbb, &FieldValues::new(), saved_key),
            "a key that is wrong is not tested"
        );
    }

    #[test]
    fn the_settings_as_typed_lie_over_the_saved_ones() {
        let descriptor = host_descriptor();
        let typed = form(&[("host", " https://new.example/up ")]).settings_as_typed(
            descriptor,
            &saved(&[("host", "https://old.example/up"), ("extra", "kept")]),
        );
        assert_eq!(
            typed.get("host").map(String::as_str),
            Some("https://new.example/up")
        );
        assert_eq!(typed.get("extra").map(String::as_str), Some("kept"));
        assert!(
            !form(&[("host", "")])
                .settings_as_typed(descriptor, &saved(&[("host", "https://old.example/up")]))
                .contains_key("host")
        );
    }

    #[test]
    fn the_header_says_how_a_provider_is_doing() {
        assert_eq!(state_badge(false, &Readiness::Ready, false), None);
        assert_eq!(
            state_badge(false, &Readiness::Needs("x".into()), false),
            None,
            "an off provider has nothing to report"
        );
        assert_eq!(
            state_badge(true, &Readiness::Ready, false),
            Some(("Active", Tone::Good))
        );
        assert_eq!(
            state_badge(true, &Readiness::Needs("x".into()), false),
            Some(("Setup needed", Tone::Warning))
        );
        assert_eq!(
            state_badge(true, &Readiness::Needs("x".into()), true),
            Some(("Checking", Tone::Neutral)),
            "while the key is being looked for, no guess"
        );
    }

    #[test]
    fn a_secret_says_where_it_comes_from() {
        let key = imgbb_descriptor().field("key").unwrap();
        assert!(secret_hint(key, SecretState::Saved).contains("secure storage"));
        assert!(secret_hint(key, SecretState::FromEnvironment).contains("DEGENVIEW_IMGBB_KEY"));
        assert_eq!(secret_hint(key, SecretState::Missing), key.hint);
        let token = host_descriptor().field("token").unwrap();
        assert!(
            secret_hint(token, SecretState::FromEnvironment).contains("DEGENVIEW_SNAPSHOT_TOKEN")
        );
    }

    #[test]
    fn every_secret_can_be_removed_and_says_so() {
        for provider in crate::service::snapshot_providers::registry() {
            let descriptor = provider.descriptor();
            for field in descriptor.secret_fields() {
                assert!(
                    !field.remove_label.is_empty(),
                    "{}.{} needs a remove button label",
                    descriptor.id,
                    field.key
                );
            }
        }
        assert_eq!(SELF_HOSTED, "self_hosted");
    }
}
