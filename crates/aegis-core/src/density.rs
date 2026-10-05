//! Sampled order-book densities. Scores describe observations, not calibrated probabilities.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Clone, Debug, Serialize)]
pub struct Density {
    pub side: Side,
    pub price: f64,
    pub quantity: f64,
    pub notional: f64,
    pub strength: f64,
    pub first_seen: u64,
    pub last_seen: u64,
    pub touches: u32,
    pub reactions: u32,
    pub score: f64,
    #[serde(skip)]
    armed: bool,
    #[serde(skip)]
    awaiting_reaction: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    pub kind: &'static str,
    pub side: Side,
    pub price: f64,
    pub time: u64,
}

#[derive(Default)]
pub struct Tracker {
    levels: BTreeMap<(Side, u64), Density>,
    last_sample: Option<u64>,
}

impl Tracker {
    pub fn reset(&mut self) {
        self.levels.clear();
        self.last_sample = None;
    }

    pub fn sample(&mut self, time: u64, bids: &[(f64, f64)], asks: &[(f64, f64)]) -> (Vec<Density>, Vec<Observation>) {
        if self.last_sample.is_some_and(|last| time.saturating_sub(last) > 5_000) {
            self.reset();
        }
        self.last_sample = Some(time);
        let bid = bids.iter().map(|v| v.0).fold(f64::NEG_INFINITY, f64::max);
        let ask = asks.iter().map(|v| v.0).fold(f64::INFINITY, f64::min);
        let mid = (bid + ask) / 2.0;
        let zone = (ask - bid).max(0.02);
        let mut next = BTreeMap::new();
        let mut events = Vec::new();
        for (side, book) in [(Side::Bid, bids), (Side::Ask, asks)] {
            let mut quantities: Vec<_> = book
                .iter()
                .filter(|(p, q)| p.is_finite() && *p > 0.0 && q.is_finite() && *q > 0.0)
                .map(|(_, q)| *q)
                .collect();
            if quantities.len() < 3 {
                continue;
            }
            quantities.sort_by(f64::total_cmp);
            let n = quantities.len();
            let median = if n % 2 == 0 {
                quantities[n / 2 - 1] / 2.0 + quantities[n / 2] / 2.0
            } else {
                quantities[n / 2]
            };
            for &(price, quantity) in book {
                let strength = quantity / median;
                if !price.is_finite() || price <= 0.0 || !strength.is_finite() || strength < 3.0 {
                    continue;
                }
                let key = (side, price.to_bits());
                let distance = match side {
                    Side::Bid => mid - price,
                    Side::Ask => price - mid,
                };
                let mut row = self.levels.remove(&key).unwrap_or_else(|| {
                    events.push(Observation {
                        kind: "appeared",
                        side,
                        price,
                        time,
                    });
                    Density {
                        side,
                        price,
                        quantity,
                        notional: price * quantity,
                        strength,
                        first_seen: time,
                        last_seen: time,
                        touches: 0,
                        reactions: 0,
                        score: 0.0,
                        armed: distance > zone,
                        awaiting_reaction: false,
                    }
                });
                row.quantity = quantity;
                row.notional = price * quantity;
                row.strength = strength;
                row.last_seen = time;
                if distance <= zone && row.armed {
                    row.touches += 1;
                    row.armed = false;
                    row.awaiting_reaction = true;
                    events.push(Observation {
                        kind: "touch",
                        side,
                        price,
                        time,
                    });
                }
                if distance > 3.0 * zone {
                    if row.awaiting_reaction {
                        row.reactions += 1;
                        row.awaiting_reaction = false;
                        events.push(Observation {
                            kind: "reaction",
                            side,
                            price,
                            time,
                        });
                    }
                    row.armed = true;
                }
                let age = time.saturating_sub(row.first_seen) as f64 / 1_000.0;
                row.score = (40.0 * (age / 300.0).min(1.0)
                    + 30.0 * ((strength - 3.0) / 7.0).clamp(0.0, 1.0)
                    + 30.0 * (row.reactions as f64 / 3.0).min(1.0))
                .min(100.0);
                next.insert(key, row);
            }
        }
        for row in self.levels.values() {
            events.push(Observation {
                kind: "no_longer_density",
                side: row.side,
                price: row.price,
                time,
            });
        }
        self.levels = next;
        (self.levels.values().cloned().collect(), events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tracker: &mut Tracker, time: u64, best: f64, quantity: f64) -> Vec<Density> {
        tracker
            .sample(
                time,
                &[(best, 1.0), (99.0, quantity), (98.0, 1.0)],
                &[(best + 0.1, 1.0), (102.0, 1.0), (103.0, 1.0)],
            )
            .0
    }

    #[test]
    fn distinct_touches_require_retreat_and_survival() {
        let mut tracker = Tracker::default();
        assert_eq!(sample(&mut tracker, 1_000, 100.0, 10.0)[0].touches, 0);
        assert_eq!(sample(&mut tracker, 2_000, 99.0, 10.0)[0].touches, 1);
        assert_eq!(sample(&mut tracker, 3_000, 99.0, 10.0)[0].touches, 1);
        let rows = sample(&mut tracker, 4_000, 100.0, 10.0);
        assert_eq!(rows[0].reactions, 1);
        assert_eq!(rows[0].first_seen, 1_000);
        assert_eq!(sample(&mut tracker, 5_000, 99.0, 10.0)[0].touches, 2);
    }

    #[test]
    fn disappearance_gap_and_reset_do_not_invent_age() {
        let mut tracker = Tracker::default();
        sample(&mut tracker, 1_000, 100.0, 10.0);
        assert!(sample(&mut tracker, 2_000, 100.0, 1.0).is_empty());
        assert_eq!(sample(&mut tracker, 3_000, 100.0, 10.0)[0].first_seen, 3_000);
        assert_eq!(sample(&mut tracker, 10_000, 100.0, 10.0)[0].first_seen, 10_000);
        tracker.reset();
        assert_eq!(sample(&mut tracker, 11_000, 100.0, 10.0)[0].first_seen, 11_000);
    }

    #[test]
    fn small_or_invalid_books_do_not_produce_densities() {
        let mut tracker = Tracker::default();
        assert!(tracker.sample(1_000, &[(99.0, 10.0)], &[(100.0, 1.0)]).0.is_empty());
        assert!(tracker
            .sample(
                2_000,
                &[(99.0, f64::NAN), (98.0, 1.0), (97.0, 1.0)],
                &[(100.0, 1.0), (101.0, 1.0), (102.0, 1.0)]
            )
            .0
            .is_empty());
    }
}
