//! Spaced repetition with FSRS, the scheduler modern Anki uses.
//!
//! On top of FSRS, what the learner did before scoring decides how far their
//! score is trusted: reading the card aloud backs it up, scoring alone counts
//! for less, and opening the meaning counts against it.

use chrono::{DateTime, Duration, Utc};
use rs_fsrs::{FSRS, Parameters, Seed, State};

use crate::model::Grade;
use crate::store::Mode;

/// The share of the longer pause a score earns when nothing backs it up.
pub const SCORE_ONLY_SHARE: f64 = 0.7;

/// How a review counts towards the card's schedule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Counted {
    /// The grade that schedules the card.
    pub grade: Grade,
    /// How much of the stability gain FSRS proposes the card keeps, 0.0–1.0.
    pub share: f64,
}

/// Turns the learner's own score into how the review counts.
///
/// - Their own score is the starting point.
/// - A pronunciation grade below it replaces it.
/// - Opening the meaning caps it at Moeilijk and stops the pause from growing.
/// - In Lezen, a score without a reading test keeps only part of the growth.
///   Luisteren has no reading test, so its scores count in full.
/// - "Weet ik niet" always counts in full: nobody claims a miss by accident.
pub fn count(own: Grade, speech: Option<Grade>, peeked: bool, mode: Mode) -> Counted {
    if own == Grade::DontKnow {
        return Counted { grade: own, share: 1.0 };
    }
    let grade = speech.map_or(own, |speech| speech.min(own));
    if peeked {
        return Counted { grade: grade.min(Grade::Hard), share: 0.0 };
    }
    let backed_up = speech.is_some() || mode == Mode::Listen;
    Counted { grade, share: if backed_up { 1.0 } else { SCORE_ONLY_SHARE } }
}

pub struct Scheduler {
    fsrs: FSRS,
    parameters: Parameters,
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler::new(true)
    }
}

impl Scheduler {
    /// Short-term steps make "Weet ik niet" bring the card back within minutes.
    /// Fuzz spreads cards learned on the same day so they don't all fall due together.
    fn new(fuzz: bool) -> Self {
        let parameters = Parameters { enable_short_term: true, enable_fuzz: fuzz, ..Parameters::default() };
        Scheduler { fsrs: FSRS::new(parameters.clone()), parameters }
    }

    /// Without fuzz, so tests get the same pauses every run.
    #[cfg(test)]
    pub(crate) fn exact() -> Self {
        Scheduler::new(false)
    }

    /// The state after a review at `now`.
    ///
    /// FSRS proposes a new stability for the grade. When the review earned
    /// less than full trust and the stability would grow, the card keeps only
    /// `share` of the growth, and the pause is worked out again from that.
    pub fn apply(&self, card: &rs_fsrs::Card, now: DateTime<Utc>, counted: Counted) -> rs_fsrs::Card {
        let mut next = self.fsrs.next(card.clone(), now, counted.grade.rating()).card;
        let gain = next.stability - card.stability;
        // A new card has no stability to grow from yet.
        if counted.share >= 1.0 || card.state == State::New || gain <= 0.0 {
            return next;
        }
        next.stability = card.stability + counted.share.max(0.0) * gain;
        // Learning steps are minutes long and stay as FSRS set them.
        if next.state == State::Review {
            let mut parameters = self.parameters.clone();
            // The seed FSRS itself uses, so fuzz spreads these cards the same way.
            parameters.seed =
                Seed::new(format!("{}_{}_{}", now.timestamp_millis(), card.reps, card.difficulty * card.stability));
            let days = parameters.next_interval(next.stability, next.elapsed_days) as i64;
            next.scheduled_days = days;
            next.due = now + Duration::days(days);
        }
        next
    }
}

/// A fresh, never-studied schedule.
pub fn new_schedule(now: DateTime<Utc>) -> rs_fsrs::Card {
    rs_fsrs::Card {
        due: now,
        last_review: now,
        ..rs_fsrs::Card::new()
    }
}

/// "10 min", "1 dag", "3 weken", in Dutch.
pub fn describe(gap: Duration) -> String {
    let minutes = gap.num_minutes().max(1);
    let plural = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let hours = (minutes + 30) / 60;
    if hours < 24 {
        return plural(hours, "uur", "uur");
    }
    let days = (hours + 12) / 24;
    if days < 14 {
        return plural(days, "dag", "dagen");
    }
    if days < 60 {
        return plural((days + 3) / 7, "week", "weken");
    }
    if days < 365 {
        return plural((days + 15) / 30, "maand", "maanden");
    }
    plural((days + 182) / 365, "jaar", "jaar")
}

/// "vandaag", "morgen", "over 5 dagen", for the deck list.
pub fn describe_due(due: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let gap = due - now;
    if gap <= Duration::zero() {
        return "nu".into();
    }
    if gap < Duration::hours(20) {
        return "vandaag".into();
    }
    if gap < Duration::hours(44) {
        return "morgen".into();
    }
    format!("over {}", describe(gap))
}

#[cfg(test)]
mod tests {
    use super::{Counted, DateTime, Duration, Grade, Mode, SCORE_ONLY_SHARE, Scheduler, State, Utc, count, describe, describe_due, new_schedule};
    use Grade::{Comfortable, DontKnow, Easy, Hard};

    fn full(grade: Grade) -> Counted {
        Counted { grade, share: 1.0 }
    }

    fn pause(scheduler: &Scheduler, card: &rs_fsrs::Card, now: DateTime<Utc>, counted: Counted) -> Duration {
        scheduler.apply(card, now, counted).due - now
    }

    #[test]
    fn how_a_review_counts() {
        let (read, listen) = (Mode::Read, Mode::Listen);
        let cases = [
            // own score, pronunciation, meaning opened, mode => grade, share
            (Easy, Some(Easy), false, read, Easy, 1.0),
            (Comfortable, Some(Easy), false, read, Comfortable, 1.0),
            (Easy, Some(Hard), false, read, Hard, 1.0),
            (Easy, Some(DontKnow), false, read, DontKnow, 1.0),
            (Comfortable, None, false, read, Comfortable, SCORE_ONLY_SHARE),
            (Easy, None, false, listen, Easy, 1.0),
            (Easy, None, true, read, Hard, 0.0),
            (Comfortable, Some(Easy), true, read, Hard, 0.0),
            (Easy, None, true, listen, Hard, 0.0),
            (Hard, None, true, read, Hard, 0.0),
            (DontKnow, None, true, listen, DontKnow, 1.0),
            (DontKnow, None, false, read, DontKnow, 1.0),
        ];
        for (own, speech, peeked, mode, grade, share) in cases {
            assert_eq!(count(own, speech, peeked, mode), Counted { grade, share }, "{own:?} {speech:?} {peeked} {mode:?}");
        }
    }

    #[test]
    fn a_new_card_is_scheduled_by_its_grade_alone_and_harder_grades_come_sooner() {
        let scheduler = Scheduler::exact();
        let now = Utc::now();
        let mut card = new_schedule(now);
        assert!(pause(&scheduler, &card, now, full(DontKnow)) <= Duration::minutes(20), "back within the session");
        for grade in Grade::ALL {
            let shares = [1.0, SCORE_ONLY_SHARE, 0.0].map(|share| pause(&scheduler, &card, now, Counted { grade, share }));
            assert!(shares.iter().all(|gap| *gap == shares[0]), "{grade:?}: {shares:?}");
        }
        for _ in 0..4 {
            for share in [1.0, SCORE_ONLY_SHARE, 0.0] {
                let gaps = Grade::ALL.map(|grade| pause(&scheduler, &card, now, Counted { grade, share }));
                assert!(gaps.is_sorted(), "share {share}: {gaps:?}");
            }
            card = scheduler.apply(&card, now, full(Comfortable));
        }
    }

    /// On a card answered "Gaat goed" four times, each when it fell due: a miss
    /// comes back within minutes, a look at the meaning keeps the last pause,
    /// and a score alone grows it less than a score backed by reading aloud.
    #[test]
    fn trust_decides_how_far_a_known_card_moves() {
        let scheduler = Scheduler::exact();
        let mut now = Utc::now();
        let mut card = new_schedule(now);
        for _ in 0..4 {
            card = scheduler.apply(&card, now, full(Comfortable));
            now = card.due;
        }
        assert_eq!(card.state, State::Review);
        let last = Duration::days(card.scheduled_days);
        let after = |counted| pause(&scheduler, &card, now, counted);
        assert!(after(count(DontKnow, None, false, Mode::Read)) <= Duration::minutes(10));
        assert!((after(count(Easy, None, true, Mode::Read)) - last).num_days().abs() <= 1);
        let (score_only, spoke) = (after(count(Comfortable, None, false, Mode::Read)), after(full(Comfortable)));
        assert!(last < score_only && score_only < spoke, "{last} < {score_only} < {spoke}");
    }

    #[test]
    fn describes_gaps_and_due_dates_in_dutch() {
        let now = Utc::now();
        let gaps = [(Duration::minutes(10), "10 min"), (Duration::hours(1), "1 uur"), (Duration::days(1), "1 dag"),
            (Duration::days(5), "5 dagen"), (Duration::days(21), "3 weken"), (Duration::days(90), "3 maanden"),
            (Duration::days(400), "1 jaar")];
        for (gap, text) in gaps {
            assert_eq!(describe(gap), text);
        }
        let dues = [(Duration::zero(), "nu"), (Duration::hours(5), "vandaag"), (Duration::hours(30), "morgen"),
            (Duration::days(5), "over 5 dagen")];
        for (gap, text) in dues {
            assert_eq!(describe_due(now + gap, now), text);
        }
    }
}
