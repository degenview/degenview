//! What the two sides of a market are called: "Bitcoin" for `BTC`, "Apple Inc." for `AAPL`.
//! These are market data, not interface text, so they are never translated.

#[cfg(test)]
mod tests;

/// The longest name kept. DEX tokens name themselves, so a name has no length of its own.
const MAX_NAME_CHARS: usize = 64;

/// The names of a market's two sides, as far as they are known.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct AssetNames {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}

impl AssetNames {
    /// Names as a provider sent them, made safe to show.
    pub fn new(base: Option<&str>, quote: Option<&str>) -> Self {
        Self {
            base: base.and_then(sanitize),
            quote: quote.and_then(sanitize),
        }
    }
}

/// A name fit to show on one line: no control or direction-changing characters, runs of
/// whitespace as one space, and not endless. `None` when nothing is left.
pub fn sanitize(name: &str) -> Option<String> {
    let spaced: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !is_invisible(*c))
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() <= MAX_NAME_CHARS {
        return Some(joined);
    }
    let cut: String = joined.chars().take(MAX_NAME_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// Zero-width and bidirectional formatting characters, which can make a name read as another.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// What a listing appends to a company's name to say which security it is.
const SECURITY_KINDS: [&str; 8] = [
    " american depositary shares",
    " american depositary share",
    " depositary shares",
    " common stock",
    " common shares",
    " capital stock",
    " ordinary shares",
    " ordinary share",
];

/// A company's name without the security a listing appends to it: "Apple Inc. Common Stock" is
/// "Apple Inc.". The share class stays, since it tells two listings apart.
pub fn company_name(listing: &str) -> String {
    let listing = listing.trim();
    let lower = listing.to_ascii_lowercase();
    let end = SECURITY_KINDS
        .iter()
        .filter_map(|kind| lower.find(kind))
        .min()
        .unwrap_or(listing.len());
    let name = listing[..end].trim_end_matches([',', '-', ' ']).trim_end();
    if name.is_empty() {
        listing.to_owned()
    } else {
        name.to_owned()
    }
}

/// The name of a currency or stablecoin code, which coin directories do not all list.
pub fn currency_name(code: &str) -> Option<&'static str> {
    Some(match code.to_ascii_uppercase().as_str() {
        "USD" => "US Dollar",
        "EUR" => "Euro",
        "GBP" => "British Pound",
        "JPY" => "Japanese Yen",
        "CHF" => "Swiss Franc",
        "AUD" => "Australian Dollar",
        "NZD" => "New Zealand Dollar",
        "CAD" => "Canadian Dollar",
        "CNH" => "Offshore Yuan",
        "CNY" => "Chinese Yuan",
        "HKD" => "Hong Kong Dollar",
        "SGD" => "Singapore Dollar",
        "SEK" => "Swedish Krona",
        "NOK" => "Norwegian Krone",
        "DKK" => "Danish Krone",
        "PLN" => "Polish Zloty",
        "CZK" => "Czech Koruna",
        "HUF" => "Hungarian Forint",
        "TRY" => "Turkish Lira",
        "ZAR" => "South African Rand",
        "MXN" => "Mexican Peso",
        "BRL" => "Brazilian Real",
        "ARS" => "Argentine Peso",
        "INR" => "Indian Rupee",
        "THB" => "Thai Baht",
        "AED" => "UAE Dirham",
        "RUB" => "Russian Ruble",
        "UAH" => "Ukrainian Hryvnia",
        "USDT" => "Tether",
        "USDC" => "USD Coin",
        "USDS" => "USDS",
        "FDUSD" => "First Digital USD",
        "TUSD" => "TrueUSD",
        "DAI" => "Dai",
        "PYUSD" => "PayPal USD",
        "BUSD" => "Binance USD",
        "USDE" => "Ethena USDe",
        _ => return None,
    })
}

/// The companies of the stocks offered before anything is searched, which no list has named yet.
pub fn popular_company(ticker: &str) -> Option<&'static str> {
    Some(match ticker.to_ascii_uppercase().as_str() {
        "AAPL" => "Apple Inc.",
        "MSFT" => "Microsoft Corporation",
        "NVDA" => "NVIDIA Corporation",
        "AMZN" => "Amazon.com, Inc.",
        "GOOGL" => "Alphabet Inc. Class A",
        "META" => "Meta Platforms, Inc.",
        "TSLA" => "Tesla, Inc.",
        "SPY" => "SPDR S&P 500 ETF Trust",
        "QQQ" => "Invesco QQQ Trust",
        "AMD" => "Advanced Micro Devices, Inc.",
        _ => return None,
    })
}
