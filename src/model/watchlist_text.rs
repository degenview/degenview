//! Plain-text watchlist exchange.
//!
//! One entry per line, `SOURCE:SYMBOL`, with `###Title` lines for sections:
//!
//! ```text
//! ###Majors
//! BINANCE:BTCUSDT
//! COINBASE:ETH-USD
//! DEXSCREENER:solana/<pair address>
//! POLYMARKET:<token id>|Will it happen?
//! ```
//!
//! A single line of comma-separated entries, as TradingView exports, is read too.
//! TradingView compatibility is best effort: exchange prefixes DegenView has no
//! provider for are reported as skipped, never guessed at.

use crate::model::watchlist::{
    MAX_NAME_CHARS, Watchlist, WatchlistEntry, WatchlistError, WatchlistInstrument,
    WatchlistSection,
};
use crate::model::{DataSource, InstrumentId};

/// US listings on these venues map to Alpaca, which serves US equities.
const US_EXCHANGES: [&str; 7] = [
    "NASDAQ",
    "NYSE",
    "AMEX",
    "ARCA",
    "NYSEARCA",
    "BATS",
    "NYSEAMERICAN",
];

const SOURCES: [DataSource; 8] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
    DataSource::Alpaca,
    DataSource::Polymarket,
    DataSource::Kalshi,
    DataSource::CoinMarketCap,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Row {
    Section(String),
    Instrument(WatchlistInstrument),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Skipped {
    /// 1-based line (or comma-separated item) in the pasted text.
    pub line: usize,
    pub text: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Parsed {
    pub rows: Vec<Row>,
    pub skipped: Vec<Skipped>,
    /// The text was longer than `MAX_IMPORT_BYTES` or `MAX_IMPORT_LINES`; the rest was left unread.
    pub truncated: bool,
}

/// The most text one import reads. A list holds at most 5,000 items, a line is short.
pub const MAX_IMPORT_BYTES: usize = 1 << 20;
/// The most lines (or comma-separated items) one import reads.
pub const MAX_IMPORT_LINES: usize = 10_000;
/// Skipped lines kept in a report; the rest are only counted.
const MAX_SKIPPED_SHOWN: usize = 200;

/// What an import did, shown once so nothing is dropped silently.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportReport {
    pub added: usize,
    pub duplicates: usize,
    pub sections_added: usize,
    pub skipped: Vec<Skipped>,
    /// Lines skipped beyond the `skipped` kept for display.
    pub skipped_not_shown: usize,
    /// Some of the text was not imported: it was too long, or the list filled up.
    pub truncated: bool,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let plural = |count: usize| if count == 1 { "" } else { "s" };
        let mut parts = vec![format!("Added {} symbol{}", self.added, plural(self.added))];
        if self.sections_added > 0 {
            parts.push(format!(
                "{} section{}",
                self.sections_added,
                plural(self.sections_added)
            ));
        }
        if self.duplicates > 0 {
            parts.push(format!("{} already in the list", self.duplicates));
        }
        let skipped = self.skipped.len() + self.skipped_not_shown;
        if skipped > 0 {
            parts.push(format!("{skipped} skipped"));
        }
        if self.truncated {
            parts.push("stopped early: too much to import".to_owned());
        }
        parts.join(", ")
    }
}

// MARK: Export

/// A file name to suggest for exporting the list `name`: path separators, drive colons and
/// control characters become spaces, so the save dialog never opens on a stray folder or an
/// unusable name. The list keeps its own name.
pub fn export_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_start_matches('.').trim();
    let stem: String = cleaned.chars().take(100).collect();
    if stem.is_empty() {
        "Watchlist.txt".to_owned()
    } else {
        format!("{stem}.txt")
    }
}

pub fn export(list: &Watchlist) -> String {
    list.entries
        .iter()
        // A portfolio card is not a market, so it has no line to import back.
        .filter(|entry| !matches!(entry, WatchlistEntry::Instrument(item) if item.card.is_some()))
        .map(|entry| match entry {
            WatchlistEntry::Section(section) => format!("###{}", section.title),
            WatchlistEntry::Instrument(item) => line_for(item),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_for(item: &WatchlistInstrument) -> String {
    let id = &item.instrument;
    let qualified = format!("{}:{}", id.source.watchlist_slug(), id.symbol);
    if id.source.is_prediction_market() {
        let title = item
            .display_name
            .as_deref()
            .unwrap_or(&item.name)
            .replace('\n', " ");
        format!("{qualified}|{title}")
    } else {
        qualified
    }
}

// MARK: Import

pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut text = text;
    if text.len() > MAX_IMPORT_BYTES {
        let mut end = MAX_IMPORT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text = &text[..end];
        parsed.truncated = true;
    }
    // One line holding commas is a comma-separated list; otherwise one item per line.
    let comma_list = text.contains(',')
        && text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(2)
            .count()
            == 1;
    let tokens: Box<dyn Iterator<Item = &str>> = if comma_list {
        Box::new(text.split(','))
    } else {
        Box::new(text.lines())
    };
    for (offset, raw) in tokens.enumerate() {
        if offset >= MAX_IMPORT_LINES {
            parsed.truncated = true;
            break;
        }
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(title) = token.strip_prefix("###") {
            let title = Watchlist::sanitized_name(title);
            parsed.rows.push(Row::Section(
                if title.is_empty() { "Section" } else { &title }.to_owned(),
            ));
            continue;
        }
        match instrument_from(token) {
            Ok(item) => parsed.rows.push(Row::Instrument(item)),
            Err(reason) => parsed.skipped.push(Skipped {
                line: offset + 1,
                text: token.chars().take(MAX_NAME_CHARS).collect(),
                reason,
            }),
        }
    }
    parsed
}

/// The longest symbol a caller may name; real ones (a DEX address) are well under this.
const MAX_SYMBOL_LEN: usize = 128;

/// Checks that `symbol` is shaped like a market on `source` (the import rules) and returns its
/// normalised identity. The shape is all it can check: whether the market exists is only learned
/// when the chart fetches. Prediction markets and CoinMarketCap are not chartable by symbol alone.
pub fn validate_market(source: DataSource, symbol: &str) -> Result<InstrumentId, String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err("Missing symbol".to_owned());
    }
    if symbol.chars().count() > MAX_SYMBOL_LEN
        || symbol
            .chars()
            .any(|c| c.is_control() || c == '|' || c == ':')
    {
        return Err("Symbol contains unsupported characters".to_owned());
    }
    if source.is_prediction_market() {
        return Err("Prediction markets need their event details; add them from search".to_owned());
    }
    instrument_from(&format!("{}:{symbol}", source.watchlist_slug())).map(|item| item.instrument)
}

fn slug_source(prefix: &str) -> Option<DataSource> {
    SOURCES
        .into_iter()
        .find(|source| source.watchlist_slug() == prefix)
}

fn instrument_from(token: &str) -> Result<WatchlistInstrument, String> {
    let Some((prefix, rest)) = token.split_once(':') else {
        return Err("Missing exchange prefix, expected SOURCE:SYMBOL".to_owned());
    };
    let prefix = prefix.trim().to_uppercase();
    let mut rest = rest.trim().to_owned();
    let mut title = None;
    if let Some((symbol, candidate)) = rest.clone().split_once('|') {
        let candidate = Watchlist::sanitized_name(candidate);
        title = (!candidate.is_empty()).then_some(candidate);
        rest = symbol.trim().to_owned();
    }
    if rest.is_empty() {
        return Err("Missing symbol".to_owned());
    }

    if US_EXCHANGES.contains(&prefix.as_str()) {
        return stock(&rest);
    }
    let Some(source) = slug_source(&prefix) else {
        return Err(format!("Unsupported exchange {prefix}"));
    };

    match source {
        DataSource::Binance => {
            let symbol = rest.to_uppercase();
            if !(3..=20).contains(&symbol.len()) || !symbol.chars().all(is_upper_alphanumeric) {
                return Err(
                    "Not a Binance spot symbol (perpetuals and futures aren't supported)"
                        .to_owned(),
                );
            }
            Ok(item(source, &symbol, &symbol, &symbol))
        }
        DataSource::Coinbase => {
            let symbol = rest.to_uppercase();
            let valid = symbol.split_once('-').is_some_and(|(base, quote)| {
                [base, quote]
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(is_upper_alphanumeric))
            });
            if !valid {
                return Err("Coinbase products are written BASE-QUOTE, like BTC-USD".to_owned());
            }
            Ok(item(source, &symbol, &symbol.replace('-', "/"), &symbol))
        }
        DataSource::Alpaca => stock(&rest),
        DataSource::CoinGecko => {
            let id = rest.to_lowercase();
            let name = id
                .split('-')
                .filter(|word| !word.is_empty())
                .map(capitalized)
                .collect::<Vec<_>>()
                .join(" ");
            Ok(item(source, &id, &name, &id.to_uppercase()))
        }
        DataSource::DexScreener => {
            // `chain/address`: the address alone is the identity; the chain is looked up.
            let address = match rest.split_once('/') {
                Some((chain, address))
                    if !address.is_empty()
                        && !chain.is_empty()
                        && chain.chars().all(|c| {
                            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'
                        }) =>
                {
                    address.to_owned()
                }
                _ => rest.clone(),
            };
            Ok(item(source, &address, &address, &address))
        }
        DataSource::Polymarket | DataSource::Kalshi => {
            let Some(title) = title else {
                return Err(
                    "Prediction markets need a title after |; add them from search instead"
                        .to_owned(),
                );
            };
            let mut found = WatchlistInstrument::new(
                InstrumentId::new(source, rest.clone()),
                title.clone(),
                "",
            );
            found.display_name = Some(title);
            Ok(found)
        }
        DataSource::CoinMarketCap => Err("CoinMarketCap index charts aren't markets".to_owned()),
    }
}

fn is_upper_alphanumeric(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn stock(symbol: &str) -> Result<WatchlistInstrument, String> {
    let upper = symbol.to_uppercase();
    // 1-5 letters, optionally followed by `.` or `-` and one letter (BRK.B).
    let (head, class) = match upper.split_once(['.', '-']) {
        Some((head, class)) => (head, Some(class)),
        None => (upper.as_str(), None),
    };
    let valid = (1..=5).contains(&head.len())
        && head.chars().all(|c| c.is_ascii_uppercase())
        && class
            .is_none_or(|class| class.len() == 1 && class.chars().all(|c| c.is_ascii_uppercase()));
    if !valid {
        return Err("Not a US stock ticker".to_owned());
    }
    Ok(item(DataSource::Alpaca, &upper, &upper, &upper))
}

fn item(source: DataSource, symbol: &str, name: &str, label: &str) -> WatchlistInstrument {
    WatchlistInstrument::new(InstrumentId::new(source, symbol), name, label)
}

/// Adds every valid row of `parsed` to `list`. Sections are reused by title, instruments
/// already present are counted, bad rows are reported and never block good ones.
pub fn import_into(list: &mut Watchlist, parsed: &Parsed) -> Result<ImportReport, WatchlistError> {
    let mut skipped = parsed.skipped.clone();
    let not_shown = skipped.len().saturating_sub(MAX_SKIPPED_SHOWN);
    skipped.truncate(MAX_SKIPPED_SHOWN);
    let mut report = ImportReport {
        skipped,
        skipped_not_shown: not_shown,
        truncated: parsed.truncated,
        ..ImportReport::default()
    };
    let mut current_section = None;
    let mut keys: std::collections::HashSet<String> = list
        .instruments()
        .map(|item| item.instrument.key())
        .collect();
    for row in &parsed.rows {
        let outcome = match row {
            Row::Section(title) => {
                let existing: Option<&WatchlistSection> = list
                    .sections()
                    .find(|section| section.title.to_lowercase() == title.to_lowercase());
                if let Some(existing) = existing {
                    current_section = Some(existing.id);
                    Ok(())
                } else {
                    list.add_section(title, None).map(|section| {
                        current_section = Some(section.id);
                        report.sections_added += 1;
                    })
                }
            }
            Row::Instrument(item) => {
                if !keys.insert(item.instrument.key()) {
                    report.duplicates += 1;
                    Ok(())
                } else {
                    list.add_checked(item.clone(), current_section).map(|()| {
                        report.added += 1;
                    })
                }
            }
        };
        match outcome {
            Ok(()) => {}
            // A full list keeps what fit and says so.
            Err(WatchlistError::ListFull) => {
                report.truncated = true;
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portfolio_card_is_left_out_of_the_text_export() {
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
        let mut list = Watchlist::new("Mixed");
        list.add(
            WatchlistInstrument::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "BTC",
            ),
            None,
        )
        .unwrap();
        list.add(
            WatchlistInstrument::for_card(PortfolioCard::new(CardKind::Allocation, None), "All"),
            None,
        )
        .unwrap();
        let text = export(&list);
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.ends_with("BTCUSDT"), "{text}");
    }

    fn instruments(parsed: &Parsed) -> Vec<(String, String)> {
        parsed
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Instrument(item) => Some((
                    item.instrument.source.watchlist_slug().to_owned(),
                    item.instrument.symbol.clone(),
                )),
                Row::Section(_) => None,
            })
            .collect()
    }

    #[test]
    fn parses_each_documented_form() {
        let parsed = parse(
            "###Majors\nBINANCE:btcusdt\nCOINBASE:eth-usd\nDEXSCREENER:solana/PairAddr1\n\
             COINGECKO:Bitcoin-Cash\nPOLYMARKET:123456|Will it happen?\nNASDAQ:aapl\nALPACA:brk.b",
        );
        assert!(parsed.skipped.is_empty(), "{:?}", parsed.skipped);
        assert_eq!(parsed.rows[0], Row::Section("Majors".to_owned()));
        assert_eq!(
            instruments(&parsed),
            [
                ("BINANCE", "BTCUSDT"),
                ("COINBASE", "ETH-USD"),
                ("DEXSCREENER", "PairAddr1"),
                ("COINGECKO", "bitcoin-cash"),
                ("POLYMARKET", "123456"),
                ("ALPACA", "AAPL"),
                ("ALPACA", "BRK.B"),
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
        let Row::Instrument(cash) = &parsed.rows[4] else {
            panic!("expected CoinGecko row");
        };
        assert_eq!(cash.name, "Bitcoin Cash");
        assert_eq!(cash.label, "BITCOIN-CASH");
        let Row::Instrument(market) = &parsed.rows[5] else {
            panic!("expected Polymarket row");
        };
        assert_eq!(market.display_name.as_deref(), Some("Will it happen?"));
    }

    #[test]
    fn a_single_comma_separated_line_is_read_as_tradingview_exports_it() {
        let parsed = parse("###Tech,NASDAQ:AAPL,NYSE:KO,BINANCE:BTCUSDT");
        assert_eq!(parsed.rows.len(), 4);
        assert!(parsed.skipped.is_empty());
        // Several lines with a comma inside one are not split.
        let multi = parse("BINANCE:BTCUSDT\nPOLYMARKET:1|Yes, maybe");
        assert_eq!(multi.rows.len(), 2);
    }

    #[test]
    fn unsupported_rows_are_reported_with_their_line_and_never_block_good_ones() {
        let parsed = parse(
            "BINANCE:BTCUSDT\nCME:ES1!\nBTC\nBINANCE:BTCUSDT.P\nCOINBASE:BTCUSD\nPOLYMARKET:9\n\
             CMC:fear\nNASDAQ:TOOLONGX\nBINANCE:\n\nBINANCE:ETHUSDT",
        );
        assert_eq!(parsed.rows.len(), 2);
        let reasons: Vec<(usize, &str)> = parsed
            .skipped
            .iter()
            .map(|skipped| (skipped.line, skipped.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            [
                (2, "Unsupported exchange CME"),
                (3, "Missing exchange prefix, expected SOURCE:SYMBOL"),
                (
                    4,
                    "Not a Binance spot symbol (perpetuals and futures aren't supported)"
                ),
                (5, "Coinbase products are written BASE-QUOTE, like BTC-USD"),
                (
                    6,
                    "Prediction markets need a title after |; add them from search instead"
                ),
                (7, "CoinMarketCap index charts aren't markets"),
                (8, "Not a US stock ticker"),
                (9, "Missing symbol"),
            ]
        );
    }

    #[test]
    fn an_empty_section_title_gets_a_default() {
        assert_eq!(parse("###  ").rows, [Row::Section("Section".to_owned())]);
    }

    #[test]
    fn export_then_parse_round_trips_order_and_sections() {
        let expected = "BINANCE:BTCUSDT\n###Majors\nCOINBASE:ETH-USD\nPOLYMARKET:42|Rain tomorrow?\nALPACA:AAPL";
        let mut list = Watchlist::new("L");
        let report = import_into(&mut list, &parse(expected)).unwrap();
        assert_eq!((report.added, report.sections_added), (4, 1));
        let text = export(&list);
        assert_eq!(text, expected);
        let mut copy = Watchlist::new("Copy");
        import_into(&mut copy, &parse(&text)).unwrap();
        assert_eq!(export(&copy), text);
    }

    #[test]
    fn import_reuses_sections_by_title_and_counts_duplicates() {
        let mut list = Watchlist::new("L");
        import_into(&mut list, &parse("###Majors\nBINANCE:BTCUSDT")).unwrap();
        let report = import_into(
            &mut list,
            &parse("###majors\nBINANCE:btcusdt\nBINANCE:ETHUSDT\nXX:Y"),
        )
        .unwrap();
        assert_eq!(
            (
                report.added,
                report.duplicates,
                report.sections_added,
                report.skipped.len()
            ),
            (1, 1, 0, 1)
        );
        assert_eq!(list.sections().count(), 1);
        assert_eq!(
            report.summary(),
            "Added 1 symbol, 1 already in the list, 1 skipped"
        );
        // The new market landed inside the reused section.
        let eth = list
            .instruments()
            .find(|item| item.instrument.symbol == "ETHUSDT")
            .unwrap()
            .id;
        assert_eq!(list.owning_section(eth).unwrap().title, "Majors");
    }

    #[test]
    fn summary_pluralises() {
        let report = ImportReport {
            added: 2,
            sections_added: 2,
            ..ImportReport::default()
        };
        assert_eq!(report.summary(), "Added 2 symbols, 2 sections");
        assert_eq!(
            ImportReport {
                added: 1,
                sections_added: 1,
                ..ImportReport::default()
            }
            .summary(),
            "Added 1 symbol, 1 section"
        );
    }

    #[test]
    fn validate_market_normalises_and_rejects_by_source() {
        let binance = validate_market(DataSource::Binance, " btcusdt ").unwrap();
        assert_eq!(binance, InstrumentId::new(DataSource::Binance, "BTCUSDT"));
        let coinbase = validate_market(DataSource::Coinbase, "btc-usd").unwrap();
        assert_eq!(coinbase.symbol, "BTC-USD");
        assert_eq!(
            validate_market(DataSource::Alpaca, "aapl").unwrap().symbol,
            "AAPL"
        );
        assert!(validate_market(DataSource::Binance, "BTC-USD").is_err());
        assert!(validate_market(DataSource::Coinbase, "BTCUSD").is_err());
        assert!(validate_market(DataSource::Binance, "").is_err());
    }

    #[test]
    fn validate_market_refuses_what_symbols_alone_cannot_chart() {
        assert!(validate_market(DataSource::Polymarket, "0xabc").is_err());
        assert!(validate_market(DataSource::Kalshi, "SERIES/MARKET").is_err());
        assert!(validate_market(DataSource::CoinMarketCap, "total").is_err());
        // The text format's own separators must not smuggle in a title or another source.
        assert!(validate_market(DataSource::Binance, "BTCUSDT|x").is_err());
        assert!(validate_market(DataSource::Binance, "COINBASE:BTC-USD").is_err());
        assert!(validate_market(DataSource::Binance, "BTC\nUSDT").is_err());
    }

    #[test]
    fn an_oversize_import_reads_what_it_may_and_says_so() {
        let many: String = (0..MAX_IMPORT_LINES + 5)
            .map(|index| format!("BINANCE:S{index}USDT\n"))
            .collect();
        let parsed = parse(&many);
        assert!(parsed.truncated);
        assert_eq!(parsed.rows.len(), MAX_IMPORT_LINES);

        let huge = format!("BINANCE:BTCUSDT\n{}", "é".repeat(MAX_IMPORT_BYTES));
        let parsed = parse(&huge);
        assert!(parsed.truncated);
        assert_eq!(
            parsed.rows.len(),
            1,
            "cut on a character boundary, no panic"
        );

        let fine = parse("BINANCE:BTCUSDT\nBINANCE:ETHUSDT");
        assert!(!fine.truncated);
        let commas = parse("BINANCE:BTCUSDT,BINANCE:ETHUSDT");
        assert_eq!(commas.rows.len(), 2);
    }

    #[test]
    fn imported_titles_are_made_fit_for_a_name() {
        let long = "t".repeat(MAX_NAME_CHARS + 50);
        let parsed = parse(&format!("###\u{7}Ma\tjors\nPOLYMARKET:abc|{long}"));
        assert_eq!(parsed.rows[0], Row::Section("Ma jors".to_owned()));
        let Row::Instrument(item) = &parsed.rows[1] else {
            panic!("a prediction market row")
        };
        assert_eq!(
            item.display_name.as_deref().map(str::len),
            Some(MAX_NAME_CHARS)
        );
    }

    #[test]
    fn the_suggested_export_name_is_a_safe_file_name() {
        assert_eq!(export_file_name("Swing"), "Swing.txt");
        assert_eq!(export_file_name("a/b\\c:d"), "a b c d.txt");
        assert_eq!(export_file_name("  ..hidden "), "hidden.txt");
        assert_eq!(export_file_name("///"), "Watchlist.txt");
        assert_eq!(export_file_name("bell\u{7}"), "bell.txt");
        assert_eq!(export_file_name(&"x".repeat(500)).chars().count(), 104);
    }
}
