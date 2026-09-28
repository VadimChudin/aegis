//! The metrics recorded for every touch. Direction-aware metrics are signed so that a
//! positive value favours the bounce (for a long at support and a short at resistance alike).

use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FeatureSpec {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    /// Slider range and step.
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
    pub help: &'static str,
}

macro_rules! features {
    ($( $name:ident = $id:literal, $group:literal, $label:literal, $lo:expr, $hi:expr, $step:expr, $help:literal; )*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub(crate) enum F { $($name),* }

        pub const FEATURES: &[FeatureSpec] = &[
            $(FeatureSpec { id: $id, group: $group, label: $label, lo: $lo, hi: $hi, step: $step, help: $help }),*
        ];
    };
}

features! {
    // Level
    Confluence = "confluence", "Level", "Confluence", 1.0, 6.0, 1.0, "Levels of different kinds inside the touch zone.";
    LevelAge = "level_age_h", "Level", "Level age, h", 0.0, 120.0, 1.0, "Hours since the level formed.";
    LevelTouches = "level_touches", "Level", "Earlier touches", 0.0, 10.0, 1.0, "How many times this level was touched before.";
    LevelCrosses = "level_crosses", "Level", "Earlier breaks", 0.0, 10.0, 1.0, "How many times price closed through the level before (mirror level).";
    Room = "room_atr", "Level", "Room to next level, ATR", 0.0, 10.0, 0.25, "Distance to the next level in the trade direction.";
    // Approach
    ApprSpeed = "appr_speed", "Approach", "Approach speed, ATR/6 bars", 0.0, 8.0, 0.25, "How far price travelled toward the level over the last 6 bars.";
    ApprBars = "appr_bars", "Approach", "Bars in a row toward", 0.0, 10.0, 1.0, "Consecutive bars closing toward the level.";
    ApprRange = "appr_range", "Approach", "Approach bar size, ATR", 0.0, 3.0, 0.1, "Average range of the 6 approach bars; small = compression.";
    ApprFrom = "appr_from", "Approach", "Run from extreme, ATR", 0.0, 15.0, 0.5, "Distance from the opposite 24-bar extreme to the level.";
    // Market
    Trend5 = "trend_5m", "Market", "Trend 5m (for trade)", -3.0, 3.0, 0.1, "EMA50 slope over 10 bars in ATR; positive = trend in the trade direction.";
    Trend1h = "trend_1h", "Market", "Trend 1h (for trade)", -15.0, 15.0, 0.5, "Distance from the 1h EMA50 in ATR; positive = price on the trade side of it.";
    AtrUsd = "atr_usd", "Market", "Volatility ATR14, $", 0.0, 20.0, 0.25, "Average 5m bar range in dollars. Small ATR = costs eat the move.";
    AtrPct = "atr_pct", "Market", "Volatility percentile", 0.0, 1.0, 0.05, "ATR percentile over the last week of bars.";
    DayPos = "day_pos", "Market", "Position in day range", 0.0, 1.0, 0.05, "0 = at the day extreme against the trade (day low for a long).";
    AdrUsed = "adr_used", "Market", "Day range used", 0.0, 3.0, 0.1, "Today's range divided by the 20-day average range.";
    Rsi = "rsi_stretch", "Market", "RSI stretch", -40.0, 40.0, 1.0, "50 - RSI14 for a long, RSI14 - 50 for a short; positive = oversold into support.";
    Vwap = "vwap_dev", "Market", "VWAP deviation, ATR", -10.0, 10.0, 0.25, "Distance from the day VWAP; positive = price stretched away from VWAP against the trade.";
    // Touch candle
    Wick = "wick", "Candle", "Rejection wick", 0.0, 1.0, 0.05, "Wick on the level side as a share of the bar range.";
    Body = "body", "Candle", "Body share", 0.0, 1.0, 0.05, "Body as a share of the bar range.";
    ClosePos = "close_pos", "Candle", "Close position", 0.0, 1.0, 0.05, "Where the bar closed in its range; 1 = at the end favouring the trade.";
    Pen = "pen_atr", "Candle", "Penetration, ATR", -0.5, 2.0, 0.05, "How far the bar pierced through the level.";
    RangeAtr = "range_atr", "Candle", "Touch bar size, ATR", 0.0, 5.0, 0.1, "Range of the touch bar in ATR.";
    CloseBack = "close_back", "Candle", "Closed back over level", 0.0, 1.0, 1.0, "1 when the bar closed back on the bounce side of the level.";
    // Volume
    VolZ = "vol_ratio", "Volume", "Touch volume ratio", 0.0, 6.0, 0.1, "Touch bar volume / 50-bar average.";
    ApprVol = "appr_vol", "Volume", "Approach volume ratio", 0.0, 4.0, 0.1, "Approach bars volume / 50-bar average.";
    // Tape
    TradesZ = "tape_speed", "Tape", "Tape speed ratio", 0.0, 6.0, 0.1, "Trades in the touch bar / 50-bar average.";
    Delta = "delta", "Tape", "Touch delta (for trade)", -1.0, 1.0, 0.05, "Aggressive buy minus sell volume / volume in the touch bar, signed for the trade.";
    ApprDelta = "appr_delta", "Tape", "Approach delta (for trade)", -1.0, 1.0, 0.05, "Delta of the 6 approach bars / their volume, signed for the trade.";
    CvdDiv = "cvd_div", "Tape", "CVD divergence", 0.0, 1.0, 1.0, "1 when price made a new 12-bar extreme into the level but cumulative delta did not.";
    // Microstructure (aggTrades)
    BigDelta = "big_delta", "Clusters", "Large-trade delta (for trade)", -1.0, 1.0, 0.05, "Delta of large trades / touch bar volume, signed for the trade.";
    BigShare = "big_share", "Clusters", "Large-trade share", 0.0, 1.0, 0.05, "Volume of large trades / touch bar volume.";
    MaxTrade = "max_trade", "Clusters", "Largest order ratio", 0.0, 10.0, 0.25, "Largest single aggressive order / its 50-bar average.";
    ExtVol = "ext_vol", "Clusters", "Volume at the extreme", 0.0, 1.0, 0.05, "Share of the bar volume traded in the 20% of the range at the level.";
    ExtDelta = "ext_delta", "Clusters", "Delta at the extreme (for trade)", -1.0, 1.0, 0.05, "Delta in that 20% / its volume, signed for the trade; negative = aggression into the level (absorbed if the level holds).";
    PocPos = "poc_pos", "Clusters", "Bar POC position", 0.0, 1.0, 0.05, "Where the bar's biggest volume cluster is; 0 = at the level.";
    Imbalance = "imbalance", "Clusters", "Cluster imbalances (for trade)", -20.0, 20.0, 1.0, "Buy minus sell 3:1 price imbalances in the bar, signed for the trade.";
    // Derivatives
    OiChg = "oi_chg", "Derivatives", "Open interest change, %", -10.0, 10.0, 0.25, "OI change over the 6 approach bars.";
    LsTop = "ls_top", "Derivatives", "Top traders long/short", 0.0, 5.0, 0.1, "Binance top-trader long/short position ratio.";
    TakerRatio = "taker_ratio", "Derivatives", "Taker buy/sell ratio", 0.0, 3.0, 0.05, "Binance taker buy/sell volume ratio.";
    // Time
    Hour = "hour", "Time", "Hour, UTC", 0.0, 23.0, 1.0, "Hour of the touch bar close.";
    Weekday = "weekday", "Time", "Weekday", 0.0, 6.0, 1.0, "0 = Monday.";
    Spread = "spread_proxy", "Time", "Bar spread proxy, ATR", 0.0, 1.0, 0.01, "Median 1-tick move proxy: (high-low)/trades in ATR; wide = thin book.";
    NewsBefore = "news_before_min", "News", "Minutes to next major release", 0.0, 1440.0, 5.0, "Minutes until the next FOMC / CPI / NFP / PCE / PPI / GDP release (scheduled in advance, so known before the touch). 1440 = none within a day.";
    NewsAfter = "news_after_min", "News", "Minutes since last major release", 0.0, 1440.0, 5.0, "Minutes since the last major US release. Levels break more often right after news.";
    SpreadUsd = "spread_usd", "Costs", "Quoted spread, $", 0.0, 2.0, 0.01, "Bid/ask spread of the touch bar (sources with quotes: Dukascopy, MT5).";
    Funding = "funding_bp", "Derivatives", "Funding rate, bp", -20.0, 20.0, 0.5, "Last Binance funding rate in basis points; positive = longs pay.";
}

pub const NF: usize = FEATURES.len();

pub fn index(id: &str) -> Option<usize> {
    FEATURES.iter().position(|f| f.id == id)
}
