use super::*;

#[test]
fn every_venue_has_a_label() {
    let labels: Vec<String> = Venue::ALL.iter().map(|venue| venue.label()).collect();
    assert_eq!(
        labels,
        ["Binance spot", "Binance perp", "Bybit spot", "Bybit perp"]
    );
}
