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
    let mut grade = own;
    if let Some(speech) = speech
        && speech.number() < grade.number()
    {
        grade = speech;
    }
    if peeked && grade.number() > Grade::Hard.number() {
        grade = Grade::Hard;
    }
    let share = if peeked {
        0.0
    } else if speech.is_some() || mode == Mode::Listen {
        1.0
    } else {
        SCORE_ONLY_SHARE
    };
    Counted { grade, share }
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
    use super::*;
    use Grade::*;

    fn full(grade: Grade) -> Counted {
        Counted { grade, share: 1.0 }
    }

    /// A card answered "Gaat goed" four times, each time when it fell due:
    /// last pause 7 weeks. Returns the card and the moment it is due again.
    fn practised_card(scheduler: &Scheduler) -> (rs_fsrs::Card, DateTime<Utc>) {
        let mut now = Utc::now();
        let mut card = new_schedule(now);
        for _ in 0..4 {
            card = scheduler.apply(&card, now, full(Comfortable));
            now = card.due;
        }
        assert_eq!(card.state, State::Review);
        (card, now)
    }

    fn pause(scheduler: &Scheduler, card: &rs_fsrs::Card, now: DateTime<Utc>, counted: Counted) -> Duration {
        scheduler.apply(card, now, counted).due - now
    }

    #[test]
    fn speaking_and_scoring_counts_in_full() {
        assert_eq!(count(Easy, Some(Easy), false, Mode::Read), full(Easy));
        assert_eq!(count(Comfortable, Some(Easy), false, Mode::Read), full(Comfortable));
    }

    #[test]
    fn a_lower_pronunciation_grade_replaces_the_score() {
        assert_eq!(count(Easy, Some(Hard), false, Mode::Read), full(Hard));
        assert_eq!(count(Easy, Some(DontKnow), false, Mode::Read), full(DontKnow));
    }

    #[test]
    fn a_score_alone_keeps_part_of_the_growth() {
        assert_eq!(count(Comfortable, None, false, Mode::Read), Counted { grade: Comfortable, share: SCORE_ONLY_SHARE });
    }

    #[test]
    fn luisteren_has_no_reading_test_so_its_scores_count_in_full() {
        assert_eq!(count(Easy, None, false, Mode::Listen), full(Easy));
    }

    #[test]
    fn opening_the_meaning_caps_the_score_and_stops_the_growth() {
        let peeked = Counted { grade: Hard, share: 0.0 };
        assert_eq!(count(Easy, None, true, Mode::Read), peeked);
        assert_eq!(count(Comfortable, Some(Easy), true, Mode::Read), peeked, "speaking does not undo a look");
        assert_eq!(count(Easy, None, true, Mode::Listen), peeked);
        assert_eq!(count(Hard, None, true, Mode::Read), peeked);
    }

    #[test]
    fn weet_ik_niet_always_counts_in_full() {
        for mode in [Mode::Read, Mode::Listen] {
            for peeked in [false, true] {
                assert_eq!(count(DontKnow, None, peeked, mode), full(DontKnow));
            }
        }
    }

    #[test]
    fn harder_grades_never_come_back_later() {
        let scheduler = Scheduler::exact();
        let now = Utc::now();
        let mut card = new_schedule(now);
        for _ in 0..4 {
            for share in [1.0, SCORE_ONLY_SHARE, 0.0] {
                let gaps = Grade::ALL.map(|grade| pause(&scheduler, &card, now, Counted { grade, share }));
                assert!(gaps.windows(2).all(|pair| pair[0] <= pair[1]), "share {share}: {gaps:?}");
            }
            card = scheduler.apply(&card, now, full(Comfortable));
        }
    }

    #[test]
    fn less_trust_means_a_shorter_pause() {
        let scheduler = Scheduler::exact();
        let (card, now) = practised_card(&scheduler);
        let spoke = pause(&scheduler, &card, now, full(Comfortable));
        let score_only = pause(&scheduler, &card, now, Counted { grade: Comfortable, share: SCORE_ONLY_SHARE });
        let last = Duration::days(card.scheduled_days);
        assert!(last < score_only && score_only < spoke, "{last} < {score_only} < {spoke}");
    }

    #[test]
    fn no_growth_keeps_the_last_pause() {
        let scheduler = Scheduler::exact();
        let (card, now) = practised_card(&scheduler);
        let peeked = pause(&scheduler, &card, now, count(Easy, None, true, Mode::Read));
        let last = Duration::days(card.scheduled_days);
        assert!((peeked - last).num_days().abs() <= 1, "{peeked} vs {last}");
    }

    #[test]
    fn a_miss_is_never_softened() {
        let scheduler = Scheduler::exact();
        let (card, now) = practised_card(&scheduler);
        let missed = pause(&scheduler, &card, now, count(DontKnow, None, false, Mode::Read));
        assert!(missed <= Duration::minutes(10));
    }

    #[test]
    fn a_new_card_is_scheduled_by_its_grade_alone() {
        let scheduler = Scheduler::exact();
        let now = Utc::now();
        let card = new_schedule(now);
        for grade in Grade::ALL {
            let shares = [1.0, SCORE_ONLY_SHARE, 0.0].map(|share| pause(&scheduler, &card, now, Counted { grade, share }));
            assert!(shares.iter().all(|gap| *gap == shares[0]), "{grade:?}: {shares:?}");
        }
    }

    #[test]
    fn dont_know_comes_back_within_the_session() {
        let scheduler = Scheduler::default();
        let now = Utc::now();
        let card = scheduler.apply(&new_schedule(now), now, full(DontKnow));
        assert!(card.due - now <= Duration::minutes(20));
    }

    #[test]
    fn repeated_success_stretches_the_gap() {
        for share in [1.0, SCORE_ONLY_SHARE] {
            let scheduler = Scheduler::default();
            let mut now = Utc::now();
            let mut card = new_schedule(now);
            let mut last_gap = Duration::zero();
            for _ in 0..6 {
                card = scheduler.apply(&card, now, Counted { grade: Comfortable, share });
                let gap = card.due - now;
                assert!(gap >= last_gap, "share {share}");
                last_gap = gap;
                now = card.due;
            }
            assert!(last_gap > Duration::days(7), "share {share}: {last_gap}");
        }
    }

    #[test]
    fn describes_gaps_in_dutch() {
        assert_eq!(describe(Duration::minutes(10)), "10 min");
        assert_eq!(describe(Duration::hours(1)), "1 uur");
        assert_eq!(describe(Duration::days(1)), "1 dag");
        assert_eq!(describe(Duration::days(5)), "5 dagen");
        assert_eq!(describe(Duration::days(21)), "3 weken");
        assert_eq!(describe(Duration::days(90)), "3 maanden");
        assert_eq!(describe(Duration::days(400)), "1 jaar");
    }
}
