//! Sampled order-book densities. Scores describe observations, not calibrated probabilities.

use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;

use crate::settings::DensitySettings;

const DAY_MS: u64 = 86_400_000;
const MINUTE_MS: u64 = 60_000;
const MAX_SUMMARIES: usize = 1_440;
const MAX_BOOK_LEVELS: usize = 500;

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
    pub max_quantity: f64,
    pub quantity_change_pct: f64,
    #[serde(skip)]
    initial_quantity: f64,
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

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Thresholds {
    /// Current bid-side minimum quantity for a new density.
    pub bid: f64,
    /// Current ask-side minimum quantity for a new density.
    pub ask: f64,
    pub warming_up: bool,
    /// Current typical book activity divided by the today's baseline (1.0 = typical).
    pub activity_ratio: f64,
}

#[derive(Clone, Copy)]
struct MinuteSummary {
    minute: u64,
    bid_median: f64,
    ask_median: f64,
    bid_percentile: f64,
    ask_percentile: f64,
}

#[derive(Default)]
pub struct Tracker {
    levels: BTreeMap<(Side, u64), Density>,
    last_sample: Option<u64>,
    day: Option<u64>,
    window_summaries: VecDeque<MinuteSummary>,
    day_summaries: VecDeque<MinuteSummary>,
    settings: Option<DensitySettings>,
    smoothed_bid: Option<f64>,
    smoothed_ask: Option<f64>,
    pub thresholds: Thresholds,
}

impl Tracker {
    pub fn reset(&mut self) {
        self.levels.clear();
        self.last_sample = None;
        self.day = None;
        self.window_summaries.clear();
        self.day_summaries.clear();
        self.settings = None;
        self.smoothed_bid = None;
        self.smoothed_ask = None;
        self.thresholds = Thresholds::default();
    }

    /// Compatibility entry point retaining the historical fixed 3× local-median threshold.
    pub fn sample(&mut self, time: u64, bids: &[(f64, f64)], asks: &[(f64, f64)]) -> (Vec<Density>, Vec<Observation>) {
        let settings = DensitySettings {
            auto_threshold: false,
            ..DensitySettings::default()
        };
        self.sample_with_settings(time, bids, asks, &settings)
    }

    /// Samples a book with adaptive thresholds; `time` is Unix milliseconds.
    pub fn sample_with_settings(
        &mut self,
        time: u64,
        bids: &[(f64, f64)],
        asks: &[(f64, f64)],
        settings: &DensitySettings,
    ) -> (Vec<Density>, Vec<Observation>) {
        if self
            .settings
            .as_ref()
            .is_some_and(|previous| previous.calculation_changed(settings))
        {
            self.window_summaries.clear();
            self.day_summaries.clear();
            self.smoothed_bid = None;
            self.smoothed_ask = None;
            self.thresholds = Thresholds::default();
        }
        self.settings = Some(settings.clone());
        if self
            .last_sample
            .is_some_and(|last| time.saturating_sub(last) > 5_000.max((settings.poll_seconds * 3_000.0) as u64))
        {
            self.reset();
            self.settings = Some(settings.clone());
        }
        self.last_sample = Some(time);

        let bids = &bids[..bids.len().min(MAX_BOOK_LEVELS)];
        let asks = &asks[..asks.len().min(MAX_BOOK_LEVELS)];

        let day = time / DAY_MS;
        if self.day != Some(day) {
            self.window_summaries.clear();
            self.day_summaries.clear();
            self.smoothed_bid = None;
            self.smoothed_ask = None;
            self.day = Some(day);
        }

        let bid = bids
            .iter()
            .filter(|(p, q)| p.is_finite() && *p > 0.0 && q.is_finite() && *q > 0.0)
            .map(|v| v.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let ask = asks
            .iter()
            .filter(|(p, q)| p.is_finite() && *p > 0.0 && q.is_finite() && *q > 0.0)
            .map(|v| v.0)
            .fold(f64::INFINITY, f64::min);
        let mid = (bid + ask) / 2.0;
        if !mid.is_finite() || bid >= ask {
            self.reset();
            return (Vec::new(), Vec::new());
        }
        let zone = (ask - bid).max(0.02);
        let mut books = Vec::new();
        for (side, book) in [(Side::Bid, bids), (Side::Ask, asks)] {
            let mut all = valid_quantities(book);
            all.sort_by(f64::total_cmp);
            let local_median = median(&all);
            let mut nearby: Vec<f64> = book
                .iter()
                .filter_map(|&(price, quantity)| {
                    let distance = match side {
                        Side::Bid => mid - price,
                        Side::Ask => price - mid,
                    };
                    (price.is_finite()
                        && price > 0.0
                        && quantity.is_finite()
                        && quantity > 0.0
                        && distance >= 0.0
                        && distance <= settings.max_distance)
                        .then_some(quantity)
                })
                .collect();
            nearby.sort_by(f64::total_cmp);
            books.push((side, book, local_median, nearby));
        }

        let minute = time / MINUTE_MS;
        if self.window_summaries.back().is_none_or(|s| s.minute != minute) {
            let (_, _, _, bid_values) = &books[0];
            let (_, _, _, ask_values) = &books[1];
            let bid_typical = median(bid_values);
            let ask_typical = median(ask_values);
            let summary = MinuteSummary {
                minute,
                bid_median: bid_typical.max(0.0),
                ask_median: ask_typical.max(0.0),
                bid_percentile: quantile(bid_values, settings.percentile / 100.0)
                    .min(bid_typical * settings.strength_multiplier),
                ask_percentile: quantile(ask_values, settings.percentile / 100.0)
                    .min(ask_typical * settings.strength_multiplier),
            };
            self.window_summaries.push_back(summary);
            self.day_summaries.push_back(summary);
            while self.day_summaries.len() > MAX_SUMMARIES {
                self.day_summaries.pop_front();
            }
        }
        let first_minute = minute.saturating_sub(settings.window_minutes.saturating_sub(1) as u64);
        while self.window_summaries.front().is_some_and(|s| s.minute < first_minute) {
            self.window_summaries.pop_front();
        }

        let bid_threshold = self.smooth_threshold(
            Side::Bid,
            self.threshold_for(Side::Bid, median(&books[0].3), settings),
            settings.auto_threshold,
        );
        let ask_threshold = self.smooth_threshold(
            Side::Ask,
            self.threshold_for(Side::Ask, median(&books[1].3), settings),
            settings.auto_threshold,
        );
        let activity_ratio = self.activity_ratio(median(&books[0].3), median(&books[1].3));
        self.thresholds = Thresholds {
            bid: bid_threshold,
            ask: ask_threshold,
            warming_up: self.window_summaries.len() < 5,
            activity_ratio,
        };

        let mut next = BTreeMap::new();
        let mut events = Vec::new();
        for (side, book, local_median, _) in books {
            if local_median <= 0.0 || !local_median.is_finite() {
                continue;
            }
            let threshold = match side {
                Side::Bid => bid_threshold,
                Side::Ask => ask_threshold,
            };
            for &(price, quantity) in book {
                if !price.is_finite() || price <= 0.0 || !quantity.is_finite() || quantity <= 0.0 {
                    continue;
                }
                let distance = match side {
                    Side::Bid => mid - price,
                    Side::Ask => price - mid,
                };
                if distance < 0.0 || distance > settings.max_distance {
                    continue;
                }
                let key = (side, price.to_bits());
                let previous = self.levels.remove(&key);
                let keep_threshold = if previous.is_some() {
                    (threshold * 0.75).max(settings.min_quantity)
                } else {
                    threshold.max(settings.min_quantity)
                };
                if quantity < keep_threshold {
                    continue;
                }
                let strength = quantity / local_median;
                let mut row = previous.unwrap_or_else(|| {
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
                        max_quantity: quantity,
                        quantity_change_pct: 0.0,
                        initial_quantity: quantity,
                        armed: distance > zone,
                        awaiting_reaction: false,
                    }
                });
                row.quantity = quantity;
                row.notional = price * quantity;
                row.strength = strength;
                row.last_seen = time;
                row.max_quantity = row.max_quantity.max(quantity);
                row.quantity_change_pct = (quantity / row.initial_quantity - 1.0) * 100.0;
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
                    + 30.0 * ((strength - settings.strength_multiplier) / 7.0).clamp(0.0, 1.0)
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

    fn threshold_for(&self, side: Side, local_median: f64, settings: &DensitySettings) -> f64 {
        if !local_median.is_finite() || local_median <= 0.0 {
            return settings.min_quantity;
        }
        let window_percentiles = self
            .window_summaries
            .iter()
            .filter_map(|s| match side {
                Side::Bid if s.bid_percentile > 0.0 => Some(s.bid_percentile),
                Side::Ask if s.ask_percentile > 0.0 => Some(s.ask_percentile),
                _ => None,
            })
            .collect::<Vec<_>>();
        let daily_percentiles = self
            .day_summaries
            .iter()
            .filter_map(|s| match side {
                Side::Bid if s.bid_percentile > 0.0 => Some(s.bid_percentile),
                Side::Ask if s.ask_percentile > 0.0 => Some(s.ask_percentile),
                _ => None,
            })
            .collect::<Vec<_>>();
        let daily_medians = self
            .day_summaries
            .iter()
            .filter_map(|s| match side {
                Side::Bid if s.bid_median > 0.0 => Some(s.bid_median),
                Side::Ask if s.ask_median > 0.0 => Some(s.ask_median),
                _ => None,
            })
            .collect::<Vec<_>>();
        let base = if settings.auto_threshold {
            median(&window_percentiles)
                .max(median(&daily_percentiles))
                .max(median(&daily_medians) * settings.strength_multiplier)
                .max(local_median * settings.strength_multiplier)
        } else {
            local_median * settings.strength_multiplier
        };
        base.max(settings.min_quantity)
    }

    fn smooth_threshold(&mut self, side: Side, target: f64, adaptive: bool) -> f64 {
        let previous = match side {
            Side::Bid => &mut self.smoothed_bid,
            Side::Ask => &mut self.smoothed_ask,
        };
        if !adaptive {
            *previous = Some(target);
            return target;
        }
        let smoothed = match *previous {
            Some(value) => {
                let alpha = if target > value { 0.35 } else { 0.15 };
                value + alpha * (target - value)
            }
            None => target,
        };
        *previous = Some(smoothed);
        smoothed
    }

    fn activity_ratio(&self, bid: f64, ask: f64) -> f64 {
        let baseline_bid = median(
            &self
                .day_summaries
                .iter()
                .map(|s| s.bid_median)
                .filter(|v| *v > 0.0)
                .collect::<Vec<_>>(),
        );
        let baseline_ask = median(
            &self
                .day_summaries
                .iter()
                .map(|s| s.ask_median)
                .filter(|v| *v > 0.0)
                .collect::<Vec<_>>(),
        );
        let mut ratios = Vec::new();
        if bid > 0.0 && baseline_bid > 0.0 {
            ratios.push(bid / baseline_bid);
        }
        if ask > 0.0 && baseline_ask > 0.0 {
            ratios.push(ask / baseline_ask);
        }
        if ratios.is_empty() {
            1.0
        } else {
            ratios.iter().sum::<f64>() / ratios.len() as f64
        }
    }
}

fn valid_quantities(book: &[(f64, f64)]) -> Vec<f64> {
    book.iter()
        .filter_map(|&(price, quantity)| {
            (price.is_finite() && price > 0.0 && quantity.is_finite() && quantity > 0.0).then_some(quantity)
        })
        .collect()
}

fn median(sorted_or_unsorted: &[f64]) -> f64 {
    if sorted_or_unsorted.is_empty() {
        return 0.0;
    }
    let mut sorted = sorted_or_unsorted.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 0 {
        sorted[n / 2 - 1] / 2.0 + sorted[n / 2] / 2.0
    } else {
        sorted[n / 2]
    }
}

fn quantile(sorted_or_unsorted: &[f64], q: f64) -> f64 {
    if sorted_or_unsorted.is_empty() {
        return 0.0;
    }
    let mut sorted = sorted_or_unsorted.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_books_reset_adaptive_history() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings::default();
        let (bids, asks) = book(10.0);
        for second in 0..=5 * 60 {
            tracker.sample_with_settings(second * 1_000, &bids, &asks, &settings);
        }
        assert!(!tracker.thresholds.warming_up);
        assert!(tracker
            .sample_with_settings(301_000, &[], &asks, &settings)
            .0
            .is_empty());
        tracker.sample_with_settings(302_000, &bids, &asks, &settings);
        assert!(tracker.thresholds.warming_up);
        let crossed = [(asks[0].0 + 1.0, 10.0)];
        assert!(tracker
            .sample_with_settings(303_000, &crossed, &asks, &settings)
            .0
            .is_empty());
        assert!(tracker.day_summaries.is_empty());
        assert!(tracker.last_sample.is_none());
    }

    #[test]
    fn unchanged_book_activity_is_one() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings::default();
        let (bids, asks) = book(10.0);
        tracker.sample_with_settings(1_000, &bids, &asks, &settings);
        assert!((tracker.thresholds.activity_ratio - 1.0).abs() < 1e-9);
        tracker.sample_with_settings(2_000, &bids, &asks, &settings);
        assert!((tracker.thresholds.activity_ratio - 1.0).abs() < 1e-9);
    }

    fn sample(tracker: &mut Tracker, time: u64, best: f64, quantity: f64) -> Vec<Density> {
        let bids = if best == 99.0 {
            vec![(best, quantity), (98.0, 1.0), (97.0, 1.0)]
        } else {
            vec![(best, 1.0), (99.0, quantity), (98.0, 1.0)]
        };
        tracker
            .sample(time, &bids, &[(best + 0.1, 1.0), (102.0, 1.0), (103.0, 1.0)])
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

    type Book = (Vec<(f64, f64)>, Vec<(f64, f64)>);

    fn book(side_size: f64) -> Book {
        (
            vec![(99.9, 1.0), (99.0, side_size), (98.0, 1.0)],
            vec![(100.1, 1.0), (101.0, side_size), (102.0, 1.0)],
        )
    }

    fn flat_book(size: f64, outlier: f64) -> Book {
        (
            vec![(99.9, size), (99.8, size), (99.0, size), (98.0, outlier)],
            vec![(100.1, size), (100.2, size), (101.0, size), (102.0, outlier)],
        )
    }

    #[test]
    fn adaptive_threshold_tracks_liquidity_and_ignores_single_outlier() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings::default();
        for minute in 0..20 {
            let size = if minute < 10 { 10.0 } else { 30.0 };
            for second in 0..60 {
                let (bids, asks) = flat_book(size, size);
                tracker.sample_with_settings(minute * MINUTE_MS + second * 1_000, &bids, &asks, &settings);
            }
        }
        let ordinary_threshold = tracker.thresholds.bid;
        let (bids, asks) = flat_book(30.0, 10_000.0);
        tracker.sample_with_settings(20 * MINUTE_MS, &bids, &asks, &settings);
        assert!(tracker.thresholds.bid <= ordinary_threshold * 1.1);
        assert!((tracker.thresholds.activity_ratio - 1.0).abs() < 0.1);
        let (bids, asks) = flat_book(30.0, 30.0);
        tracker.sample_with_settings(21 * MINUTE_MS, &bids, &asks, &settings);
        assert!(tracker.thresholds.bid >= 30.0);
        assert!((tracker.thresholds.activity_ratio - 1.0).abs() < 0.1);
    }

    #[test]
    fn auto_threshold_has_warmup_and_hysteresis_keeps_surviving_levels() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings::default();
        let (bids, asks) = book(10.0);
        let initial = tracker.sample_with_settings(1_000, &bids, &asks, &settings).0;
        assert!(tracker.thresholds.warming_up);
        let threshold = tracker.thresholds.bid;
        let (bids, asks) = book(threshold * 0.8);
        let rows = tracker.sample_with_settings(2_000, &bids, &asks, &settings).0;
        assert!(rows.iter().any(|r| r.side == Side::Bid));
        assert_eq!(
            initial.iter().find(|r| r.side == Side::Bid).unwrap().first_seen,
            rows.iter().find(|r| r.side == Side::Bid).unwrap().first_seen
        );
    }

    #[test]
    fn settings_changes_clear_baseline_and_manual_mode_uses_configured_multiplier() {
        let mut tracker = Tracker::default();
        let mut settings = DensitySettings {
            auto_threshold: false,
            strength_multiplier: 5.0,
            ..DensitySettings::default()
        };
        let (bids, asks) = book(10.0);
        tracker.sample_with_settings(1_000, &bids, &asks, &settings);
        assert_eq!(tracker.thresholds.bid, 5.0);

        settings.percentile = 90.0;
        tracker.sample_with_settings(2_000, &bids, &asks, &settings);
        assert!(tracker.thresholds.warming_up);
        assert_eq!(tracker.window_summaries.len(), 1);
        assert_eq!(tracker.day_summaries.len(), 1);
    }

    #[test]
    fn minimum_quantity_is_an_absolute_hysteresis_floor() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings {
            auto_threshold: false,
            min_quantity: 5.0,
            ..DensitySettings::default()
        };
        let (bids, asks) = book(6.0);
        assert!(tracker
            .sample_with_settings(1_000, &bids, &asks, &settings)
            .0
            .iter()
            .any(|row| { row.side == Side::Bid && row.price == 99.0 }));
        assert!(tracker.thresholds.bid >= settings.min_quantity);

        let (bids, asks) = book(4.0);
        assert!(!tracker
            .sample_with_settings(2_000, &bids, &asks, &settings)
            .0
            .iter()
            .any(|row| { row.side == Side::Bid && row.price == 99.0 }));
    }

    #[test]
    fn display_and_poll_settings_keep_density_age_touches_and_baselines() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings::default();
        let bids = [(99.9, 1.0), (99.0, 10.0), (98.9, 1.0)];
        let asks = [(100.1, 1.0), (101.0, 1.0), (102.0, 1.0)];
        let initial = tracker.sample_with_settings(1_000, &bids, &asks, &settings).0;
        let density = initial
            .iter()
            .find(|row| row.side == Side::Bid && row.price == 99.0)
            .unwrap();
        assert_eq!(density.touches, 0);

        let changed = DensitySettings {
            bid_color: "#123456".into(),
            sort: "distance".into(),
            poll_seconds: 2.0,
            recording: false,
            retention_days: 9,
            ..settings
        };
        let touch_bids = [(99.0, 10.0), (98.9, 1.0), (98.8, 1.0)];
        let touch_asks = [(99.1, 1.0), (99.2, 1.0), (99.3, 1.0)];
        let rows = tracker
            .sample_with_settings(2_000, &touch_bids, &touch_asks, &changed)
            .0;
        let density = rows
            .iter()
            .find(|row| row.side == Side::Bid && row.price == 99.0)
            .unwrap();
        assert_eq!(density.first_seen, 1_000);
        assert_eq!(density.touches, 1);
        assert_eq!(tracker.window_summaries.len(), 1);
        assert_eq!(tracker.day_summaries.len(), 1);
    }

    #[test]
    fn daily_baseline_outlives_the_shorter_rolling_window() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings {
            window_minutes: 5,
            ..DensitySettings::default()
        };
        let (bids, asks) = flat_book(10.0, 10.0);
        for second in 0..=10 * 60 {
            tracker.sample_with_settings(second * 1_000, &bids, &asks, &settings);
        }
        assert_eq!(tracker.window_summaries.len(), 5);
        assert_eq!(tracker.day_summaries.len(), 11);
    }

    #[test]
    fn daily_rollover_and_source_gap_clear_adaptive_history() {
        let mut tracker = Tracker::default();
        let settings = DensitySettings::default();
        let (bids, asks) = book(10.0);
        for second in 0..=5 * 60 {
            tracker.sample_with_settings(second * 1_000, &bids, &asks, &settings);
        }
        assert!(!tracker.thresholds.warming_up);
        tracker.sample_with_settings(DAY_MS, &bids, &asks, &settings);
        assert!(tracker.thresholds.warming_up);
        tracker.sample_with_settings(DAY_MS + 6_000, &bids, &asks, &settings);
        assert!(tracker.thresholds.warming_up);
    }
}
