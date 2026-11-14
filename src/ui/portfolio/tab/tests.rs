use super::*;
use crate::store::{SharedStore, Store};
use std::sync::{Arc, Mutex};

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
}

/// A host with one portfolio per entry of `archived`, and "All Portfolios" selected.
fn host_with(archived: &[bool]) -> (PortfolioHost, Vec<u64>) {
    let mut host = PortfolioHost::load(store());
    let ids: Vec<u64> = archived
        .iter()
        .enumerate()
        .map(|(index, _)| {
            host.create_portfolio(&format!("Portfolio {index}"), PortfolioCurrency::Usd)
                .expect("create")
        })
        .collect();
    for (id, archived) in ids.iter().zip(archived) {
        let mut portfolio = host
            .snapshot()
            .portfolios
            .iter()
            .find(|portfolio| portfolio.id == *id)
            .cloned()
            .expect("portfolio");
        portfolio.is_archived = *archived;
        host.update_portfolio(portfolio).expect("update");
    }
    host.select(None).expect("select all");
    (host, ids)
}

#[test]
fn the_header_is_hidden_only_when_there_are_no_portfolios() {
    let (host, _) = host_with(&[]);
    assert!(!shows_header(host.snapshot()));

    let (host, _) = host_with(&[true, true]);
    assert!(
        shows_header(host.snapshot()),
        "all archived: the picker is the way back"
    );

    let (host, _) = host_with(&[false]);
    assert!(shows_header(host.snapshot()));
}

#[test]
fn transaction_controls_need_a_portfolio_in_scope() {
    let (host, _) = host_with(&[]);
    assert!(!has_portfolio_scope(host.snapshot()), "no portfolios");

    let (host, _) = host_with(&[true, true]);
    assert!(
        !has_portfolio_scope(host.snapshot()),
        "everything archived and All selected is the empty page"
    );

    let (host, _) = host_with(&[false]);
    assert!(has_portfolio_scope(host.snapshot()));

    let (host, _) = host_with(&[true, false]);
    assert!(has_portfolio_scope(host.snapshot()), "one active is enough");

    let (mut host, ids) = host_with(&[true, true]);
    host.select(Some(ids[0])).expect("select archived");
    assert!(
        has_portfolio_scope(host.snapshot()),
        "a selected portfolio is in scope even when archived"
    );
}

#[test]
fn a_cmc_import_lands_in_the_selected_portfolio_only() {
    let (mut host, ids) = host_with(&[false, false]);
    assert_eq!(cmc_destination(host.snapshot()), None, "All Portfolios");
    host.select(Some(ids[1])).expect("select");
    assert_eq!(
        cmc_destination(host.snapshot()),
        Some((ids[1], PortfolioCurrency::Usd))
    );
}
