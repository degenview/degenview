//! `alert.freq_*`: how often one `alert()` call site may fire.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertFrequency {
    /// Every execution that reaches the call.
    All,
    /// The first execution on each bar.
    OncePerBar,
    /// Only the closing execution of a bar.
    OncePerBarClose,
}

impl AlertFrequency {
    pub const PINE_PREFIX: &'static str = "alert.";
    pub const PINE_NAMES: [&'static str; 3] = [
        "alert.freq_all",
        "alert.freq_once_per_bar",
        "alert.freq_once_per_bar_close",
    ];

    pub fn pine_name(self) -> &'static str {
        match self {
            Self::All => "alert.freq_all",
            Self::OncePerBar => "alert.freq_once_per_bar",
            Self::OncePerBarClose => "alert.freq_once_per_bar_close",
        }
    }

    pub fn from_pine_name(name: &str) -> Option<Self> {
        match name {
            "alert.freq_all" => Some(Self::All),
            "alert.freq_once_per_bar" => Some(Self::OncePerBar),
            "alert.freq_once_per_bar_close" => Some(Self::OncePerBarClose),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for name in AlertFrequency::PINE_NAMES {
            let freq = AlertFrequency::from_pine_name(name).unwrap();
            assert_eq!(freq.pine_name(), name);
        }
        assert_eq!(AlertFrequency::from_pine_name("alert.freq_nope"), None);
    }
}
