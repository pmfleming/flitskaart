//! Spaced repetition with FSRS, the scheduler modern Anki uses.
//!
//! The learner's own score always schedules the card. Reading aloud, writing
//! and opening the meaning only shape the score the app suggests; whether the
//! learner tested the card decides how far their score is trusted.

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

/// The weaker of the two tests, reading aloud and writing, of those taken.
pub fn weaker_test(speech: Option<Grade>, written: Option<Grade>) -> Option<Grade> {
    [speech, written].into_iter().flatten().min()
}

/// The score the app suggests: the weaker test, and Moeilijk at best once
/// the meaning was opened. `None` when there is nothing to go on.
pub fn suggest(tested: Option<Grade>, peeked: bool) -> Option<Grade> {
    let cap = if peeked { Grade::Hard } else { Grade::Easy };
    match tested {
        Some(tested) => Some(tested.min(cap)),
        None => peeked.then_some(cap),
    }
}

/// Turns the learner's own score into how the review counts.
///
/// - Their own score is final: the tests only suggest one.
/// - A score after a test (reading aloud or writing) counts in full, whatever
///   the test showed. Luisteren plays the sentence first, so its scores count
///   in full too.
/// - In Lezen, a score without a test keeps only part of the growth.
/// - "Weet ik niet" always counts in full: nobody claims a miss by accident.
pub fn count(own: Grade, tested: bool, mode: Mode) -> Counted {
    let backed_up = own == Grade::DontKnow || tested || mode == Mode::Listen;
    Counted { grade: own, share: if backed_up { 1.0 } else { SCORE_ONLY_SHARE } }
}

pub struct Scheduler {
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
        Scheduler { parameters: Parameters { enable_short_term: true, enable_fuzz: fuzz, ..Parameters::default() } }
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
        apply(&self.parameters, card, now, counted)
    }

    /// Roughly how long until the card comes back after this review, for the
    /// score buttons. Without fuzz, so it doesn't change while the learner looks.
    pub fn gap(&self, card: &rs_fsrs::Card, now: DateTime<Utc>, counted: Counted) -> Duration {
        let parameters = Parameters { enable_fuzz: false, ..self.parameters.clone() };
        apply(&parameters, card, now, counted).due - now
    }
}

/// [`Scheduler::apply`] with the given FSRS parameters.
fn apply(parameters: &Parameters, card: &rs_fsrs::Card, now: DateTime<Utc>, counted: Counted) -> rs_fsrs::Card {
    let mut next = FSRS::new(parameters.clone()).next(card.clone(), now, counted.grade.rating()).card;
    let gain = next.stability - card.stability;
    // A new card has no stability to grow from yet.
    if counted.share >= 1.0 || card.state == State::New || gain <= 0.0 {
        return next;
    }
    next.stability = card.stability + counted.share.max(0.0) * gain;
    // Learning steps are minutes long and stay as FSRS set them.
    if next.state == State::Review {
        let mut parameters = parameters.clone();
        // The seed FSRS itself uses, so fuzz spreads these cards the same way.
        parameters.seed =
            Seed::new(format!("{}_{}_{}", now.timestamp_millis(), card.reps, card.difficulty * card.stability));
        let days = parameters.next_interval(next.stability, next.elapsed_days) as i64;
        next.scheduled_days = days;
        next.due = now + Duration::days(days);
    }
    next
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

/// "over 7 min", "vandaag", "morgen", "over 5 dagen", for the deck list and
/// the rest screen.
pub fn describe_due(due: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let gap = due - now;
    if gap <= Duration::zero() {
        return "nu".into();
    }
    if gap < Duration::hours(1) {
        return format!("over {}", describe(gap));
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
    use super::{Counted, DateTime, Duration, Grade, Mode, SCORE_ONLY_SHARE, Scheduler, State, Utc, count, describe, describe_due, new_schedule, suggest};    use Grade::{Comfortable, DontKnow, Easy, Hard};

    fn full(grade: Grade) -> Counted {
        Counted { grade, share: 1.0 }
    }

    fn pause(scheduler: &Scheduler, card: &rs_fsrs::Card, now: DateTime<Utc>, counted: Counted) -> Duration {
        scheduler.apply(card, now, counted).due - now
    }

    #[test]
    fn the_own_score_counts_and_a_test_earns_full_trust() {
        let (read, listen) = (Mode::Read, Mode::Listen);
        let cases = [
            // own score, tested, mode => share
            (Easy, true, read, 1.0),
            (Comfortable, true, read, 1.0),
            (Comfortable, false, read, SCORE_ONLY_SHARE),
            (Easy, false, listen, 1.0),
            (DontKnow, false, read, 1.0),
        ];
        for (own, tested, mode, share) in cases {
            assert_eq!(count(own, tested, mode), Counted { grade: own, share }, "{own:?} {tested} {mode:?}");
        }
    }

    #[test]
    fn the_app_suggests_the_weaker_test_and_moeilijk_at_best_after_a_look() {
        let cases = [
            // weaker test, meaning opened => suggestion
            (Some(Easy), false, Some(Easy)),
            (Some(DontKnow), false, Some(DontKnow)),
            (Some(Easy), true, Some(Hard)),
            (Some(DontKnow), true, Some(DontKnow)),
            (None, true, Some(Hard)),
            (None, false, None),
        ];
        for (tested, peeked, suggestion) in cases {
            assert_eq!(suggest(tested, peeked), suggestion, "{tested:?} {peeked}");
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
    /// comes back within minutes, and a score alone grows the pause less than
    /// a score after a test, however the test went.
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
        assert!(after(count(DontKnow, false, Mode::Read)) <= Duration::minutes(10));
        let (score_only, tested) = (after(count(Comfortable, false, Mode::Read)), after(count(Comfortable, true, Mode::Read)));
        assert!(last < score_only && score_only < tested, "{last} < {score_only} < {tested}");
        assert_eq!(scheduler.gap(&card, now, full(Comfortable)), tested, "the buttons show the pause the score gives");
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
        let dues = [(Duration::zero(), "nu"), (Duration::minutes(7), "over 7 min"), (Duration::hours(5), "vandaag"), (Duration::hours(30), "morgen"),
            (Duration::days(5), "over 5 dagen")];
        for (gap, text) in dues {
            assert_eq!(describe_due(now + gap, now), text);
        }
    }
}
