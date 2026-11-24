//! How long making cards will probably take, learnt from earlier runs.
//!
//! The model says nothing about how far along it is, so the app times each
//! run and remembers how much text went in and which model and effort did it.
//! A longer conversation takes longer, and a slower model or more thinking
//! adds time, so the estimate is a straight line, seconds against characters
//! of text, through the earlier runs, recent ones counting more than old.
//!
//! The line is drawn through all runs first, then redrawn through the runs of
//! the same tool, the same model and finally the same effort, each step only
//! as far as its own runs show: one run of the exact model already counts
//! twice what the wider runs say, so a new model starts from what the others
//! took and soon goes by its own times.

/// One timed run of making cards.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The tool, model and effort, as saved in the settings.
    pub cli: String,
    pub model: String,
    pub effort: String,
    /// Characters of text the cards were made from.
    pub chars: usize,
    pub seconds: f64,
}

/// What to expect of the next run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub seconds: f64,
    /// Earlier runs with the same tool, model and effort.
    pub same_model: usize,
    /// Earlier runs in all.
    pub runs: usize,
}

/// Only this many recent runs are looked at.
pub const RUNS_KEPT: usize = 200;
/// Each older run counts this much less than the one after it.
const AGEING: f64 = 0.95;
/// How much the wider runs count against a narrower group's own: half a run.
const WIDER_WEIGHT: f64 = 0.5;
/// Until the runs show otherwise, every thousand characters add four seconds.
const SECONDS_PER_CHAR: f64 = 0.004;
/// How firmly the slope holds to the wider runs' (or the guess): as much as
/// one run whose length is 2000 characters off the average.
const SLOPE_FIRMNESS: f64 = 2000. * 2000.;
const SHORTEST: f64 = 5.;
const LONGEST: f64 = 900.;

/// Seconds as a straight line in characters.
#[derive(Clone, Copy, Debug)]
struct Line {
    intercept: f64,
    slope: f64,
}

impl Line {
    fn at(self, chars: f64) -> f64 {
        self.intercept + self.slope * chars
    }
}

/// The line through `runs` (weight, characters, seconds), leaning on `wider`
/// as much as the runs are few: unchanged if there are none.
fn refine(wider: Line, runs: &[(f64, f64, f64)], lean: f64) -> Line {
    let total: f64 = runs.iter().map(|(w, _, _)| w).sum();
    if total <= 0. {
        return wider;
    }
    let mean_chars = runs.iter().map(|(w, x, _)| w * x).sum::<f64>() / total;
    let mean_seconds = runs.iter().map(|(w, _, y)| w * y).sum::<f64>() / total;
    let spread: f64 = runs.iter().map(|(w, x, _)| w * (x - mean_chars).powi(2)).sum();
    let together: f64 = runs.iter().map(|(w, x, y)| w * (x - mean_chars) * (y - mean_seconds)).sum();
    // Runs all of a length say nothing about how length matters, so the slope
    // starts from the wider one and moves to what the runs show as they vary.
    let slope = ((together + SLOPE_FIRMNESS * wider.slope) / (spread + SLOPE_FIRMNESS)).max(0.);
    let own = Line { intercept: mean_seconds - slope * mean_chars, slope };
    let share = total / (total + lean);
    Line { intercept: share * own.intercept + (1. - share) * wider.intercept, slope: share * own.slope + (1. - share) * wider.slope }
}

/// How long making cards from `chars` characters with this model and effort
/// will probably take, from `runs` (newest first); `None` before the first run.
pub fn estimate(runs: &[Run], cli: &str, model: &str, effort: &str, chars: usize) -> Option<Estimate> {
    let runs: Vec<(&Run, f64)> = runs
        .iter()
        .take(RUNS_KEPT)
        .enumerate()
        .filter(|(_, run)| run.seconds.is_finite() && run.seconds > 0.)
        .map(|(age, run)| (run, AGEING.powi(age as i32)))
        .collect();
    if runs.is_empty() {
        return None;
    }
    let group = |keep: &dyn Fn(&Run) -> bool| -> Vec<(f64, f64, f64)> {
        runs.iter().filter(|(run, _)| keep(run)).map(|(run, w)| (*w, run.chars as f64, run.seconds)).collect()
    };
    let guess = Line { intercept: 0., slope: SECONDS_PER_CHAR };
    let line = refine(guess, &group(&|_| true), 0.);
    let line = refine(line, &group(&|run| run.cli == cli), WIDER_WEIGHT);
    let line = refine(line, &group(&|run| run.cli == cli && run.model == model), WIDER_WEIGHT);
    let exact = group(&|run| run.cli == cli && run.model == model && run.effort == effort);
    let line = refine(line, &exact, WIDER_WEIGHT);
    let seconds = line.at(chars as f64).clamp(SHORTEST, LONGEST);
    Some(Estimate { seconds, same_model: exact.len(), runs: runs.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(model: &str, effort: &str, chars: usize, seconds: f64) -> Run {
        Run { cli: "claude".into(), model: model.into(), effort: effort.into(), chars, seconds }
    }

    fn guess(runs: &[Run], model: &str, effort: &str, chars: usize) -> f64 {
        estimate(runs, "claude", model, effort, chars).unwrap().seconds
    }

    #[test]
    fn nothing_to_go_on_before_the_first_run() {
        assert_eq!(estimate(&[], "claude", "opus", "high", 1000), None);
    }

    #[test]
    fn one_run_is_expected_again_with_a_little_more_for_longer_text() {
        let runs = [run("opus", "high", 1000, 30.)];
        assert!((guess(&runs, "opus", "high", 1000) - 30.).abs() < 1e-9);
        assert!((guess(&runs, "opus", "high", 3000) - 38.).abs() < 1e-9, "four seconds a thousand characters");
        assert_eq!(estimate(&runs, "claude", "opus", "high", 1000).unwrap().same_model, 1);
    }

    #[test]
    fn longer_conversations_are_learnt_to_take_longer() {
        // Ten seconds plus twenty a thousand characters.
        let runs: Vec<Run> = [500, 4000, 1500, 6000, 2500, 800].iter().map(|&n| run("opus", "high", n, 10. + 0.02 * n as f64)).collect();
        let long = guess(&runs, "opus", "high", 8000);
        let short = guess(&runs, "opus", "high", 300);
        assert!((long - 170.).abs() < 25., "{long}");
        assert!((short - 16.).abs() < 6., "{short}");
    }

    #[test]
    fn the_same_model_and_effort_count_most() {
        let mut runs = vec![run("haiku", "low", 2000, 12.); 6];
        runs.extend([run("opus", "max", 2000, 90.), run("opus", "max", 2000, 100.)]);
        let opus = guess(&runs, "opus", "max", 2000);
        let haiku = guess(&runs, "haiku", "low", 2000);
        assert!(opus > 80., "{opus}");
        assert!(haiku < 20., "{haiku}");
        // Opus at another effort leans on opus at max before haiku.
        let opus_low = guess(&runs, "opus", "low", 2000);
        assert!(opus_low > 50., "{opus_low}");
        let estimate = estimate(&runs, "claude", "opus", "low", 2000).unwrap();
        assert_eq!((estimate.same_model, estimate.runs), (0, 8));
    }

    #[test]
    fn a_new_model_starts_from_the_others_and_soon_goes_by_its_own() {
        let mut runs = vec![run("sonnet", "medium", 1500, 40.); 8];
        assert!((guess(&runs, "fable", "high", 1500) - 40.).abs() < 1e-6);
        runs.insert(0, run("fable", "high", 1500, 100.));
        let once = guess(&runs, "fable", "high", 1500);
        assert!((75. ..100.).contains(&once), "{once}");
        runs.insert(0, run("fable", "high", 1500, 100.));
        assert!(guess(&runs, "fable", "high", 1500) > once);
    }

    #[test]
    fn recent_runs_count_more_than_old_ones() {
        // Newest first: the model got quicker lately.
        let mut runs = vec![run("opus", "high", 1000, 20.); 10];
        runs.extend(vec![run("opus", "high", 1000, 60.); 10]);
        let now = guess(&runs, "opus", "high", 1000);
        assert!(now < 40., "{now}");
    }

    #[test]
    fn the_estimate_stays_sensible() {
        let runs = [run("opus", "high", 1000, 2.), run("opus", "high", 1100, 1.)];
        assert_eq!(guess(&runs, "opus", "high", 10), SHORTEST);
        let runs = [run("opus", "high", 100, 400.), run("opus", "high", 200, 800.)];
        assert_eq!(guess(&runs, "opus", "high", 100_000), LONGEST);
    }
}
