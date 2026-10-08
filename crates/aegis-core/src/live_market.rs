use serde::{Deserialize, Serialize};

use crate::market_depth::OrderBookSnapshot;

const MAX_TICKS: usize = 256;
const MAX_FUTURE_MS: u64 = 2_000;
const MAX_STALE_MS: u64 = 10_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub time_ms: u64,
    pub bid: f64,
    pub ask: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    pub time_ms: u64,
    pub bid: f64,
    pub ask: f64,
    pub last: Option<f64>,
    pub volume: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketSample {
    pub symbol: String,
    pub observed_at_ms: u64,
    pub quote: Quote,
    pub ticks: Vec<Tick>,
    pub book: Option<OrderBookSnapshot>,
    pub book_note: String,
    pub volume_kind: String,
}

impl MarketSample {
    pub fn validate(&self, now_ms: u64) -> Result<(), String> {
        if self.symbol.trim().is_empty() {
            return Err("symbol is empty".into());
        }
        validate_time("observation", self.observed_at_ms, now_ms, true)?;
        if self.ticks.len() > MAX_TICKS {
            return Err(format!("tick count exceeds {MAX_TICKS}"));
        }
        validate_prices("quote", self.quote.bid, self.quote.ask)?;
        validate_time("quote", self.quote.time_ms, now_ms, true)?;

        let mut previous = None;
        for (index, tick) in self.ticks.iter().enumerate() {
            validate_time(&format!("ticks[{index}]"), tick.time_ms, now_ms, false)?;
            validate_prices(&format!("ticks[{index}]"), tick.bid, tick.ask)?;
            if tick.last.is_some_and(|last| !last.is_finite() || last <= 0.0) {
                return Err(format!("ticks[{index}].last must be finite and positive"));
            }
            if tick.volume.is_some_and(|volume| !volume.is_finite() || volume < 0.0) {
                return Err(format!("ticks[{index}].volume must be finite and non-negative"));
            }
            if previous.is_some_and(|time| tick.time_ms < time) {
                return Err("ticks are not in nondecreasing timestamp order".into());
            }
            previous = Some(tick.time_ms);
        }

        if self.volume_kind != "mt5_ticks_not_exchange_tape" {
            return Err("volume_kind must identify MT5 tick volume, not exchange tape".into());
        }
        match &self.book {
            Some(book) => {
                book.validate()?;
                validate_time("book", book.timestamp, now_ms, true)?;
                if book.symbol != self.symbol {
                    return Err("book symbol does not match sample symbol".into());
                }
            }
            None if self.book_note.trim().is_empty() => {
                return Err("book_note must explain when depth is unavailable".into());
            }
            None => {}
        }
        Ok(())
    }
}

fn validate_prices(name: &str, bid: f64, ask: f64) -> Result<(), String> {
    if !bid.is_finite() || bid <= 0.0 {
        return Err(format!("{name}.bid must be finite and positive"));
    }
    if !ask.is_finite() || ask <= 0.0 {
        return Err(format!("{name}.ask must be finite and positive"));
    }
    if ask < bid {
        return Err(format!("{name}.ask must be greater than or equal to bid"));
    }
    Ok(())
}

fn validate_time(name: &str, timestamp: u64, now_ms: u64, require_fresh: bool) -> Result<(), String> {
    if timestamp == 0 {
        return Err(format!("{name} timestamp must be positive Unix milliseconds"));
    }
    if timestamp > now_ms.saturating_add(MAX_FUTURE_MS) {
        let ahead_ms = timestamp.saturating_sub(now_ms);
        return Err(format!(
            "{name} timestamp is more than 2 seconds in the future: ahead_ms={ahead_ms}, \
             timestamp_ms={timestamp}, app_now_ms={now_ms}, allowed_future_ms={MAX_FUTURE_MS}. \
             Synchronize the computer clock and reconnect MT5. If this persists, inspect broker timestamps; \
             changing the display timezone does not change Unix time. Paper/Money start is blocked."
        ));
    }
    if require_fresh && now_ms.saturating_sub(timestamp) > MAX_STALE_MS {
        let age_ms = now_ms.saturating_sub(timestamp);
        return Err(format!(
            "{name} timestamp is more than 10 seconds stale: age_ms={age_ms}, \
             timestamp_ms={timestamp}, app_now_ms={now_ms}, allowed_stale_ms={MAX_STALE_MS}. \
             Check the MT5 connection and whether the symbol's market is open. \
             Old prices are not replaced with the current time. Paper/Money start is blocked."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(now: u64) -> MarketSample {
        MarketSample {
            symbol: "XAUUSD".into(),
            observed_at_ms: now,
            quote: Quote {
                time_ms: now,
                bid: 2_000.0,
                ask: 2_000.1,
            },
            ticks: vec![Tick {
                time_ms: now,
                bid: 2_000.0,
                ask: 2_000.1,
                last: Some(2_000.05),
                volume: None,
            }],
            book: None,
            book_note: "Market depth is not supported".into(),
            volume_kind: "mt5_ticks_not_exchange_tape".into(),
        }
    }

    #[test]
    fn validates_live_sample_and_allows_missing_book_and_volume() {
        assert!(sample(1_700_000_000_000).validate(1_700_000_000_000).is_ok());
    }

    #[test]
    fn rejects_stale_and_future_quote_and_unsorted_or_oversized_ticks() {
        let now = 1_700_000_000_000;
        let mut value = sample(now);
        value.quote.time_ms -= MAX_STALE_MS + 1;
        assert!(value.validate(now).unwrap_err().contains("stale"));

        let mut value = sample(now);
        value.quote.time_ms += MAX_FUTURE_MS + 1;
        assert!(value.validate(now).unwrap_err().contains("future"));

        let mut value = sample(now);
        value.observed_at_ms -= MAX_STALE_MS + 1;
        assert!(value.validate(now).unwrap_err().contains("observation"));

        let mut value = sample(now);
        value.ticks.push(Tick {
            time_ms: now - 1,
            bid: 2_000.0,
            ask: 2_000.1,
            last: None,
            volume: None,
        });
        assert!(value.validate(now).unwrap_err().contains("nondecreasing"));

        let mut value = sample(now);
        let first = value.ticks[0].clone();
        value.ticks.resize(MAX_TICKS + 1, first);
        assert!(value.validate(now).unwrap_err().contains("exceeds"));
    }

    #[test]
    fn timestamp_diagnostics_preserve_exact_future_and_stale_boundaries() {
        let now = 1_700_000_000_000;
        assert!(validate_time("quote", now + MAX_FUTURE_MS, now, true).is_ok());
        let future = validate_time("quote", now + MAX_FUTURE_MS + 1, now, true).unwrap_err();
        assert!(future.contains("quote timestamp is more than 2 seconds in the future"));
        assert!(future.contains("ahead_ms=2001"));
        assert!(future.contains("timestamp_ms=1700000002001"));
        assert!(future.contains("app_now_ms=1700000000000"));
        assert!(future.contains("allowed_future_ms=2000"));
        assert!(future.contains("Synchronize"));
        assert!(validate_time("quote", now - MAX_STALE_MS, now, true).is_ok());
        let stale = validate_time("quote", now - MAX_STALE_MS - 1, now, true).unwrap_err();
        assert!(stale.contains("age_ms=10001"));
        assert!(stale.contains("allowed_stale_ms=10000"));
        assert!(stale.contains("market is open"));
        // Historical ticks can be old, but a future tick must still be rejected.
        assert!(validate_time("ticks[0]", now - MAX_STALE_MS - 1, now, false).is_ok());
        assert!(validate_time("ticks[0]", now + MAX_FUTURE_MS + 1, now, false)
            .unwrap_err()
            .contains("ticks[0]"));
        assert!(validate_time("observation", 0, now, true).is_err());
    }

    #[test]
    fn rejects_invalid_prices_and_unlabeled_volume() {
        let now = 1_700_000_000_000;
        let mut value = sample(now);
        value.quote.ask = f64::NAN;
        assert!(value.validate(now).unwrap_err().contains("ask"));

        let mut value = sample(now);
        value.volume_kind = "exchange_tape".into();
        assert!(value.validate(now).unwrap_err().contains("volume_kind"));
    }
}
