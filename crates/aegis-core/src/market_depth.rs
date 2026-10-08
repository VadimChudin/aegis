use serde::{Deserialize, Serialize};

/// One price level in an order book. Quantity uses the venue's native units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookLevel {
    pub price: f64,
    pub quantity: f64,
}

/// A point-in-time order-book snapshot. `timestamp` is Unix milliseconds;
/// it is exchange-provided when available, otherwise the local observation time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderBook {
    pub symbol: String,
    pub timestamp: u64,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
}

pub type DepthLevel = BookLevel;
pub type OrderBookSnapshot = OrderBook;

impl OrderBook {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.symbol.trim().is_empty() {
            return Err("symbol is empty".into());
        }
        if self.timestamp == 0 {
            return Err("timestamp must be positive Unix milliseconds".into());
        }
        validate_side("bids", &self.bids, true)?;
        validate_side("asks", &self.asks, false)?;
        if self.bids[0].price >= self.asks[0].price {
            return Err("order book is locked or crossed".into());
        }
        Ok(())
    }
}

fn validate_side(name: &str, levels: &[BookLevel], descending: bool) -> Result<(), String> {
    if levels.is_empty() {
        return Err(format!("{name} are empty"));
    }
    for (index, level) in levels.iter().enumerate() {
        if !level.price.is_finite() || level.price <= 0.0 {
            return Err(format!("{name}[{index}] price must be finite and positive"));
        }
        if !level.quantity.is_finite() || level.quantity <= 0.0 {
            return Err(format!("{name}[{index}] quantity must be finite and positive"));
        }
        if index > 0 {
            let prev = levels[index - 1].price;
            let ordered = if descending {
                prev > level.price
            } else {
                prev < level.price
            };
            if !ordered {
                return Err(format!("{name} are not strictly price-ordered"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> OrderBook {
        OrderBook {
            symbol: "XAUUSDT".into(),
            timestamp: 1_700_000_000_000,
            bids: vec![BookLevel {
                price: 10.0,
                quantity: 1.0,
            }],
            asks: vec![BookLevel {
                price: 11.0,
                quantity: 2.0,
            }],
        }
    }

    #[test]
    fn validates_snapshot_data() {
        assert!(snapshot().validate().is_ok());
        let mut invalid = snapshot();
        invalid.timestamp = 0;
        assert!(invalid.validate().unwrap_err().contains("timestamp"));
        let mut invalid = snapshot();
        invalid.bids[0].price = f64::NAN;
        assert!(invalid.validate().unwrap_err().contains("price"));
        let mut invalid = snapshot();
        invalid.asks.push(BookLevel {
            price: 12.0,
            quantity: 1.0,
        });
        invalid.asks.swap(0, 1);
        assert!(invalid.validate().unwrap_err().contains("ordered"));
        let mut invalid = snapshot();
        invalid.asks[0].price = invalid.bids[0].price;
        assert!(invalid.validate().unwrap_err().contains("crossed"));
    }
}
