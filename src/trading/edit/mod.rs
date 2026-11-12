//! Editing an open order in place: the rules and the state machine behind the Edit sheet.
//!
//! An [`EditSession`] is the whole life of one edit: the typed price and size, the broker's preview
//! of exactly that change, the review the user approves and the amend that follows. The screen
//! hands it a fresh [`EditFrame`] every frame (the order as the broker has it now, the account and
//! connection, the market's rules), draws the [`EditView`] it returns and passes back what the user
//! pressed as [`EditEvent`]s. Nothing here draws or waits: time is a parameter, and a venue's answers
//! arrive as polled [`Task`]s.
//!
//! The rules it keeps, each a transition with a test:
//!
//! * A change is sent only after the broker's preview of **exactly that change, on exactly that
//!   order, on this connection**, and only while that preview is younger than [`PREVIEW_FRESH`].
//! * A review stands only while the order is unchanged: a fill, an edit made elsewhere, another
//!   account or another connection drops it back to editing and nothing is sent.
//! * The amend carries the fingerprint of the order that was reviewed, which the venue checks again
//!   before anything leaves.

use std::time::{Duration, Instant};

use degenview_brokers::model::orders::{EditOutcome, EditPreview, OrderChange, OrderFingerprint};
use degenview_brokers::{
    AccountId, Decimal, LiveOrder, OrderId, OrderSize, OrderStatus, OrderType, ProductSpec, Side,
};

use super::task::Task;
use crate::model::broker::ticket::{
    self as rules, PREVIEW_DEBOUNCE, PREVIEW_FRESH, PreviewAnswer, PreviewGate, preview_gate,
};

/// Why a venue call failed, in words for the user, and whether the venue may nevertheless have
/// acted (the connection dropped before an answer): then the order is read again, not retried.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditError {
    pub text: String,
    pub outcome_unknown: bool,
}

pub type EditAnswer<T> = Task<Result<T, EditError>>;

/// The order and what an edit of it is read against, as the venue has them now.
#[derive(Clone, Debug, Default)]
pub struct EditWorld {
    /// The order; `None` once it is gone.
    pub live: Option<LiveOrder>,
    pub account: Option<AccountId>,
    pub generation: u64,
    pub spec: Option<ProductSpec>,
}

/// What the venue does for an edit. Implemented by a connected broker.
pub trait EditVenue {
    /// The order with this id and the account, connection and market rules around it.
    fn world(&self, order: &OrderId) -> EditWorld;

    /// Asks what the change would do, without applying it.
    fn preview_edit(&self, order: OrderId, change: OrderChange) -> EditAnswer<EditPreview>;

    /// Applies the change, but only to the order `base` describes.
    fn amend(
        &self,
        order: OrderId,
        change: OrderChange,
        base: OrderFingerprint,
    ) -> EditAnswer<EditOutcome>;
}

/// What the venue says about the world this frame.
pub struct EditFrame<'a> {
    pub now: Instant,
    /// The order as the venue has it now; `None` once it is gone.
    pub live: Option<&'a LiveOrder>,
    /// The account the orders go to; `None` while there is none.
    pub account: Option<&'a AccountId>,
    /// Which connection the account is read on; a preview says nothing about another one.
    pub generation: u64,
    pub spec: Option<&'a ProductSpec>,
}

/// What the user did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditEvent {
    /// "Review change": show the change for approval.
    Review,
    /// "Back" from the approval.
    Back,
    /// "Change order" in the approval: send it.
    Confirm,
    /// "Try again" after a failed preview.
    TryAgain,
    /// "Cancel" or Escape: close without sending (ignored while an amend is in flight).
    Cancel,
}

/// A message for the user once the sheet has closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

#[derive(Debug)]
pub enum Step {
    Open,
    Closed(Option<Notice>),
}

/// Everything a preview is a statement about. A preview answers one of these and no other.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Asked {
    change: OrderChange,
    order: OrderFingerprint,
    generation: u64,
}

enum Stage {
    Editing,
    /// The change, and the broker's preview of exactly it.
    Confirming(OrderChange, Box<EditPreview>),
    Sending(EditAnswer<EditOutcome>),
}

/// The broker's preview of what is typed, and when to ask for it.
struct Preview {
    /// What the sheet last asked about; `None` while what is typed is not a valid change.
    seen: Option<Asked>,
    /// When `seen` last changed: the question waits until typing pauses.
    changed_at: Instant,
    pending: Option<(Asked, EditAnswer<EditPreview>)>,
    answer: Option<EditPreview>,
    failure: Option<String>,
    /// What the answer or failure is about.
    answered_for: Option<Asked>,
    /// When the broker's answer arrived.
    at: Option<Instant>,
}

impl Preview {
    fn new(now: Instant) -> Self {
        Self {
            seen: None,
            changed_at: now,
            pending: None,
            answer: None,
            failure: None,
            answered_for: None,
            at: None,
        }
    }

    fn forget(&mut self) {
        self.pending = None;
        self.answer = None;
        self.failure = None;
        self.answered_for = None;
        self.at = None;
    }

    /// Asks again at once.
    fn ask_now(&mut self, now: Instant) {
        self.changed_at = now.checked_sub(PREVIEW_DEBOUNCE).unwrap_or(now);
    }

    fn age(&self, now: Instant) -> Option<Duration> {
        self.at.map(|at| now.saturating_duration_since(at))
    }

    fn is_old(&self, now: Instant) -> bool {
        self.age(now).is_some_and(|age| age > PREVIEW_FRESH)
    }
}

/// What this frame worked out about what is typed.
struct Computed {
    change: Result<OrderChange, String>,
    problems: Vec<String>,
    key: Option<Asked>,
    gate: PreviewGate,
    can_review: bool,
}

impl Default for Computed {
    fn default() -> Self {
        Self {
            change: Err(String::new()),
            problems: Vec::new(),
            key: None,
            gate: PreviewGate::Waiting,
            can_review: false,
        }
    }
}

pub struct EditSession {
    order: LiveOrder,
    price_text: String,
    size_text: String,
    preview: Preview,
    stage: Stage,
    error: Option<String>,
    computed: Computed,
}

impl EditSession {
    /// Starts editing `order`; `price` is where a dragged order line was dropped, if it was.
    pub fn new(order: LiveOrder, price: Option<Decimal>, now: Instant) -> Self {
        let price_text = price
            .or(order.limit_price)
            .map(|p| p.to_string())
            .unwrap_or_default();
        let size_text = match order.size {
            Some(OrderSize::Base(size)) => size.to_string(),
            _ => String::new(),
        };
        Self {
            order,
            price_text,
            size_text,
            preview: Preview::new(now),
            stage: Stage::Editing,
            error: None,
            computed: Computed::default(),
        }
    }

    /// The two text boxes: price (the stop price of a stop-limit order) and size.
    pub fn fields_mut(&mut self) -> (&mut String, &mut String) {
        (&mut self.price_text, &mut self.size_text)
    }

    /// The order as the venue last reported it.
    pub fn order(&self) -> &LiveOrder {
        &self.order
    }

    pub fn is_sending(&self) -> bool {
        matches!(self.stage, Stage::Sending(_))
    }

    pub fn is_confirming(&self) -> bool {
        matches!(self.stage, Stage::Confirming(..))
    }

    /// Whether the broker's accepting answer for what is typed has arrived.
    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        self.preview.answered_for.is_some()
            && self.preview.answered_for == self.preview.seen
            && self
                .preview
                .answer
                .as_ref()
                .is_some_and(EditPreview::is_acceptable)
    }

    /// Whether "Review change" was enabled in the last frame.
    #[cfg(test)]
    pub fn can_review(&self) -> bool {
        self.computed.can_review
    }

    #[cfg(test)]
    pub fn preview_failed(&self) -> bool {
        self.preview.failure.is_some()
    }

    /// The filled size of the order, as the sheet shows it.
    #[cfg(test)]
    pub fn filled_size(&self) -> Decimal {
        self.order.filled_size
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reconciles the sheet with the world for one frame: adopts the order as it is now, works out
    /// what is typed, keeps the broker's preview of it current and settles an amend that was sent.
    pub fn update(&mut self, frame: &EditFrame, venue: Option<&dyn EditVenue>) -> Step {
        let Some(venue) = venue else {
            return Step::Closed(None);
        };
        let now = frame.now;
        if frame
            .live
            .is_none_or(|order| order.status != OrderStatus::Open)
            && !self.is_sending()
        {
            return closed("The order is no longer open, so it was not edited.", true);
        }
        let Some(account) = frame.account else {
            return closed(
                "The broker is not connected, so the order was not edited.",
                true,
            );
        };
        // Every figure below (what changed, the filled size, the old values in the question) is the
        // order as it is now, not as it was when the sheet opened: a fill or an edit made elsewhere
        // meanwhile must show, and must not leave a review standing that describes the old order.
        if let Some(order) = frame.live {
            self.order = order.clone();
        }
        let fingerprint = OrderFingerprint::of(account, &self.order);
        let change = self.typed_change();
        let problems = match (&change, frame.spec) {
            (Ok(change), Some(spec)) => issues(&self.order, change, spec),
            _ => Vec::new(),
        };
        let key = match &change {
            Ok(change) if problems.is_empty() => Some(Asked {
                change: *change,
                order: fingerprint.clone(),
                generation: frame.generation,
            }),
            _ => None,
        };

        self.sync_preview(key.as_ref(), now, venue);
        if let Some(step) = self.settle_amend() {
            return step;
        }
        self.drop_stale_review(&fingerprint, now);

        let answer = match (&key, &self.preview) {
            (Some(key), preview) if preview.answered_for.as_ref() == Some(key) => {
                match (&preview.answer, &preview.failure) {
                    (Some(answer), _) if answer.is_acceptable() => PreviewAnswer::Accepted,
                    (Some(_), _) => PreviewAnswer::Problems,
                    (None, Some(error)) => PreviewAnswer::Failed(error.clone()),
                    (None, None) => PreviewAnswer::Waiting,
                }
            }
            _ => PreviewAnswer::Waiting,
        };
        let gate = preview_gate(answer, self.preview.age(now));
        let can_review = key.is_some() && gate.allows_review();
        self.computed = Computed {
            change,
            problems,
            key,
            gate,
            can_review,
        };
        Step::Open
    }

    /// What the user pressed.
    pub fn handle(&mut self, event: EditEvent, now: Instant, venue: &dyn EditVenue) -> Step {
        match event {
            EditEvent::Review => {
                if self.computed.can_review
                    && let Ok(change) = self.computed.change.clone()
                    && let Some(preview) = self.preview.answer.clone()
                    && matches!(self.stage, Stage::Editing)
                {
                    self.stage = Stage::Confirming(change, Box::new(preview));
                }
            }
            EditEvent::Back => {
                if self.is_confirming() {
                    self.stage = Stage::Editing;
                }
            }
            EditEvent::Confirm => {
                if let Stage::Confirming(change, preview) = &self.stage {
                    let change = *change;
                    let base = preview.base.clone();
                    self.error = None;
                    match base {
                        // Sent against the order that was reviewed, which the broker's worker
                        // checks again before anything leaves.
                        Some(base) => {
                            self.stage =
                                Stage::Sending(venue.amend(self.order.id.clone(), change, base));
                        }
                        None => {
                            self.error = Some("This change has not been checked.".to_owned());
                            self.stage = Stage::Editing;
                        }
                    }
                }
            }
            EditEvent::TryAgain => {
                self.preview.failure = None;
                self.preview.answered_for = None;
                self.preview.at = None;
                self.preview.ask_now(now);
            }
            EditEvent::Cancel => {
                if !self.is_sending() {
                    return Step::Closed(None);
                }
            }
        }
        Step::Open
    }

    /// What to draw for the last frame.
    pub fn view(&self) -> EditView {
        if let Stage::Confirming(change, preview) = &self.stage {
            return EditView::Reviewing(self.review_lines(change, preview));
        }
        let computed = &self.computed;
        let order = &self.order;
        let preview = self.preview.answer.as_ref();
        EditView::Editing(EditForm {
            heading: format!(
                "{} {} {} · {}",
                if order.side == Side::Buy {
                    "Buy"
                } else {
                    "Sell"
                },
                rules::size_text(order),
                order.product,
                rules::type_label(order)
            ),
            price_title: if order.order_type == OrderType::StopLimit {
                "Stop price"
            } else {
                "Limit price"
            },
            typed_error: computed
                .change
                .as_ref()
                .err()
                .filter(|text| !text.starts_with("Nothing") && !text.is_empty())
                .cloned(),
            problems: computed.problems.clone(),
            broker_problems: preview
                .map(|preview| {
                    preview
                        .problems
                        .iter()
                        .map(|problem| problem.text.clone())
                        .collect()
                })
                .unwrap_or_default(),
            broker_total: preview
                .filter(|preview| preview.is_acceptable())
                .and_then(|preview| preview.order_total)
                .map(rules::display),
            asking: computed.key.is_some() && computed.gate == PreviewGate::Waiting,
            gate: computed.gate.clone(),
            error: self.error.clone(),
            can_review: computed.can_review,
            sending: self.is_sending(),
        })
    }

    fn typed_change(&self) -> Result<OrderChange, String> {
        let order = &self.order;
        let price = rules::parse_decimal(&self.price_text)
            .map_err(|()| "The price is not a number.".to_owned())?;
        let size = rules::parse_decimal(&self.size_text)
            .map_err(|()| "The size is not a number.".to_owned())?;
        let change = OrderChange {
            price: price
                .filter(|p| Some(*p) != order.limit_price && order.order_type == OrderType::Limit),
            stop_price: price.filter(|p| {
                Some(*p) != order.stop_price && order.order_type == OrderType::StopLimit
            }),
            size: size.filter(|s| matches!(order.size, Some(OrderSize::Base(b)) if b != *s)),
        };
        if change.is_empty() {
            Err("Nothing has changed yet.".to_owned())
        } else {
            Ok(change)
        }
    }

    /// Keeps the broker's preview current: forgets it when what it describes changed, asks again
    /// when it grew old, takes an answer that arrived and asks once typing has paused.
    fn sync_preview(&mut self, key: Option<&Asked>, now: Instant, venue: &dyn EditVenue) {
        let editing = matches!(self.stage, Stage::Editing);
        let preview = &mut self.preview;
        if key != preview.seen.as_ref() {
            preview.seen = key.cloned();
            preview.changed_at = now;
            preview.forget();
        }
        // An answer older than a minute is a statement about an older market: it is asked for again.
        if preview.is_old(now) && editing {
            preview.answer = None;
            preview.answered_for = None;
            preview.at = None;
            preview.ask_now(now);
        }
        if let Some((asked, task)) = &mut preview.pending
            && let Some(answer) = task.poll().cloned()
        {
            let asked = asked.clone();
            preview.pending = None;
            if Some(&asked) == preview.seen.as_ref() {
                match answer {
                    Ok(answer) => {
                        preview.answer = Some(answer);
                        preview.at = Some(now);
                    }
                    Err(error) => {
                        preview.failure = Some(error.text);
                        preview.at = None;
                    }
                }
                preview.answered_for = Some(asked);
            }
        }
        if let Some(key) = key
            && preview.answered_for.as_ref() != Some(key)
            && preview.pending.is_none()
            && now.saturating_duration_since(preview.changed_at) >= PREVIEW_DEBOUNCE
            && editing
        {
            preview.pending = Some((
                key.clone(),
                venue.preview_edit(self.order.id.clone(), key.change),
            ));
        }
    }

    /// Takes the answer of an amend that was sent: done, or back to editing with the reason.
    fn settle_amend(&mut self) -> Option<Step> {
        let Stage::Sending(task) = &mut self.stage else {
            return None;
        };
        match task.poll()?.clone() {
            Ok(_) => Some(closed(
                "Edit sent. The order updates when the broker confirms it.",
                false,
            )),
            Err(error) if error.outcome_unknown => Some(closed(
                &format!(
                    "{} The order is being read again to see what happened.",
                    error.text
                ),
                true,
            )),
            Err(error) => {
                self.error = Some(error.text);
                self.stage = Stage::Editing;
                None
            }
        }
    }

    /// A review stands only while the order is the one it was worked out for, and the answer is
    /// recent: otherwise the sheet goes back to editing, and asks again.
    fn drop_stale_review(&mut self, fingerprint: &OrderFingerprint, now: Instant) {
        let Stage::Confirming(_, preview) = &self.stage else {
            return;
        };
        let same_order = preview.base.as_ref() == Some(fingerprint);
        let recent = self
            .preview
            .age(now)
            .is_some_and(|age| age <= PREVIEW_FRESH);
        if !same_order || !recent {
            self.stage = Stage::Editing;
            self.error = Some(if same_order {
                "The broker's check of this change is out of date. Review it again.".to_owned()
            } else {
                "The order changed while you were reviewing it. Nothing was sent. Review it again."
                    .to_owned()
            });
        }
    }

    fn review_lines(&self, change: &OrderChange, preview: &EditPreview) -> Vec<ReviewLine> {
        let order = &self.order;
        let mut lines = vec![
            ReviewLine::new("Market", order.product.to_string()),
            ReviewLine::new(
                "Action",
                if order.side == Side::Buy {
                    "Buy"
                } else {
                    "Sell"
                }
                .to_owned(),
            ),
        ];
        if let Some(new) = change.price.or(change.stop_price) {
            let old = if change.price.is_some() {
                order.limit_price
            } else {
                order.stop_price
            };
            lines.push(ReviewLine::new(
                "Price",
                format!(
                    "{} → {}",
                    old.map_or("—".to_owned(), rules::display),
                    rules::display(new)
                ),
            ));
        }
        if let Some(new) = change.size {
            lines.push(ReviewLine::new(
                "Size",
                format!("{} → {}", rules::size_text(order), rules::display(new)),
            ));
        }
        // The broker's figures for the changed order, from a preview of exactly this change.
        if let Some(total) = preview.order_total {
            lines.push(ReviewLine::new(
                "Total (broker's estimate)",
                rules::display(total),
            ));
        }
        if let Some(fee) = preview.fees {
            lines.push(ReviewLine::new(
                "Fee (broker's estimate)",
                rules::display(fee),
            ));
        }
        lines
    }
}

fn closed(text: &str, error: bool) -> Step {
    Step::Closed(Some(Notice {
        text: text.to_owned(),
        error,
    }))
}

/// What is wrong with `change` on `order` in a market with these rules, before the broker is asked.
pub fn issues(order: &LiveOrder, change: &OrderChange, spec: &ProductSpec) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(price) = change.price.or(change.stop_price)
        && !degenview_brokers::validate::is_multiple(price, spec.price_increment)
    {
        problems.push(format!(
            "The price must be a multiple of {}. The nearest valid price below is {}.",
            rules::display(spec.price_increment),
            rules::price_display(
                degenview_brokers::validate::floor_to_increment(price, spec.price_increment),
                spec
            ),
        ));
    }
    if let Some(size) = change.size {
        if !degenview_brokers::validate::is_multiple(size, spec.base_increment) {
            problems.push(format!(
                "The size must be a multiple of {}.",
                rules::display(spec.base_increment)
            ));
        }
        if size < spec.base_min {
            problems.push(format!(
                "The size is below the minimum of {}.",
                rules::display(spec.base_min)
            ));
        }
        if size < order.filled_size {
            problems.push("The size cannot be less than what has already filled.".to_owned());
        }
    }
    problems
}

/// What the Edit sheet shows.
pub enum EditView {
    Editing(EditForm),
    /// The change laid out for approval.
    Reviewing(Vec<ReviewLine>),
}

pub struct EditForm {
    /// "Sell 0.5 BTC-USD · Limit".
    pub heading: String,
    pub price_title: &'static str,
    /// Why what is typed is not a change (not a number), unless it is simply unchanged.
    pub typed_error: Option<String>,
    /// What the market's rules say is wrong with the change.
    pub problems: Vec<String>,
    /// What the broker's preview says is wrong with it.
    pub broker_problems: Vec<String>,
    /// The broker's figure for the changed order, when it would accept it.
    pub broker_total: Option<String>,
    /// Waiting on the broker's preview of a valid change.
    pub asking: bool,
    pub gate: PreviewGate,
    pub error: Option<String>,
    pub can_review: bool,
    pub sending: bool,
}

pub struct ReviewLine {
    pub label: &'static str,
    pub value: String,
}

impl ReviewLine {
    fn new(label: &'static str, value: String) -> Self {
        Self { label, value }
    }
}

#[cfg(test)]
mod tests;
