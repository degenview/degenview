//! What a spot account is worth, as far as the prices at hand say.
//!
//! A broker account is a list of balances. Its value is each balance times a price in one chosen
//! currency; an asset with no price is left out and counted, never guessed, so the figure is an
//! estimate that says what it leaves out. Pure functions over [`Decimal`]: the adapter supplies the
//! balances and a lookup for prices.

use degenview_brokers::Decimal;

/// Cash assets, in the order they are preferred when two hold the same amount.
const CASH: [&str; 6] = ["USD", "USDC", "EUR", "GBP", "USDT", "DAI"];

/// One asset held: what can be used and what is committed to open orders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Holding {
    pub asset: String,
    pub available: Decimal,
    pub held: Decimal,
}

impl Holding {
    fn total(&self) -> Decimal {
        self.available + self.held
    }
}

/// The account's value in one currency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Valuation {
    /// The currency every figure is in.
    pub currency: String,
    /// Everything that could be priced, cash included.
    pub equity: Decimal,
    /// The valuation currency that is free to spend.
    pub cash: Decimal,
    /// Priced value committed to open orders.
    pub on_hold: Decimal,
    /// Priced value of everything that is not the valuation currency.
    pub holdings: Decimal,
    /// Assets held with no price, sorted.
    pub unpriced: Vec<String>,
    /// How many assets with a balance there are, the unpriced included.
    pub assets: usize,
}

/// The currency to value in: the cash asset with the most of it, `USD` when there is none.
pub fn valuation_currency(holdings: &[Holding]) -> String {
    CASH.iter()
        .filter_map(|code| {
            let total = holdings
                .iter()
                .find(|holding| holding.asset == *code)
                .map(Holding::total)?;
            (total > Decimal::ZERO).then_some((*code, total))
        })
        // `max_by` keeps the last of equals; the list is walked in order of preference, so walk it
        // backwards to keep the first.
        .rev()
        .max_by(|a, b| a.1.cmp(&b.1))
        .map_or_else(|| "USD".to_owned(), |(code, _)| code.to_owned())
}

/// Values `holdings` in `currency`. `price` gives the price of one unit of an asset in `currency`.
pub fn value(
    holdings: &[Holding],
    currency: &str,
    price: impl Fn(&str) -> Option<Decimal>,
) -> Valuation {
    let mut valuation = Valuation {
        currency: currency.to_owned(),
        equity: Decimal::ZERO,
        cash: Decimal::ZERO,
        on_hold: Decimal::ZERO,
        holdings: Decimal::ZERO,
        unpriced: Vec::new(),
        assets: 0,
    };
    for holding in holdings.iter().filter(|h| h.total() > Decimal::ZERO) {
        valuation.assets += 1;
        let unit = if holding.asset == currency {
            Some(Decimal::ONE)
        } else {
            price(&holding.asset).filter(|p| *p > Decimal::ZERO)
        };
        let Some(unit) = unit else {
            valuation.unpriced.push(holding.asset.clone());
            continue;
        };
        valuation.equity += holding.total() * unit;
        valuation.on_hold += holding.held * unit;
        if holding.asset == currency {
            valuation.cash += holding.available;
        } else {
            valuation.holdings += holding.total() * unit;
        }
    }
    valuation.unpriced.sort();
    valuation
}

/// `1,234.56` with the currency's mark: `$` and `€` and `£` lead, anything else follows as a code.
pub fn money(value: Decimal, currency: &str) -> String {
    let rounded = value.round_dp(2);
    let text = format!("{:.2}", rounded.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, "00"));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if rounded.is_sign_negative() && !rounded.is_zero() {
        "-"
    } else {
        ""
    };
    match currency {
        "USD" => format!("{sign}${grouped}.{fraction}"),
        "EUR" => format!("{sign}€{grouped}.{fraction}"),
        "GBP" => format!("{sign}£{grouped}.{fraction}"),
        code => format!("{sign}{grouped}.{fraction} {code}"),
    }
}

/// A fee rate as a percent: `0.006` is `0.60%`.
pub fn percent(rate: Decimal) -> String {
    format!("{:.2}%", (rate * Decimal::ONE_HUNDRED).round_dp(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn holding(asset: &str, available: &str, held: &str) -> Holding {
        Holding {
            asset: asset.to_owned(),
            available: d(available),
            held: d(held),
        }
    }

    fn prices(asset: &str) -> Option<Decimal> {
        match asset {
            "BTC" => Some(d("60000")),
            "ETH" => Some(d("3000")),
            _ => None,
        }
    }

    #[test]
    fn the_currency_is_the_cash_asset_with_the_most_of_it() {
        let held = [
            holding("USD", "100", "0"),
            holding("EUR", "900", "50"),
            holding("BTC", "5", "0"),
        ];
        assert_eq!(valuation_currency(&held), "EUR");
        assert_eq!(valuation_currency(&[holding("BTC", "1", "0")]), "USD");
        assert_eq!(valuation_currency(&[]), "USD");
        // A tie goes to the one listed first.
        let tie = [holding("USDC", "10", "0"), holding("USD", "10", "0")];
        assert_eq!(valuation_currency(&tie), "USD");
    }

    #[test]
    fn value_is_each_balance_at_its_price_and_cash_counts_one_to_one() {
        let held = [
            holding("USD", "1000", "200"),
            holding("BTC", "0.5", "0.1"),
            holding("ETH", "2", "0"),
        ];
        let valuation = value(&held, "USD", prices);
        // 1200 cash + 0.6 BTC (36000) + 2 ETH (6000)
        assert_eq!(valuation.equity, d("43200"));
        assert_eq!(valuation.cash, d("1000"));
        assert_eq!(valuation.holdings, d("42000"));
        // 200 USD and 0.1 BTC (6000) are committed to orders.
        assert_eq!(valuation.on_hold, d("6200"));
        assert_eq!(valuation.assets, 3);
        assert!(valuation.unpriced.is_empty());
    }

    #[test]
    fn an_asset_without_a_price_is_left_out_and_counted() {
        let held = [
            holding("USD", "10", "0"),
            holding("BONK", "5000000", "0"),
            holding("ADA", "100", "0"),
            holding("BTC", "0", "0"),
        ];
        let valuation = value(&held, "USD", prices);
        assert_eq!(valuation.equity, d("10"));
        assert_eq!(valuation.unpriced, ["ADA", "BONK"]);
        // An empty balance is not an asset held.
        assert_eq!(valuation.assets, 3);
    }

    #[test]
    fn a_zero_or_negative_price_is_no_price() {
        let held = [holding("BTC", "1", "0")];
        assert_eq!(value(&held, "USD", |_| Some(d("0"))).unpriced, ["BTC"]);
        assert_eq!(value(&held, "USD", |_| Some(d("-1"))).unpriced, ["BTC"]);
    }

    #[test]
    fn money_groups_thousands_and_marks_the_currency() {
        assert_eq!(money(d("1234567.891"), "USD"), "$1,234,567.89");
        assert_eq!(money(d("0.5"), "EUR"), "€0.50");
        assert_eq!(money(d("-12.3"), "GBP"), "-£12.30");
        assert_eq!(money(d("1000"), "USDC"), "1,000.00 USDC");
        assert_eq!(money(d("-0.001"), "USD"), "$0.00");
    }

    #[test]
    fn a_fee_rate_is_a_percent() {
        assert_eq!(percent(d("0.006")), "0.60%");
        assert_eq!(percent(d("0.0035")), "0.35%");
        assert_eq!(percent(d("0")), "0.00%");
    }
}
