//! Helps choose the warm-up. Each replication counts the parts in the system
//! over its whole length, warm-up included, as time averages over equal bins.
//! Averaged across replications and smoothed with a moving average, that is
//! Welch's plot; the MSER rule (White, 1997) suggests where to cut.
//!
//! With shifts the count follows the week, so bins are days, the moving
//! average spans a week, and MSER cuts in whole weeks, on weekly means.

use serde::Serialize;
use simulancer_engine::spec::Spec;
use simulancer_engine::{Observer, SimTime, TraceEvent};

/// Bins over warm-up plus run length, without shifts.
const BINS: usize = 120;
/// Half-width of the moving average without shifts, in bins.
const WINDOW: usize = 5;
const DAY: f64 = 24.0 * 3600.0;
/// Fewer full weeks than this, with shifts, are too few to judge.
const MIN_WEEKS: usize = 4;

/// The bins a model's check uses: their width in seconds and how many.
fn bins(spec: &Spec) -> (f64, usize) {
    let e = &spec.experiment;
    let end = (e.warmup.as_micros() + e.run_length.as_micros()) as f64 / 1e6;
    if spec.schedules.is_empty() {
        (end / BINS as f64, BINS)
    } else {
        (DAY, (end / DAY).ceil().max(1.0) as usize)
    }
}

/// Counts the parts in the system and integrates the count over time.
pub struct PartsInSystem {
    /// Bin width in seconds.
    width: f64,
    end: f64,
    area: Vec<f64>,
    last: f64,
    count: f64,
}

impl PartsInSystem {
    pub fn new(spec: &Spec) -> Self {
        let e = &spec.experiment;
        let (width, n) = bins(spec);
        PartsInSystem {
            width,
            end: (e.warmup.as_micros() + e.run_length.as_micros()) as f64 / 1e6,
            area: vec![0.0; n],
            last: 0.0,
            count: 0.0,
        }
    }

    fn advance(&mut self, t: f64) {
        let t = t.min(self.end);
        let last_bin = self.area.len() - 1;
        while self.last < t {
            let bin = ((self.last / self.width) as usize).min(last_bin);
            let to = t.min((bin + 1) as f64 * self.width);
            // Rounding can leave `to` at `last` at a bin edge: step past it.
            let to = if to <= self.last { t } else { to };
            self.area[bin] += self.count * (to - self.last);
            self.last = to;
        }
    }

    /// The time-average number of parts in the system in each bin (the last
    /// one may be shorter).
    pub fn finish(mut self) -> Vec<f64> {
        self.advance(self.end);
        let (width, end) = (self.width, self.end);
        self.area
            .into_iter()
            .enumerate()
            .map(|(i, a)| a / width.min(end - i as f64 * width))
            .collect()
    }
}

impl Observer for PartsInSystem {
    #[inline]
    fn record(&mut self, time: SimTime, event: TraceEvent) {
        let change = match event {
            TraceEvent::Created { .. }
            | TraceEvent::Stocked { .. }
            | TraceEvent::Split { .. }
            | TraceEvent::Assembled { .. } => 1.0,
            TraceEvent::Exited { .. }
            | TraceEvent::Lost { .. }
            | TraceEvent::GaveUp { .. }
            | TraceEvent::Consumed { .. } => -1.0,
            _ => return,
        };
        self.advance(time.as_micros() as f64 / 1e6);
        self.count += change;
    }
}

/// Passes every event to two observers.
pub struct Both<'a, A, B>(pub &'a mut A, pub &'a mut B);

impl<A: Observer, B: Observer> Observer for Both<'_, A, B> {
    #[inline]
    fn record(&mut self, time: SimTime, event: TraceEvent) {
        self.0.record(time, event);
        self.1.record(time, event);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WarmupCheck {
    /// What is plotted.
    pub response: &'static str,
    /// Bin width in seconds; bin `i` starts at `i·bin`.
    pub bin: f64,
    /// The mean across replications, per bin.
    pub mean: Vec<f64>,
    /// `mean` smoothed with a centered moving average of `2·window + 1` bins,
    /// as in Welch's method: shorter at the start, none for the last `window`
    /// bins (null). With shifts, none at the start either, where a window
    /// shorter than a week would show the weekly cycle as a start-up.
    pub smoothed: Vec<Option<f64>>,
    pub window: usize,
    /// "bin", or "week" when the model has shifts and MSER cuts in weeks.
    pub cut_by: &'static str,
    /// Where MSER cuts, in seconds (at most half the length); none with shifts
    /// and fewer than four full weeks.
    pub suggested: Option<f64>,
    /// False when the cut is at its limit (half the length): the count may still
    /// be changing at the end, so the run is too short or the system never settles.
    pub settles: Option<bool>,
}

/// Where MSER cuts a series, and whether that's before half of it. At least
/// three points are kept after the cut: on a short series (a few weeks) the
/// rule otherwise favors a two-point tail, whose spread is small by chance.
fn mser(series: &[f64]) -> (usize, bool) {
    let half = series.len() / 2;
    let limit = half.min(series.len().saturating_sub(3));
    let score = |d: usize| {
        let tail = &series[d..];
        let m = tail.iter().sum::<f64>() / tail.len() as f64;
        tail.iter().map(|y| (y - m).powi(2)).sum::<f64>() / (tail.len() as f64).powi(2)
    };
    let cut = (0..=limit)
        .min_by(|&a, &b| score(a).total_cmp(&score(b)))
        .unwrap_or(0);
    (cut, cut < half)
}

/// Welch's plot and MSER's cut from each replication's bins.
pub fn check(spec: &Spec, series: &[Vec<f64>]) -> Option<WarmupCheck> {
    let n = series.first()?.len();
    if n == 0 || series.iter().any(|s| s.len() != n) {
        return None;
    }
    let reps = series.len() as f64;
    let mean: Vec<f64> = (0..n)
        .map(|i| series.iter().map(|s| s[i]).sum::<f64>() / reps)
        .collect();
    let shifts = !spec.schedules.is_empty();
    let window = if shifts { 3 } else { WINDOW };
    let smoothed = (0..n)
        .map(|i| {
            if i + window >= n || (shifts && i < window) {
                return None;
            }
            let w = window.min(i);
            Some(mean[i - w..=i + w].iter().sum::<f64>() / (2 * w + 1) as f64)
        })
        .collect();
    let (bin, _) = bins(spec);
    let (suggested, settles) = if shifts {
        // Weekly means of full weeks; the warm-up goes in whole weeks.
        let weeks: Vec<f64> = mean
            .as_chunks::<7>()
            .0
            .iter()
            .map(|w| w.iter().sum::<f64>() / 7.0)
            .collect();
        if weeks.len() < MIN_WEEKS {
            (None, None)
        } else {
            let (cut, settles) = mser(&weeks);
            (Some(cut as f64 * 7.0 * DAY), Some(settles))
        }
    } else {
        let (cut, settles) = mser(&mean);
        (Some(cut as f64 * bin), Some(settles))
    };
    Some(WarmupCheck {
        response: "parts in the system",
        bin,
        mean,
        smoothed,
        window,
        cut_by: if shifts { "week" } else { "bin" },
        suggested,
        settles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(file: &str) -> Spec {
        simulancer_model::load_str(file).unwrap().spec
    }

    /// A count that starts empty and settles: MSER cuts near where it settles.
    #[test]
    fn mser_cuts_where_the_start_up_ends() {
        let series: Vec<Vec<f64>> = (0..5)
            .map(|r| {
                (0..BINS)
                    .map(|i| {
                        let noise = (((i * 7 + r * 13) % 11) as f64 - 5.0) * 0.1;
                        (if i < 20 { i as f64 / 2.0 } else { 10.0 }) + noise
                    })
                    .collect()
            })
            .collect();
        let spec = spec(include_str!("../../../examples/two-cells.yaml"));
        let check = check(&spec, &series).unwrap();
        let cut = (check.suggested.unwrap() / check.bin).round() as usize;
        assert!((18..=22).contains(&cut), "cut at bin {cut}");
        assert_eq!(check.settles, Some(true));
        assert_eq!(check.cut_by, "bin");
    }

    /// A count that keeps growing never settles.
    #[test]
    fn a_growing_count_never_settles() {
        let series = vec![(0..BINS).map(|i| i as f64).collect::<Vec<_>>(); 3];
        let spec = spec(include_str!("../../../examples/two-cells.yaml"));
        assert_eq!(check(&spec, &series).unwrap().settles, Some(false));
    }

    /// With shifts, bins are days and the cut is in whole weeks: a weekly
    /// cycle alone needs no warm-up, a first week that fills up needs one.
    #[test]
    fn with_shifts_the_cut_is_in_weeks() {
        let spec = spec(include_str!("../../../examples/bank-branch.yaml"));
        let (width, n) = bins(&spec);
        assert_eq!(width, DAY);
        let cycle = |day: usize| if day % 7 < 5 { 4.0 } else { 0.0 };
        let steady = vec![(0..n).map(cycle).collect::<Vec<_>>(); 3];
        let check = check(&spec, &steady).unwrap();
        assert_eq!((check.cut_by, check.suggested), ("week", Some(0.0)));
        let filling = vec![
            (0..n)
                .map(|d| cycle(d) * if d < 7 { 0.3 } else { 1.0 })
                .collect::<Vec<_>>();
            3
        ];
        let check = super::check(&spec, &filling).unwrap();
        assert_eq!(check.suggested, Some(7.0 * DAY));
    }
}
