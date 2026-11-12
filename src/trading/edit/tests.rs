use std::cell::RefCell;
use std::str::FromStr;
use std::time::{Duration, Instant};

use degenview_brokers::model::orders::{EditWorld, Problem};
use degenview_brokers::{OrderStatus, ProductId, Restrictions};

use super::*;

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn order() -> LiveOrder {
    LiveOrder {
        id: OrderId::new("o-1"),
        client_id: None,
        product: ProductId::new("BTC-USD"),
        side: Side::Sell,
        order_type: OrderType::Limit,
        status: OrderStatus::Open,
        tif: None,
        size: Some(OrderSize::Base(d("1"))),
        limit_price: Some(d("100")),
        stop_price: None,
        stop_direction: None,
        post_only: false,
        filled_size: d("0"),
        filled_value: d("0"),
        average_price: None,
        fees: d("0"),
        created_ms: Some(0),
        updated_ms: None,
        reason: None,
        stop_triggered: false,
    }
}

fn spec() -> ProductSpec {
    ProductSpec {
        id: ProductId::new("BTC-USD"),
        base: "BTC".to_owned(),
        quote: "USD".to_owned(),
        display_name: "BTC-USD".to_owned(),
        price_increment: d("0.01"),
        base_increment: d("0.001"),
        quote_increment: d("0.01"),
        base_min: d("0.01"),
        base_max: None,
        quote_min: d("1"),
        quote_max: None,
        restrictions: Restrictions::default(),
        fetched_ms: 0,
    }
}

fn accepted(order: &LiveOrder, account: &AccountId) -> EditPreview {
    EditPreview {
        order_total: Some(d("99")),
        fees: Some(d("0.5")),
        base: Some(OrderFingerprint::of(account, order)),
        ..EditPreview::default()
    }
}

/// A venue that answers at once with what the test set, and remembers what it was asked.
#[derive(Default)]
struct Venue {
    preview: RefCell<Option<Result<EditPreview, EditError>>>,
    amend: RefCell<Option<Result<EditOutcome, EditError>>>,
    previews: RefCell<Vec<OrderChange>>,
    amends: RefCell<Vec<(OrderChange, OrderFingerprint)>>,
}

impl EditVenue for Venue {
    fn world(&self, _: &OrderId) -> EditWorld {
        EditWorld::default()
    }

    fn preview_edit(&self, _: OrderId, change: OrderChange) -> EditAnswer<EditPreview> {
        self.previews.borrow_mut().push(change);
        match self.preview.borrow().clone() {
            Some(answer) => Task::ready(answer),
            None => Task::new(|| None),
        }
    }

    fn amend(
        &self,
        _: OrderId,
        change: OrderChange,
        base: OrderFingerprint,
    ) -> EditAnswer<EditOutcome> {
        self.amends.borrow_mut().push((change, base));
        match self.amend.borrow().clone() {
            Some(answer) => Task::ready(answer),
            None => Task::new(|| None),
        }
    }
}

struct Rig {
    session: EditSession,
    venue: Venue,
    now: Instant,
    live: Option<LiveOrder>,
    account: Option<AccountId>,
    generation: u64,
    spec: Option<ProductSpec>,
}

impl Rig {
    fn new() -> Self {
        let now = Instant::now();
        let rig = Self {
            session: EditSession::new(order(), Some(d("101")), now),
            venue: Venue::default(),
            now,
            live: Some(order()),
            account: Some(AccountId::new("acct")),
            generation: 1,
            spec: Some(spec()),
        };
        rig.venue
            .preview
            .replace(Some(Ok(accepted(&order(), rig.account.as_ref().unwrap()))));
        rig
    }

    fn update(&mut self) -> Step {
        let frame = EditFrame {
            now: self.now,
            live: self.live.as_ref(),
            account: self.account.as_ref(),
            generation: self.generation,
            spec: self.spec.as_ref(),
        };
        self.session.update(&frame, Some(&self.venue))
    }

    fn press(&mut self, event: EditEvent) -> Step {
        self.session.handle(event, self.now, &self.venue)
    }

    fn wait(&mut self, by: Duration) {
        self.now += by;
    }

    /// Typed, settled, asked and answered: ready to review.
    fn ready(&mut self) {
        self.update();
        self.wait(PREVIEW_DEBOUNCE);
        self.update();
        self.update();
        assert!(self.session.is_ready(), "the preview should have arrived");
    }

    fn confirming(&mut self) {
        self.ready();
        self.press(EditEvent::Review);
        assert!(self.session.is_confirming());
    }

    fn type_price(&mut self, text: &str) {
        text.clone_into(self.session.fields_mut().0);
    }
}

fn closed_with(step: Step) -> Option<Notice> {
    match step {
        Step::Closed(notice) => notice,
        Step::Open => panic!("the sheet should have closed"),
    }
}

#[test]
fn nothing_is_asked_until_typing_has_paused() {
    let mut rig = Rig::new();
    rig.update();
    assert!(rig.venue.previews.borrow().is_empty());
    rig.wait(PREVIEW_DEBOUNCE - Duration::from_millis(1));
    rig.update();
    assert!(
        rig.venue.previews.borrow().is_empty(),
        "still within the pause"
    );
    rig.wait(Duration::from_millis(1));
    rig.update();
    assert_eq!(rig.venue.previews.borrow().len(), 1);
    rig.update();
    assert_eq!(
        rig.venue.previews.borrow().len(),
        1,
        "asked once, not every frame"
    );
}

#[test]
fn review_is_enabled_only_on_the_brokers_fresh_accepting_answer() {
    let mut rig = Rig::new();
    rig.update();
    assert!(!rig.session.can_review(), "no answer yet");
    rig.press(EditEvent::Review);
    assert!(
        !rig.session.is_confirming(),
        "a press before the answer does nothing"
    );
    rig.ready();
    assert!(rig.session.can_review());
    // The answer ages out: it is asked for again and review is off meanwhile.
    rig.venue.preview.replace(None);
    rig.wait(PREVIEW_FRESH + Duration::from_secs(1));
    rig.update();
    assert!(!rig.session.can_review());
    rig.update();
    assert_eq!(rig.venue.previews.borrow().len(), 2, "asked again");
}

#[test]
fn a_preview_with_problems_or_a_failure_blocks_review_and_try_again_asks_at_once() {
    let mut rig = Rig::new();
    let account = rig.account.clone().unwrap();
    let mut refused = accepted(&order(), &account);
    refused.problems = vec![Problem {
        code: None,
        text: "Insufficient funds".to_owned(),
    }];
    rig.venue.preview.replace(Some(Ok(refused)));
    rig.update();
    rig.wait(PREVIEW_DEBOUNCE);
    rig.update();
    rig.update();
    assert!(!rig.session.can_review());
    let EditView::Editing(form) = rig.session.view() else {
        panic!("editing")
    };
    assert_eq!(form.broker_problems, ["Insufficient funds"]);
    assert_eq!(form.gate, PreviewGate::Blocked);

    rig.venue.preview.replace(Some(Err(EditError {
        text: "The connection dropped".to_owned(),
        outcome_unknown: false,
    })));
    rig.press(EditEvent::TryAgain);
    rig.update();
    rig.update();
    assert!(rig.session.preview_failed());
    assert!(!rig.session.can_review());
    let before = rig.venue.previews.borrow().len();
    rig.venue
        .preview
        .replace(Some(Ok(accepted(&order(), &account))));
    rig.press(EditEvent::TryAgain);
    rig.update();
    assert_eq!(
        rig.venue.previews.borrow().len(),
        before + 1,
        "asked again at once, without waiting"
    );
    rig.update();
    assert!(rig.session.can_review());
}

#[test]
fn what_is_typed_is_checked_against_the_markets_rules_before_the_broker_is_asked() {
    let mut rig = Rig::new();
    rig.type_price("100.005");
    rig.update();
    let EditView::Editing(form) = rig.session.view() else {
        panic!("editing")
    };
    assert!(form.problems[0].starts_with("The price must be a multiple of 0.01"));
    rig.wait(PREVIEW_DEBOUNCE);
    rig.update();
    assert!(
        rig.venue.previews.borrow().is_empty(),
        "an invalid change is never previewed"
    );

    "0.0005".clone_into(rig.session.fields_mut().1);
    rig.type_price("101");
    rig.update();
    let EditView::Editing(form) = rig.session.view() else {
        panic!("editing")
    };
    assert!(
        form.problems
            .iter()
            .any(|p| p.contains("below the minimum"))
    );
    assert!(
        form.problems
            .iter()
            .any(|p| p.contains("multiple of 0.001"))
    );
}

#[test]
fn the_size_cannot_go_below_what_has_filled() {
    let mut rig = Rig::new();
    let mut partly = order();
    partly.filled_size = d("0.6");
    rig.live = Some(partly);
    "0.5".clone_into(rig.session.fields_mut().1);
    rig.update();
    let EditView::Editing(form) = rig.session.view() else {
        panic!("editing")
    };
    assert_eq!(
        form.problems,
        ["The size cannot be less than what has already filled."]
    );
}

#[test]
fn text_that_is_not_a_number_or_not_a_change_says_so() {
    let mut rig = Rig::new();
    rig.type_price("abc");
    rig.update();
    let EditView::Editing(form) = rig.session.view() else {
        panic!("editing")
    };
    assert_eq!(
        form.typed_error.as_deref(),
        Some("The price is not a number.")
    );
    rig.type_price("100");
    rig.update();
    let EditView::Editing(form) = rig.session.view() else {
        panic!("editing")
    };
    assert_eq!(
        form.typed_error, None,
        "unchanged is not an error, only nothing to review"
    );
    assert!(!form.can_review);
}

#[test]
fn a_review_is_shown_with_the_old_and_new_terms_and_the_brokers_figures() {
    let mut rig = Rig::new();
    rig.confirming();
    let EditView::Reviewing(lines) = rig.session.view() else {
        panic!("reviewing")
    };
    let text: Vec<(&str, &str)> = lines.iter().map(|l| (l.label, l.value.as_str())).collect();
    assert_eq!(
        text,
        [
            ("Market", "BTC-USD"),
            ("Action", "Sell"),
            ("Price", "100 → 101"),
            ("Total (broker's estimate)", "99"),
            ("Fee (broker's estimate)", "0.5"),
        ]
    );
}

#[test]
fn confirming_sends_the_change_against_the_order_that_was_reviewed() {
    let mut rig = Rig::new();
    rig.confirming();
    assert!(
        rig.venue.amends.borrow().is_empty(),
        "nothing is sent before approval"
    );
    rig.press(EditEvent::Confirm);
    let amends = rig.venue.amends.borrow();
    assert_eq!(amends.len(), 1);
    assert_eq!(amends[0].0.price, Some(d("101")));
    assert_eq!(
        amends[0].1,
        OrderFingerprint::of(rig.account.as_ref().unwrap(), &order())
    );
    assert!(rig.session.is_sending());
}

#[test]
fn a_change_the_broker_did_not_check_is_never_sent() {
    let mut rig = Rig::new();
    let mut unchecked = accepted(&order(), rig.account.as_ref().unwrap());
    unchecked.base = None;
    rig.venue.preview.replace(Some(Ok(unchecked)));
    rig.confirming();
    rig.press(EditEvent::Confirm);
    assert!(rig.venue.amends.borrow().is_empty());
    assert_eq!(
        rig.session.error(),
        Some("This change has not been checked.")
    );
    assert!(!rig.session.is_confirming());
}

#[test]
fn a_fill_or_an_edit_elsewhere_drops_the_review_and_sends_nothing() {
    let mut rig = Rig::new();
    rig.confirming();
    let mut filled = order();
    filled.filled_size = d("0.2");
    rig.live = Some(filled);
    assert!(matches!(rig.update(), Step::Open));
    assert!(!rig.session.is_confirming());
    assert!(rig.session.error().unwrap().contains("order changed"));
    assert_eq!(
        rig.session.filled_size(),
        d("0.2"),
        "the sheet shows the order as it is now"
    );
    assert!(rig.venue.amends.borrow().is_empty());
    // And it cannot be confirmed from the stale review.
    rig.press(EditEvent::Confirm);
    assert!(rig.venue.amends.borrow().is_empty());
}

#[test]
fn another_connection_or_account_drops_the_review() {
    let mut rig = Rig::new();
    rig.confirming();
    rig.generation = 2;
    rig.update();
    assert!(
        !rig.session.is_confirming(),
        "a preview is about one connection"
    );
    assert!(!rig.session.can_review());

    let mut rig = Rig::new();
    rig.confirming();
    rig.account = Some(AccountId::new("another"));
    rig.update();
    assert!(!rig.session.is_confirming(), "and about one account");
}

#[test]
fn a_review_whose_answer_has_gone_stale_goes_back_to_editing() {
    let mut rig = Rig::new();
    rig.confirming();
    rig.wait(PREVIEW_FRESH + Duration::from_secs(1));
    rig.update();
    assert!(!rig.session.is_confirming());
    assert!(rig.session.error().unwrap().contains("out of date"));
}

#[test]
fn the_sheet_closes_when_the_order_is_gone_or_the_broker_is() {
    let mut rig = Rig::new();
    rig.live = None;
    let notice = closed_with(rig.update()).unwrap();
    assert!(notice.error && notice.text.contains("no longer open"));

    let mut rig = Rig::new();
    let mut done = order();
    done.status = OrderStatus::Filled;
    rig.live = Some(done);
    assert!(
        closed_with(rig.update())
            .unwrap()
            .text
            .contains("no longer open")
    );

    let mut rig = Rig::new();
    rig.account = None;
    assert!(
        closed_with(rig.update())
            .unwrap()
            .text
            .contains("not connected")
    );

    let mut rig = Rig::new();
    let frame = EditFrame {
        now: rig.now,
        live: rig.live.as_ref(),
        account: rig.account.as_ref(),
        generation: 1,
        spec: None,
    };
    assert!(
        closed_with(rig.session.update(&frame, None)).is_none(),
        "no venue: closed without a word"
    );
}

#[test]
fn an_amend_in_flight_survives_the_order_leaving_the_book() {
    let mut rig = Rig::new();
    rig.confirming();
    rig.press(EditEvent::Confirm);
    // The broker applied it: the order is no longer the open one we sent against.
    rig.live = None;
    assert!(
        matches!(rig.update(), Step::Open),
        "the answer is still awaited"
    );

    let mut sent = Rig::new();
    sent.venue.amend.replace(Some(Ok(EditOutcome::Native)));
    sent.confirming();
    sent.press(EditEvent::Confirm);
    let notice = closed_with(sent.update()).unwrap();
    assert!(!notice.error && notice.text.starts_with("Edit sent."));
}

#[test]
fn a_failed_amend_returns_to_editing_with_the_reason_and_an_unknown_one_is_read_again() {
    let mut rig = Rig::new();
    rig.venue.amend.replace(Some(Err(EditError {
        text: "The price is too far from the market.".to_owned(),
        outcome_unknown: false,
    })));
    rig.confirming();
    rig.press(EditEvent::Confirm);
    assert!(matches!(rig.update(), Step::Open));
    assert!(!rig.session.is_sending());
    assert_eq!(
        rig.session.error(),
        Some("The price is too far from the market.")
    );

    let mut rig = Rig::new();
    rig.venue.amend.replace(Some(Err(EditError {
        text: "The connection dropped.".to_owned(),
        outcome_unknown: true,
    })));
    rig.confirming();
    rig.press(EditEvent::Confirm);
    let notice = closed_with(rig.update()).unwrap();
    assert!(notice.error);
    assert!(notice.text.contains("being read again"));
}

#[test]
fn back_and_cancel_leave_without_sending_and_cancel_waits_for_a_send_in_flight() {
    let mut rig = Rig::new();
    rig.confirming();
    rig.press(EditEvent::Back);
    assert!(!rig.session.is_confirming());
    assert!(rig.venue.amends.borrow().is_empty());
    assert!(closed_with(rig.press(EditEvent::Cancel)).is_none());

    let mut rig = Rig::new();
    rig.confirming();
    rig.press(EditEvent::Confirm);
    assert!(
        matches!(rig.press(EditEvent::Cancel), Step::Open),
        "an amend in flight cannot be abandoned"
    );
}

#[test]
fn a_stop_limit_edits_its_stop_price() {
    let mut stop = order();
    stop.order_type = OrderType::StopLimit;
    stop.stop_price = Some(d("95"));
    let now = Instant::now();
    let mut session = EditSession::new(stop.clone(), None, now);
    "96".clone_into(session.fields_mut().0);
    let venue = Venue::default();
    let account = AccountId::new("acct");
    let spec = spec();
    let frame = EditFrame {
        now,
        live: Some(&stop),
        account: Some(&account),
        generation: 1,
        spec: Some(&spec),
    };
    session.update(&frame, Some(&venue));
    let EditView::Editing(form) = session.view() else {
        panic!("editing")
    };
    assert_eq!(form.price_title, "Stop price");
    let change = session.computed.change.clone().unwrap();
    assert_eq!(change.stop_price, Some(d("96")));
    assert_eq!(change.price, None);
}
