//! Spaced repetition with FSRS, the scheduler modern Anki uses.

use chrono::{DateTime, Duration, Utc};
use rs_fsrs::{FSRS, Parameters};

use crate::model::Grade;

pub struct Scheduler {
    fsrs: FSRS,
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler {
            // Short-term steps make "Weet ik niet" bring the card back within minutes.
            // Fuzz spreads cards learned on the same day so they don't all fall due together.
            fsrs: FSRS::new(Parameters {
                enable_short_term: true,
                enable_fuzz: true,
                ..Parameters::default()
            }),
        }
    }
}

impl Scheduler {
    /// The state after grading `card` at `now`.
    pub fn apply(&self, card: &rs_fsrs::Card, now: DateTime<Utc>, grade: Grade) -> rs_fsrs::Card {
        self.fsrs.next(card.clone(), now, grade.rating()).card
    }

    /// How long until the card comes back for each of the four grades.
    pub fn preview(&self, card: &rs_fsrs::Card, now: DateTime<Utc>) -> [Duration; 4] {
        let log = self.fsrs.repeat(card.clone(), now);
        Grade::ALL.map(|grade| log[&grade.rating()].card.due - now)
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

    #[test]
    fn harder_grades_never_come_back_later() {
        let scheduler = Scheduler::default();
        let now = Utc::now();
        let mut card = new_schedule(now);
        for _ in 0..4 {
            let gaps = scheduler.preview(&card, now);
            assert!(gaps.windows(2).all(|pair| pair[0] <= pair[1]), "{gaps:?}");
            card = scheduler.apply(&card, now, Grade::Comfortable);
        }
    }

    #[test]
    fn dont_know_comes_back_within_the_session() {
        let scheduler = Scheduler::default();
        let now = Utc::now();
        let card = scheduler.apply(&new_schedule(now), now, Grade::DontKnow);
        assert!(card.due - now <= Duration::minutes(20));
    }

    #[test]
    fn repeated_success_stretches_the_gap() {
        let scheduler = Scheduler::default();
        let mut now = Utc::now();
        let mut card = new_schedule(now);
        let mut last_gap = Duration::zero();
        for _ in 0..5 {
            card = scheduler.apply(&card, now, Grade::Comfortable);
            let gap = card.due - now;
            assert!(gap >= last_gap);
            last_gap = gap;
            now = card.due;
        }
        assert!(last_gap > Duration::days(7));
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
