//! Strategy model: settings, trades and the backtest report, ported from `PineStrategySettings`,
//! `PineOpenTrade`, `PineTrade` and `PineStrategyReport`.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuantityType {
    Fixed,
    Cash,
    PercentOfEquity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommissionType {
    Percent,
    CashPerOrder,
    CashPerContract,
}

/// The `strategy()` declaration arguments the broker emulator honours.
#[derive(Clone, Debug, PartialEq)]
pub struct StrategySettings {
    pub initial_capital: f64,
    pub quantity_type: QuantityType,
    pub quantity_value: f64,
    pub commission_type: CommissionType,
    pub commission_value: f64,
    /// Adverse slippage, in ticks, on market and stop fills.
    pub slippage: usize,
    pub pyramiding: usize,
    pub currency: Option<String>,
    pub process_orders_on_close: bool,
    pub calc_on_every_tick: bool,
}

impl Default for StrategySettings {
    fn default() -> Self {
        Self {
            initial_capital: 1_000_000.0,
            quantity_type: QuantityType::Fixed,
            quantity_value: 1.0,
            commission_type: CommissionType::Percent,
            commission_value: 0.0,
            slippage: 0,
            pyramiding: 1,
            currency: None,
            process_orders_on_close: false,
            calc_on_every_tick: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenTrade {
    pub entry_id: String,
    /// The `comment` of the order that opened it.
    pub entry_comment: String,
    pub is_long: bool,
    pub quantity: f64,
    pub entry_bar: usize,
    pub entry_time: i64,
    pub entry_price: f64,
    pub entry_commission: f64,
    /// The highest and lowest price the market has reached since the trade opened, from which its
    /// run-up and drawdown follow.
    pub highest_price: f64,
    pub lowest_price: f64,
}

impl OpenTrade {
    pub fn id(&self) -> String {
        format!("{}@{}", self.entry_id, self.entry_bar)
    }
}

impl OpenTrade {
    /// The most the trade has been ahead and the most it has been behind (both as positive amounts
    /// of money, before commission) at the best and the worst price it has seen.
    pub fn excursions(&self) -> (f64, f64) {
        excursions(
            self.is_long,
            self.entry_price,
            self.quantity,
            self.highest_price,
            self.lowest_price,
        )
    }
}

impl Trade {
    /// What the trade's orders said about it: their comments, one per line, empty when neither
    /// gave one. Shown with the trade next to its ids.
    pub fn comments(&self) -> String {
        let mut lines = Vec::new();
        if !self.entry_comment.is_empty() {
            lines.push(format!("Entry: {}", self.entry_comment));
        }
        if !self.exit_comment.is_empty() {
            lines.push(format!("Exit: {}", self.exit_comment));
        }
        lines.join("\n")
    }
}

/// `(run-up, drawdown)` of `quantity` units entered at `entry` that the market has since taken to
/// `highest` and `lowest`: how far the trade could have been in profit and how far in loss.
pub fn excursions(
    is_long: bool,
    entry: f64,
    quantity: f64,
    highest: f64,
    lowest: f64,
) -> (f64, f64) {
    let (favourable, adverse) = if is_long {
        (highest - entry, entry - lowest)
    } else {
        (entry - lowest, highest - entry)
    };
    (favourable.max(0.0) * quantity, adverse.max(0.0) * quantity)
}

/// A round trip closed by the broker emulator. `profit` is net of both commissions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trade {
    pub id: usize,
    pub entry_id: String,
    /// The `comment` of the order that opened it, and of the one that closed it.
    pub entry_comment: String,
    pub exit_comment: String,
    pub exit_id: String,
    pub is_long: bool,
    pub quantity: f64,
    pub entry_bar: usize,
    pub entry_time: i64,
    pub entry_price: f64,
    pub exit_bar: usize,
    pub exit_time: i64,
    pub exit_price: f64,
    pub commission: f64,
    pub profit: f64,
    /// How far the trade was in profit at best and in loss at worst, as positive amounts of money
    /// before commission.
    pub max_runup: f64,
    pub max_drawdown: f64,
}

/// The extremes of the equity curve, kept as its points arrive: the largest fall from a peak and the
/// largest rise from a trough, each in currency and as a percentage of the peak (or trough) it is
/// measured from. The curve starts at the initial capital.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EquityExtremes {
    peak: f64,
    trough: f64,
    pub max_drawdown: f64,
    pub max_drawdown_percent: f64,
    pub max_runup: f64,
    pub max_runup_percent: f64,
}

impl EquityExtremes {
    pub fn starting_at(initial_capital: f64) -> Self {
        Self {
            peak: initial_capital,
            trough: initial_capital,
            max_drawdown: 0.0,
            max_drawdown_percent: 0.0,
            max_runup: 0.0,
            max_runup_percent: 0.0,
        }
    }

    /// Takes the next point of the curve into account.
    pub fn include(&mut self, equity: f64) {
        self.peak = self.peak.max(equity);
        self.trough = self.trough.min(equity);
        let drawdown = self.peak - equity;
        let runup = equity - self.trough;
        self.max_drawdown = self.max_drawdown.max(drawdown);
        self.max_runup = self.max_runup.max(runup);
        if self.peak > 0.0 {
            self.max_drawdown_percent = self.max_drawdown_percent.max(drawdown / self.peak * 100.0);
        }
        if self.trough > 0.0 {
            self.max_runup_percent = self.max_runup_percent.max(runup / self.trough * 100.0);
        }
    }
}

/// Backtest result of a `strategy()` script.
#[derive(Clone, Debug, PartialEq)]
pub struct StrategyReport {
    pub settings: StrategySettings,
    pub trades: Vec<Trade>,
    pub open_trades: Vec<OpenTrade>,
    /// Mark-to-market equity after each bar, parallel to the script's bars.
    pub equity: Vec<f64>,
    /// Unrealized P&L of the open position at the last bar, net of entry commissions.
    pub open_profit: f64,
    /// The chart the strategy ran on. On a price-based chart its orders filled at that chart's
    /// constructed prices, which are not prices anyone traded at.
    pub chart: crate::model::ChartKind,
}

impl StrategyReport {
    pub fn net_profit(&self) -> f64 {
        self.trades.iter().map(|trade| trade.profit).sum()
    }

    pub fn gross_profit(&self) -> f64 {
        self.trades
            .iter()
            .filter(|t| t.profit > 0.0)
            .map(|t| t.profit)
            .sum()
    }

    pub fn gross_loss(&self) -> f64 {
        -self
            .trades
            .iter()
            .filter(|t| t.profit < 0.0)
            .map(|t| t.profit)
            .sum::<f64>()
    }

    pub fn win_count(&self) -> usize {
        self.trades.iter().filter(|t| t.profit > 0.0).count()
    }

    pub fn win_rate(&self) -> Option<f64> {
        if self.trades.is_empty() {
            None
        } else {
            Some(self.win_count() as f64 / self.trades.len() as f64)
        }
    }

    pub fn profit_factor(&self) -> Option<f64> {
        if self.gross_loss() > 0.0 {
            Some(self.gross_profit() / self.gross_loss())
        } else {
            None
        }
    }

    pub fn average_trade(&self) -> Option<f64> {
        if self.trades.is_empty() {
            None
        } else {
            Some(self.net_profit() / self.trades.len() as f64)
        }
    }

    pub fn final_equity(&self) -> f64 {
        self.equity
            .last()
            .copied()
            .unwrap_or(self.settings.initial_capital)
    }

    pub fn return_fraction(&self) -> f64 {
        if self.settings.initial_capital > 0.0 {
            (self.final_equity() - self.settings.initial_capital) / self.settings.initial_capital
        } else {
            0.0
        }
    }

    /// Largest peak-to-trough fall of the equity curve, in currency and as a fraction of the peak.
    pub fn max_drawdown(&self) -> (f64, f64) {
        let mut peak = self.settings.initial_capital;
        let mut worst = (0.0, 0.0);
        for &value in &self.equity {
            peak = peak.max(value);
            let drop = peak - value;
            if drop > worst.0 {
                worst = (drop, if peak > 0.0 { drop / peak } else { 0.0 });
            }
        }
        worst
    }
}
