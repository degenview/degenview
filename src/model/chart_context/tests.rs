use super::*;

fn context(price: Option<f64>) -> ChartContext {
    ChartContext {
        chart_id: 3,
        symbol: "BTCUSD".to_owned(),
        source: DataSource::Binance,
        time_ms: Some(1_700_000_000_000),
        price,
        target: Target::Background,
    }
}

fn items(entries: &[Entry]) -> Vec<&Item> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item),
            Entry::Separator | Entry::Heading(_) => None,
        })
        .collect()
}

fn find<'a>(entries: &'a [Entry], command: &Command) -> Option<&'a Item> {
    items(entries)
        .into_iter()
        .find(|item| item.command.as_ref() == Some(command))
}

fn labels(entries: &[Entry]) -> Vec<String> {
    items(entries)
        .into_iter()
        .map(|item| item.label.clone())
        .collect()
}

fn facts() -> Facts {
    Facts {
        last_price: Some(118_000.0),
        tick: 0.01,
        quantity: "0.5".to_owned(),
        drawings: 17,
        indicators: 5,
        clipboard: true,
        ..Facts::default()
    }
}

#[test]
fn orders_follow_the_side_of_the_market_the_price_is_on() {
    let above = suggest_orders(118_150.99, 118_000.0, 0.01).unwrap();
    assert_eq!(
        above,
        [
            QuickOrder {
                side: PaperOrderSide::Sell,
                kind: PaperOrderType::Limit
            },
            QuickOrder {
                side: PaperOrderSide::Buy,
                kind: PaperOrderType::Stop
            },
        ]
    );
    let below = suggest_orders(117_000.0, 118_000.0, 0.01).unwrap();
    assert_eq!(
        below,
        [
            QuickOrder {
                side: PaperOrderSide::Buy,
                kind: PaperOrderType::Limit
            },
            QuickOrder {
                side: PaperOrderSide::Sell,
                kind: PaperOrderType::Stop
            },
        ]
    );
}

#[test]
fn no_pending_order_is_suggested_at_the_market() {
    assert_eq!(suggest_orders(100.0, 100.0, 0.01), None);
    assert_eq!(suggest_orders(100.004, 100.0, 0.01), None);
    assert!(suggest_orders(100.02, 100.0, 0.01).is_some());
    assert_eq!(suggest_orders(f64::NAN, 100.0, 0.01), None);
    assert_eq!(suggest_orders(100.0, f64::INFINITY, 0.01), None);
    // With no known tick only an exact tie is "at the market".
    assert!(suggest_orders(100.000_1, 100.0, 0.0).is_some());
}

#[test]
fn an_alert_goes_the_way_the_price_must_travel() {
    assert_eq!(alert_side(110.0, Some(100.0)), Some(AlertSide::Above));
    assert_eq!(alert_side(90.0, Some(100.0)), Some(AlertSide::Below));
    assert_eq!(alert_side(100.0, Some(100.0)), None);
    assert_eq!(alert_side(100.0, None), None);
    assert_eq!(alert_side(f64::NAN, Some(1.0)), None);
}

#[test]
fn counts_read_singular_and_plural() {
    assert_eq!(remove_drawings_label(1), "Remove 1 drawing");
    assert_eq!(remove_drawings_label(17), "Remove 17 drawings");
    assert_eq!(remove_drawings_label(0), "Remove drawings");
    assert_eq!(remove_indicators_label(1), "Remove 1 indicator");
    assert_eq!(remove_indicators_label(5), "Remove 5 indicators");
}

#[test]
fn the_background_menu_carries_the_clicked_price_and_symbol() {
    let menu = menu(&context(Some(118_150.99)), &facts());
    let labels = labels(&menu);
    assert_eq!(labels[0], "Copy price 118,151");
    assert_eq!(labels[1], "Paste");
    assert_eq!(labels[2], "Add alert on BTCUSD at 118,151…");
    assert_eq!(labels[3], "Sell 0.5 BTCUSD @ 118,151 limit");
    assert_eq!(labels[4], "Buy 0.5 BTCUSD @ 118,151 stop");
    assert_eq!(labels[5], "Add order on BTCUSD at 118,151…");
    assert_eq!(labels[6], "Lock vertical cursor line by time");
    assert!(labels.contains(&"Remove 17 drawings".to_owned()));
    assert!(labels.contains(&"Remove 5 indicators".to_owned()));
    assert_eq!(labels.last().unwrap(), "Settings…");
}

#[test]
fn below_the_market_the_entries_flip() {
    let menu = menu(&context(Some(117_000.0)), &facts());
    let labels = labels(&menu);
    assert_eq!(labels[3], "Buy 0.5 BTCUSD @ 117,000 limit");
    assert_eq!(labels[4], "Sell 0.5 BTCUSD @ 117,000 stop");
}

#[test]
fn quantity_is_never_invented() {
    let mut facts = facts();
    facts.quantity.clear();
    let menu = menu(&context(Some(118_150.0)), &facts);
    assert_eq!(labels(&menu)[3], "Sell BTCUSD @ 118,150 limit");
}

#[test]
fn groups_are_separated_and_never_doubled() {
    let menu = menu(&context(Some(118_150.0)), &facts());
    assert!(!matches!(menu.first(), Some(Entry::Separator)));
    assert!(!matches!(menu.last(), Some(Entry::Separator)));
    assert!(
        menu.windows(2)
            .all(|pair| !matches!(pair, [Entry::Separator, Entry::Separator]))
    );
    // Clipboard | trading | cursor | utilities | cleanup | settings.
    let separators = menu
        .iter()
        .filter(|entry| matches!(entry, Entry::Separator))
        .count();
    assert_eq!(separators, 5);
}

#[test]
fn unsupported_sources_drop_whole_groups() {
    let mut facts = facts();
    facts.alerts_supported = false;
    facts.paper = PaperAccess::Unsupported;
    let menu = menu(&context(Some(1.0)), &facts);
    let labels = labels(&menu);
    assert!(!labels.iter().any(|l| l.starts_with("Add alert")));
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("limit") || l.starts_with("Add order"))
    );
    assert!(labels.contains(&"Table view".to_owned()));
    assert!(labels.contains(&"Settings…".to_owned()));
}

#[test]
fn a_missing_price_disables_what_needs_one() {
    let menu = menu(&context(None), &facts());
    for command in [Command::CopyPrice, Command::AddAlert, Command::AddOrder] {
        let item = find(&menu, &command).unwrap();
        assert!(!item.is_enabled(), "{command:?}");
    }
    assert_eq!(
        find(&menu, &Command::CopyPrice).unwrap().label,
        "Copy price"
    );
    // Settings and the panels do not need a price.
    assert!(find(&menu, &Command::OpenSettings).unwrap().is_enabled());
    assert!(find(&menu, &Command::ToggleTable).unwrap().is_enabled());
}

#[test]
fn a_non_finite_price_is_not_a_price() {
    let menu = menu(&context(Some(f64::NAN)), &facts());
    assert!(!find(&menu, &Command::CopyPrice).unwrap().is_enabled());
}

#[test]
fn paste_is_disabled_without_a_copy_and_shows_its_shortcut() {
    let mut facts = facts();
    facts.clipboard = false;
    let menu = menu(&context(Some(1.0)), &facts);
    let paste = find(&menu, &Command::Paste).unwrap();
    assert!(!paste.is_enabled());
    assert_eq!(paste.hint, Some(Hint::Paste));
}

#[test]
fn replay_disables_alerts_and_orders_with_a_reason() {
    let mut facts = facts();
    facts.replay = true;
    let menu = menu(&context(Some(118_150.0)), &facts);
    let add_alert = find(&menu, &Command::AddAlert).unwrap();
    assert_eq!(add_alert.disabled, Some(IN_REPLAY));
    let add_order = find(&menu, &Command::AddOrder).unwrap();
    assert_eq!(add_order.disabled, Some(IN_REPLAY));
    // Copying a price and the chart panels still work in replay.
    assert!(find(&menu, &Command::CopyPrice).unwrap().is_enabled());
}

#[test]
fn a_price_at_the_market_greys_the_pending_orders_with_a_reason() {
    let menu = menu(&context(Some(118_000.0)), &facts());
    let pending: Vec<&Item> = items(&menu)
        .into_iter()
        .filter(|item| matches!(item.command, Some(Command::QuickOrder(_))))
        .collect();
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().all(|item| item.disabled == Some(NO_MARKET)));
    // The full ticket is still reachable at the market price.
    assert!(find(&menu, &Command::AddOrder).unwrap().is_enabled());
}

#[test]
fn cleanup_entries_disable_when_there_is_nothing_to_remove() {
    let mut facts = facts();
    facts.drawings = 0;
    facts.indicators = 0;
    let menu = menu(&context(Some(1.0)), &facts);
    let drawings = find(&menu, &Command::RemoveDrawings).unwrap();
    assert!(!drawings.is_enabled());
    assert_eq!(drawings.label, "Remove drawings");
    assert!(find(&menu, &Command::RemoveIndicators).unwrap().destructive);
}

#[test]
fn the_lock_entry_toggles_its_label() {
    let mut facts = facts();
    let unlocked = menu(&context(Some(1.0)), &facts);
    assert!(labels(&unlocked).contains(&"Lock vertical cursor line by time".to_owned()));
    facts.time_locked = true;
    let locked = menu(&context(Some(1.0)), &facts);
    assert!(labels(&locked).contains(&"Unlock vertical cursor line".to_owned()));
}

#[test]
fn panels_show_whether_they_are_open() {
    let mut facts = facts();
    facts.table_open = true;
    let menu = menu(&context(Some(1.0)), &facts);
    assert!(find(&menu, &Command::ToggleTable).unwrap().checked);
    assert!(!find(&menu, &Command::ToggleObjectTree).unwrap().checked);
}

fn template(id: u64, name: &str, built_in: bool, current: bool) -> TemplateRow {
    TemplateRow {
        id,
        name: name.to_owned(),
        built_in,
        current,
    }
}

fn submenu_of(facts: &Facts) -> Item {
    let menu = menu(&context(Some(1.0)), facts);
    items(&menu)
        .into_iter()
        .find(|item| item.label == "Chart template")
        .cloned()
        .expect("the template submenu")
}

fn headings(entries: &[Entry]) -> Vec<&str> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Heading(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_template_submenu_groups_my_templates_presets_and_actions() {
    let mut facts = facts();
    facts.templates = vec![
        template(1, "Classic", true, false),
        template(2, "Midnight", true, false),
        template(1_001, "Mine", false, false),
    ];
    let submenu = submenu_of(&facts);
    assert_eq!(submenu.command, None);
    assert_eq!(submenu.glyph, Some(Glyph::Template));
    assert_eq!(headings(&submenu.children), ["My templates", "Presets"]);
    assert_eq!(
        labels(&submenu.children),
        [
            "Mine",
            "Classic",
            "Midnight",
            "Save current as template…",
            "Manage templates…",
            "Reset to default"
        ],
        "saved templates come before the presets, then the actions"
    );
    assert!(find(&submenu.children, &Command::ApplyTemplate(1_001)).is_some());
    // Each group is separated, none twice in a row, and none leads or trails.
    assert!(!matches!(submenu.children.first(), Some(Entry::Separator)));
    assert!(!matches!(submenu.children.last(), Some(Entry::Separator)));
    assert!(
        submenu
            .children
            .windows(2)
            .all(|pair| !matches!(pair, [Entry::Separator, Entry::Separator]))
    );
}

#[test]
fn with_nothing_saved_the_submenu_says_so_and_still_offers_the_presets() {
    let mut facts = facts();
    facts.templates = vec![template(1, "Classic", true, false)];
    let submenu = submenu_of(&facts);
    let none = items(&submenu.children)
        .into_iter()
        .find(|item| item.label == "None saved yet")
        .expect("a placeholder");
    assert!(!none.is_enabled());
    assert_eq!(none.command, None, "a placeholder does nothing");
    assert!(find(&submenu.children, &Command::ApplyTemplate(1)).is_some());
    assert!(find(&submenu.children, &Command::SaveTemplate).is_some());
}

#[test]
fn the_submenu_works_with_no_templates_at_all() {
    let submenu = submenu_of(&facts());
    assert_eq!(headings(&submenu.children), ["My templates"]);
    assert!(find(&submenu.children, &Command::SaveTemplate).is_some());
    assert!(find(&submenu.children, &Command::ManageTemplates).is_some());
    assert!(find(&submenu.children, &Command::ResetTemplate).is_some());
}

#[test]
fn the_template_that_matches_the_chart_is_marked_current() {
    let mut facts = facts();
    facts.templates = vec![
        template(1, "Classic", true, true),
        template(1_001, "Mine", false, false),
    ];
    let submenu = submenu_of(&facts);
    let apply = |id| find(&submenu.children, &Command::ApplyTemplate(id)).unwrap();
    assert!(apply(1).current);
    assert!(!apply(1_001).current);
    // Marking it does not disable it: applying the look you have is harmless.
    assert!(apply(1).is_enabled());
}

#[test]
fn axes_get_their_own_menus_not_the_background_one() {
    let mut price = context(Some(10.0));
    price.target = Target::PriceAxis;
    let mut facts = facts();
    facts.scale_mode = PriceScaleMode::Percent;
    let menu_price = menu(&price, &facts);
    assert_eq!(
        labels(&menu_price),
        [
            "Reset price scale",
            "Regular",
            "Percent",
            "Indexed to 100",
            "Logarithmic"
        ]
    );
    // One mode is marked, the active one, and each row sets exactly its own.
    let marked: Vec<_> = PriceScaleMode::ALL
        .into_iter()
        .filter(|mode| {
            find(&menu_price, &Command::SetScaleMode(*mode))
                .unwrap()
                .checked
        })
        .collect();
    assert_eq!(marked, [PriceScaleMode::Percent]);

    let mut time = context(None);
    time.target = Target::TimeAxis;
    assert_eq!(labels(&super::menu(&time, &facts)), ["Reset chart view"]);
}

#[test]
fn a_script_pane_menu_acts_on_its_instance() {
    let mut pane = context(None);
    pane.target = Target::Pane { instance: 4 };
    let mut facts = facts();
    facts.pane = Some(PaneFacts {
        name: "RSI".to_owned(),
        visible: true,
        order: VisualOrder::Front,
    });
    let menu = menu(&pane, &facts);
    assert!(find(&menu, &Command::PaneSettings(4)).is_some());
    assert_eq!(
        find(&menu, &Command::PineLogs(4)).unwrap().label,
        "Pine logs"
    );
    assert_eq!(find(&menu, &Command::PaneToggle(4)).unwrap().label, "Hide");
    let remove = find(&menu, &Command::PaneRemove(4)).unwrap();
    assert!(remove.destructive);
    let order = items(&menu)
        .into_iter()
        .find(|item| item.label == "Visual order")
        .unwrap();
    assert!(
        find(&order.children, &Command::PaneOrder(4, VisualOrder::Front))
            .unwrap()
            .checked
    );
    // A pane that has gone offers nothing rather than a dead menu.
    facts.pane = None;
    assert!(super::menu(&pane, &facts).is_empty());
}

#[test]
fn an_alert_line_menu_reflects_its_state() {
    let mut line = context(Some(5.0));
    line.target = Target::Alert { id: 9 };
    let mut facts = facts();
    facts.alert = Some(AlertState::Active);
    let menu_active = menu(&line, &facts);
    assert_eq!(
        find(&menu_active, &Command::ToggleAlert(9)).unwrap().label,
        "Pause alert"
    );
    assert_eq!(
        find(&menu_active, &Command::ToggleAlert(9)).unwrap().glyph,
        Some(Glyph::Pause)
    );
    facts.alert = Some(AlertState::Paused);
    let menu_paused = menu(&line, &facts);
    assert_eq!(
        find(&menu_paused, &Command::ToggleAlert(9)).unwrap().label,
        "Resume alert"
    );
    assert_eq!(
        find(&menu_paused, &Command::ToggleAlert(9)).unwrap().glyph,
        Some(Glyph::Play)
    );
    assert!(
        find(&menu_paused, &Command::DeleteAlert(9))
            .unwrap()
            .destructive
    );
    // A triggered alert can be edited or deleted but neither paused nor resumed.
    facts.alert = Some(AlertState::Triggered);
    let menu_done = menu(&line, &facts);
    assert!(find(&menu_done, &Command::ToggleAlert(9)).is_none());
    assert!(find(&menu_done, &Command::EditAlert(9)).is_some());
}

#[test]
fn a_filled_order_cannot_be_cancelled() {
    let mut pill = context(Some(5.0));
    pill.target = Target::Order { id: 2 };
    let mut facts = facts();
    facts.order = Some(OrderFacts { cancelable: false });
    let menu = menu(&pill, &facts);
    assert!(!find(&menu, &Command::CancelOrder(2)).unwrap().is_enabled());
    facts.order = Some(OrderFacts { cancelable: true });
    assert!(
        find(&super::menu(&pill, &facts), &Command::CancelOrder(2))
            .unwrap()
            .is_enabled()
    );
}

#[test]
fn a_position_menu_closes_that_position() {
    let mut pill = context(Some(5.0));
    pill.target = Target::Position {
        key: "binance:BTCUSD".to_owned(),
    };
    let menu = menu(&pill, &facts());
    assert!(find(&menu, &Command::ClosePosition("binance:BTCUSD".to_owned())).is_some());
}

fn live() -> Live {
    Live {
        chart: Some(("BTCUSD".to_owned(), DataSource::Binance)),
        instances: vec![4],
        alerts: vec![9],
        orders: vec![2],
        positions: vec!["k".to_owned()],
    }
}

#[test]
fn the_snapshot_is_valid_while_its_chart_and_target_stand() {
    assert_eq!(check(&context(Some(1.0)), &live()), Ok(()));
    let mut pane = context(None);
    pane.target = Target::Pane { instance: 4 };
    assert_eq!(check(&pane, &live()), Ok(()));
}

#[test]
fn a_closed_chart_fails_safely() {
    let live = Live {
        chart: None,
        ..live()
    };
    assert_eq!(check(&context(Some(1.0)), &live), Err(Stale::ChartClosed));
}

#[test]
fn a_chart_that_switched_market_is_stale() {
    let mut other = live();
    other.chart = Some(("ETHUSD".to_owned(), DataSource::Binance));
    assert_eq!(check(&context(Some(1.0)), &other), Err(Stale::ChartChanged));
    other.chart = Some(("BTCUSD".to_owned(), DataSource::Coinbase));
    assert_eq!(check(&context(Some(1.0)), &other), Err(Stale::ChartChanged));
}

#[test]
fn a_removed_target_is_stale() {
    let mut context = context(None);
    for target in [
        Target::Pane { instance: 5 },
        Target::Alert { id: 1 },
        Target::Order { id: 3 },
        Target::Position {
            key: "gone".to_owned(),
        },
    ] {
        context.target = target;
        assert_eq!(check(&context, &live()), Err(Stale::TargetGone));
    }
}

#[test]
fn stale_messages_are_actionable_sentences() {
    for stale in [Stale::ChartClosed, Stale::ChartChanged, Stale::TargetGone] {
        assert!(stale.message().ends_with('.'));
    }
}
