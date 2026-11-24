//! Voortgang: what the learner has done and how well the deck is known,
//! worked out from the saved reviews, the cards and the conversations.
//!
//! A day is a local calendar day, the same boundary the daily limit for new
//! cards uses. A day counts as practised when a card was scored or a
//! conversation had a line from the learner; skipping a card saves nothing.

use anyhow::Result;
use chrono::{DateTime, Datelike as _, Duration, Local, NaiveDate, Utc};
use turso::Value;

use crate::model::{Card, CardId, Grade, Status};
use crate::store::{Mode, Store, integer, parse_stamp, stamp, start_of_local_day, text};

/// How many days the activity chart and the skills look back.
pub const WINDOW_DAYS: i64 = 30;
/// A card whose FSRS stability (the days until recall falls to 90%) is below
/// this is Wankel.
pub const SHAKY_BELOW_DAYS: f64 = 7.0;
/// From this stability on a card is Sterk; in between it is Groeit.
pub const STRONG_FROM_DAYS: f64 = 30.0;
/// Onthouden is shown once there are this many reviews to judge it by.
pub const RECALL_MIN_REVIEWS: usize = 10;
/// The longest time one card's review counts for; beyond it the learner was
/// most likely doing something else.
pub const MAX_REVIEW_SECONDS: f64 = 120.0;
/// The longest time one answer in Gesprek counts for.
pub const MAX_ANSWER_SECONDS: f64 = 180.0;

/// Days in a row with practice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Streak {
    /// The run ending today, or ending yesterday while today is still open.
    pub days: u32,
    /// The longest run so far.
    pub longest: u32,
    /// Whether today has been practised yet.
    pub today: bool,
}

/// Today's queue: what has had its turn and what is still waiting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Today {
    /// Cards scored today that aren't due again yet.
    pub done: usize,
    /// Cards due now and new cards still allowed today.
    pub waiting: usize,
    /// Reviews saved today, counting a card scored twice twice.
    pub reviews: usize,
}

/// The deck's cards by how well they are known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strength {
    pub new: usize,
    pub shaky: usize,
    pub growing: usize,
    pub strong: usize,
    /// Paused cards, left out of the four above.
    pub paused: usize,
}

impl Strength {
    pub fn total(&self) -> usize {
        self.new + self.shaky + self.growing + self.strong
    }
}

/// One way of practising over the last [`WINDOW_DAYS`] days.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skill {
    /// 0.0–1.0; what it measures depends on the skill.
    pub score: f32,
    /// How many times it was practised.
    pub count: usize,
    /// The change in percentage points against the [`WINDOW_DAYS`] days before.
    pub change: Option<f32>,
}

/// Gesprek over the last [`WINDOW_DAYS`] days.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Talking {
    pub talks: usize,
    /// The learner's lines.
    pub lines: usize,
    /// The share of those lines spoken rather than typed.
    pub spoken: f32,
}

/// One day of the activity chart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    /// Reviews of cards seen for the first time.
    pub new: usize,
    /// All other reviews.
    pub again: usize,
}

/// Everything Voortgang shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Whether anything has been practised, ever.
    pub started: bool,
    pub streak: Streak,
    /// The last seven days, oldest first, and whether each was practised.
    pub week: Vec<(NaiveDate, bool)>,
    pub today: Today,
    /// Active cards studied at least once.
    pub learned: usize,
    pub strength: Strength,
    /// The share of reviews a day or more after the last one that weren't
    /// Weet ik niet. `None` until there are enough of them.
    pub recall: Option<f32>,
    /// Reviews over the last [`WINDOW_DAYS`] days.
    pub reviews: usize,
    /// Time spent reviewing and answering in Gesprek over the same days.
    pub seconds: f64,
    /// Oldest first, ending today.
    pub activity: Vec<Day>,
    /// Understandability when reading aloud.
    pub speaking: Option<Skill>,
    /// Letters right first time when writing.
    pub writing: Option<Skill>,
    /// Cards known in Luisteren.
    pub listening: Option<Skill>,
    /// Reviews backed by speaking or writing.
    pub tested: Option<Skill>,
    pub talking: Option<Talking>,
}

/// A saved review, as Voortgang reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewRow {
    pub card: CardId,
    pub at: DateTime<Utc>,
    /// The learner's own score.
    pub own: Grade,
    pub mode: Mode,
    pub speech: Option<f32>,
    pub written: Option<f32>,
    pub seconds: f64,
}

/// A conversation in Gesprek, saved once the learner has said something.
#[derive(Clone, Debug, PartialEq)]
pub struct Talk {
    /// `None` until it is first saved.
    pub id: Option<i64>,
    pub topic: String,
    pub started_at: DateTime<Utc>,
    /// The learner's lines that were answered.
    pub lines: u32,
    pub spoken: u32,
    /// Time spent answering, each answer up to [`MAX_ANSWER_SECONDS`].
    pub seconds: f64,
}

impl Talk {
    pub fn new(topic: &str, now: DateTime<Utc>) -> Talk {
        Talk { id: None, topic: topic.trim().to_string(), started_at: now, lines: 0, spoken: 0, seconds: 0.0 }
    }

    /// Counts one answered line, spoken or typed, that took `seconds` to give.
    pub fn add_line(&mut self, spoken: bool, seconds: f64) {
        self.lines += 1;
        self.spoken += u32::from(spoken);
        self.seconds += seconds.clamp(0.0, MAX_ANSWER_SECONDS);
    }
}

fn day_of(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&Local).date_naive()
}

/// The streak on `today`, given every practised day in any order.
pub fn streak(days: &[NaiveDate], today: NaiveDate) -> Streak {
    let mut days: Vec<NaiveDate> = days.iter().copied().filter(|day| *day <= today).collect();
    days.sort_unstable();
    days.dedup();
    let mut longest = 0;
    let mut run = 0;
    let mut previous: Option<NaiveDate> = None;
    for day in &days {
        run = if previous.is_some_and(|p| *day - p == Duration::days(1)) { run + 1 } else { 1 };
        longest = longest.max(run);
        previous = Some(*day);
    }
    let practised_today = days.last() == Some(&today);
    let alive = practised_today || days.last() == Some(&(today - Duration::days(1)));
    Streak { days: if alive { run } else { 0 }, longest, today: practised_today }
}

/// Adds up the reviews (in the order they were saved), the deck's cards and
/// the conversations. `waiting` is what Oefenen still has for today.
pub fn summarize(reviews: &[ReviewRow], cards: &[Card], talks: &[Talk], waiting: usize, now: DateTime<Utc>) -> Progress {
    let today = day_of(now);
    let since = now - Duration::days(WINDOW_DAYS);
    let before = since - Duration::days(WINDOW_DAYS);
    let talks: Vec<&Talk> = talks.iter().filter(|talk| talk.lines > 0).collect();

    let practised: Vec<NaiveDate> =
        reviews.iter().map(|r| day_of(r.at)).chain(talks.iter().map(|t| day_of(t.started_at))).collect();
    let week = (0..7).rev().map(|back| today - Duration::days(back)).map(|day| (day, practised.contains(&day))).collect();

    // The first review of each card, and the time since its previous one.
    let mut last_seen: std::collections::HashMap<CardId, DateTime<Utc>> = std::collections::HashMap::new();
    let mut activity: Vec<Day> =
        (0..WINDOW_DAYS).rev().map(|back| Day { date: today - Duration::days(back), new: 0, again: 0 }).collect();
    let (mut recalled, mut judged) = (0, 0);
    for review in reviews {
        let previous = last_seen.insert(review.card, review.at);
        if review.at < since {
            continue;
        }
        if let Some(day) = activity.iter_mut().find(|d| d.date == day_of(review.at)) {
            if previous.is_none() { day.new += 1 } else { day.again += 1 }
        }
        // Learning steps minutes apart say nothing about remembering.
        if previous.is_some_and(|p| review.at - p >= Duration::days(1)) {
            judged += 1;
            recalled += usize::from(review.own != Grade::DontKnow);
        }
    }

    let reviewed_today: Vec<CardId> = reviews.iter().filter(|r| day_of(r.at) == today).map(|r| r.card).collect();
    let done = cards
        .iter()
        .filter(|card| card.status == Status::Active && reviewed_today.contains(&card.id) && card.due.is_some_and(|due| due > now))
        .count();

    let mut strength = Strength::default();
    for card in cards {
        let place = match card.status {
            Status::Draft => continue,
            Status::Suspended => &mut strength.paused,
            Status::Active if card.is_new() => &mut strength.new,
            Status::Active if card.schedule.stability < SHAKY_BELOW_DAYS => &mut strength.shaky,
            Status::Active if card.schedule.stability < STRONG_FROM_DAYS => &mut strength.growing,
            Status::Active => &mut strength.strong,
        };
        *place += 1;
    }

    let recent: Vec<&ReviewRow> = reviews.iter().filter(|r| r.at >= since).collect();
    let earlier: Vec<&ReviewRow> = reviews.iter().filter(|r| r.at >= before && r.at < since).collect();
    let skill = |measure: fn(&ReviewRow) -> Option<f32>| {
        let average = |rows: &[&ReviewRow]| {
            let values: Vec<f32> = rows.iter().filter_map(|r| measure(r)).collect();
            (!values.is_empty()).then(|| (values.iter().sum::<f32>() / values.len() as f32, values.len()))
        };
        let (score, count) = average(&recent)?;
        Some(Skill { score, count, change: average(&earlier).map(|(old, _)| (score - old) * 100.0) })
    };
    let recent_talks: Vec<&&Talk> = talks.iter().filter(|t| t.started_at >= since).collect();
    let lines: u32 = recent_talks.iter().map(|t| t.lines).sum();

    Progress {
        started: !practised.is_empty(),
        streak: streak(&practised, today),
        week,
        today: Today { done, waiting, reviews: reviewed_today.len() },
        learned: cards.iter().filter(|c| c.status == Status::Active && !c.is_new()).count(),
        strength,
        recall: (judged >= RECALL_MIN_REVIEWS).then(|| recalled as f32 / judged as f32),
        reviews: recent.len(),
        seconds: recent.iter().map(|r| r.seconds).sum::<f64>() + recent_talks.iter().map(|t| t.seconds).sum::<f64>(),
        activity,
        speaking: skill(|r| r.speech),
        writing: skill(|r| r.written),
        listening: skill(|r| (r.mode == Mode::Listen).then_some(if r.own == Grade::DontKnow { 0.0 } else { 1.0 })),
        tested: skill(|r| Some(if r.speech.is_some() || r.written.is_some() { 1.0 } else { 0.0 })),
        talking: (!recent_talks.is_empty()).then(|| Talking {
            talks: recent_talks.len(),
            lines: lines as usize,
            spoken: recent_talks.iter().map(|t| t.spoken).sum::<u32>() as f32 / lines.max(1) as f32,
        }),
    }
}

/// "ma", "di", … for a day's place in the week.
pub fn weekday_short(day: NaiveDate) -> &'static str {
    ["ma", "di", "wo", "do", "vr", "za", "zo"][day.weekday().num_days_from_monday() as usize]
}

impl Store {
    /// Everything Voortgang shows. `new_limit` is today's limit for new cards.
    pub fn progress(&self, now: DateTime<Utc>, new_limit: u32) -> Result<Progress> {
        let counts = self.counts(now, new_limit)?;
        let cards = self.cards_where("status != 'draft'", vec![])?;
        Ok(summarize(&self.review_rows()?, &cards, &self.talks()?, counts.due + counts.new, now))
    }

    /// The streak alone, for Oefenen.
    pub fn streak(&self, now: DateTime<Utc>) -> Result<Streak> {
        let reviews = self.rows("SELECT reviewed_at FROM reviews", vec![])?;
        let talks = self.rows("SELECT started_at FROM talks WHERE lines > 0", vec![])?;
        let days: Vec<NaiveDate> = reviews.iter().chain(&talks).filter_map(|row| parse_stamp(&row[0])).map(day_of).collect();
        Ok(streak(&days, day_of(now)))
    }

    /// How many reviews were saved today.
    pub fn reviewed_today(&self, now: DateTime<Utc>) -> Result<usize> {
        let rows = self.rows("SELECT COUNT(*) FROM reviews WHERE reviewed_at >= ?1", vec![stamp(start_of_local_day(now)).into()])?;
        Ok(integer(&rows[0][0]) as usize)
    }

    /// Saves a conversation, the first time with a new id.
    pub fn save_talk(&self, talk: &mut Talk) -> Result<()> {
        let values = vec![
            talk.topic.as_str().into(),
            stamp(talk.started_at).into(),
            Value::Integer(i64::from(talk.lines)),
            Value::Integer(i64::from(talk.spoken)),
            Value::Real(talk.seconds),
        ];
        match talk.id {
            Some(id) => self.exec(
                "UPDATE talks SET topic = ?1, started_at = ?2, lines = ?3, spoken = ?4, seconds = ?5 WHERE id = ?6",
                values.into_iter().chain([Value::Integer(id)]).collect(),
            ),
            None => {
                self.exec("INSERT INTO talks (topic, started_at, lines, spoken, seconds) VALUES (?1, ?2, ?3, ?4, ?5)", values)?;
                talk.id = Some(self.last_id());
                Ok(())
            }
        }
    }

    fn review_rows(&self) -> Result<Vec<ReviewRow>> {
        let rows = self.rows(
            "SELECT card_id, reviewed_at, COALESCE(own_grade, grade), mode, speech_accuracy, written_accuracy, COALESCE(seconds, 0)
             FROM reviews ORDER BY reviewed_at, id",
            vec![],
        )?;
        let real = |value: &Value| match value {
            Value::Real(n) => Some(*n),
            Value::Integer(n) => Some(*n as f64),
            _ => None,
        };
        Ok(rows
            .iter()
            .filter_map(|row| {
                Some(ReviewRow {
                    card: integer(&row[0]),
                    at: parse_stamp(&row[1])?,
                    own: Grade::from_number(u8::try_from(integer(&row[2])).ok()?)?,
                    mode: Mode::parse(&text(&row[3])),
                    speech: real(&row[4]).map(|n| n as f32),
                    written: real(&row[5]).map(|n| n as f32),
                    seconds: real(&row[6]).unwrap_or(0.0),
                })
            })
            .collect())
    }

    fn talks(&self) -> Result<Vec<Talk>> {
        let rows = self.rows("SELECT id, topic, started_at, lines, spoken, seconds FROM talks ORDER BY id", vec![])?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                Some(Talk {
                    id: Some(integer(&row[0])),
                    topic: text(&row[1]),
                    started_at: parse_stamp(&row[2])?,
                    lines: u32::try_from(integer(&row[3])).ok()?,
                    spoken: u32::try_from(integer(&row[4])).ok()?,
                    seconds: match row[5] {
                        Value::Real(n) => n,
                        Value::Integer(n) => n as f64,
                        _ => 0.0,
                    },
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone as _, Utc};

    use super::{Card, Grade, Mode, ReviewRow, STRONG_FROM_DAYS, Status, Streak, Talk, streak, summarize, weekday_short};
    use crate::schedule::new_schedule;

    fn review(card: i64, at: DateTime<Utc>, own: Grade) -> ReviewRow {
        ReviewRow { card, at, own, mode: Mode::Read, speech: None, written: None, seconds: 10.0 }
    }

    fn card(id: i64, status: Status, reviews: i64, stability: f64, due: DateTime<Utc>) -> Card {
        let mut schedule = new_schedule(due);
        schedule.stability = stability;
        Card {
            id,
            dutch: format!("kaart {id}"),
            english: String::new(),
            note: String::new(),
            tags: String::new(),
            status,
            snippet_id: None,
            due: Some(due),
            schedule,
            reviews,
            misses: 0,
            peeks: 0,
            speech_accuracy: None,
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// A moment on a local day, as the learner's clock shows it.
    fn local(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
        let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
        Local.from_local_datetime(&day.and_time(time)).earliest().unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_streak_counts_days_in_a_row_and_survives_until_midnight() {
        let today = date(2026, 11, 24);
        let back = |n: i64| today - Duration::days(n);
        // Four days, a missed day, then two days ending yesterday.
        let days = [back(8), back(7), back(6), back(5), back(2), back(1), back(1)];
        assert_eq!(streak(&days, today), Streak { days: 2, longest: 4, today: false }, "still alive before today's first card");
        let days: Vec<_> = days.iter().copied().chain([today]).collect();
        assert_eq!(streak(&days, today), Streak { days: 3, longest: 4, today: true });
        assert_eq!(streak(&[back(2)], today), Streak { days: 0, longest: 1, today: false }, "a missed day ends it");
        assert_eq!(streak(&[], today), Streak::default());
        assert_eq!(weekday_short(today), "di");
    }

    #[test]
    fn a_day_ends_at_local_midnight() {
        let today = Local::now().date_naive();
        let yesterday = today - Duration::days(1);
        let reviews = [review(1, local(yesterday, 23, 50), Grade::Easy), review(1, local(today, 0, 10), Grade::Easy)];
        let progress = summarize(&reviews, &[], &[], 0, local(today, 12, 0));
        assert_eq!(progress.streak, Streak { days: 2, longest: 2, today: true });
        assert_eq!(progress.today.reviews, 1);
        let last: Vec<_> = progress.activity.iter().rev().take(2).map(|day| (day.new, day.again)).collect();
        assert_eq!(last, [(0, 1), (1, 0)], "first ever review counts as new, on the day it was made");
        assert_eq!(progress.week.iter().filter(|(_, practised)| *practised).count(), 2);
    }

    #[test]
    fn recall_leaves_out_learning_steps_and_waits_for_enough_reviews() {
        let now = Utc::now();
        let mut reviews = Vec::new();
        for id in 0..12 {
            let first = now - Duration::days(20);
            reviews.push(review(id, first, Grade::Hard));
            // Minutes later: a learning step, so it isn't judged.
            reviews.push(review(id, first + Duration::minutes(10), Grade::DontKnow));
            reviews.push(review(id, first + Duration::days(3), if id < 3 { Grade::DontKnow } else { Grade::Comfortable }));
        }
        let progress = summarize(&reviews, &[], &[], 0, now);
        assert_eq!(progress.recall, Some(9.0 / 12.0));
        assert_eq!(progress.reviews, 36);
        assert_eq!(summarize(&reviews[..27], &[], &[], 0, now).recall, None, "nine judged reviews are too few");
    }

    #[test]
    fn cards_by_strength_and_todays_queue() {
        let now = Utc::now();
        let (later, earlier) = (now + Duration::days(2), now - Duration::minutes(1));
        let cards = [
            card(1, Status::Active, 0, 0.0, now),
            card(2, Status::Active, 2, 6.9, later),
            card(3, Status::Active, 3, 7.0, earlier),
            card(4, Status::Active, 9, STRONG_FROM_DAYS, later),
            card(5, Status::Suspended, 4, 40.0, later),
        ];
        let reviews = [review(2, now - Duration::minutes(5), Grade::Easy), review(3, now - Duration::minutes(4), Grade::Hard)];
        let progress = summarize(&reviews, &cards, &[], 4, now);
        let s = progress.strength;
        assert_eq!((s.new, s.shaky, s.growing, s.strong, s.paused, s.total()), (1, 1, 1, 1, 1, 4));
        assert_eq!(progress.learned, 3);
        assert_eq!((progress.today.done, progress.today.waiting), (1, 4), "card 3 is due again, so it is still waiting");
    }

    #[test]
    fn skills_compare_with_the_month_before() {
        let now = Utc::now();
        let spoke = |card, days: i64, accuracy| ReviewRow { speech: Some(accuracy), ..review(card, now - Duration::days(days), Grade::Easy) };
        let reviews = [
            spoke(1, 50, 0.70),
            spoke(1, 10, 0.80),
            spoke(2, 5, 0.90),
            ReviewRow { written: Some(1.0), ..review(3, now - Duration::days(2), Grade::Easy) },
            ReviewRow { mode: Mode::Listen, ..review(4, now - Duration::days(1), Grade::DontKnow) },
            ReviewRow { mode: Mode::Listen, ..review(5, now - Duration::days(1), Grade::Easy) },
        ];
        let progress = summarize(&reviews, &[], &[], 0, now);
        let speaking = progress.speaking.unwrap();
        assert!((speaking.score - 0.85).abs() < 1e-6 && speaking.count == 2);
        assert!((speaking.change.unwrap() - 15.0).abs() < 1e-3, "{speaking:?}");
        assert_eq!(progress.writing.map(|w| (w.count, w.change)), Some((1, None)));
        assert_eq!(progress.listening.map(|l| (l.score, l.count)), Some((0.5, 2)));
        assert_eq!(progress.tested.map(|t| t.count), Some(5), "the review 50 days ago is out of the window");
        assert!((progress.tested.unwrap().score - 0.6).abs() < 1e-6);
        assert_eq!(progress.talking, None);
    }

    #[test]
    fn a_conversation_counts_once_the_learner_says_something() {
        let now = Utc::now();
        let mut talk = Talk::new(" Het weekend ", now - Duration::days(1));
        let silent = summarize(&[], &[], std::slice::from_ref(&talk), 0, now);
        assert_eq!((silent.started, silent.streak.days), (false, 0), "opened but never answered");
        talk.add_line(true, 20.0);
        talk.add_line(false, 900.0);
        assert_eq!((talk.topic.as_str(), talk.lines, talk.spoken, talk.seconds), ("Het weekend", 2, 1, 200.0), "a long pause counts for 3 minutes");
        let progress = summarize(&[], &[], &[talk], 0, now);
        assert_eq!((progress.started, progress.streak.days, progress.seconds), (true, 1, 200.0));
        assert_eq!(progress.talking.map(|t| (t.talks, t.lines, t.spoken)), Some((1, 2, 0.5)));
    }

    #[test]
    fn the_store_reads_back_reviews_talks_and_deleted_cards() {
        use crate::model::NewCard;
        use crate::schedule::Scheduler;
        use crate::store::Review;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::Store::open(&crate::paths::Paths::at(dir.path())).unwrap();
        let now = Utc::now();
        let empty = store.progress(now, 10).unwrap();
        assert!(!empty.started && empty.streak == Streak::default() && empty.activity.len() == 30 && empty.week.len() == 7);
        let cards = [NewCard { dutch: "een".into(), ..NewCard::default() }, NewCard { dutch: "twee".into(), ..NewCard::default() }];
        let ids = store.add_drafts(None, &cards, now).unwrap();
        ids.iter().for_each(|id| store.accept(*id, now).unwrap());
        let review = Review { own: Grade::Easy, speech: None, written: None, mode: Mode::Read, peeked: false, seconds: Some(500.0) };
        store.record_review(ids[0], review, &Scheduler::default(), now).unwrap();
        let mut talk = Talk::new("Op het werk", now);
        talk.add_line(true, 30.0);
        store.save_talk(&mut talk).unwrap();
        talk.add_line(true, 30.0);
        store.save_talk(&mut talk).unwrap();

        let progress = store.progress(now, 10).unwrap();
        assert_eq!((progress.today.done, progress.today.waiting, progress.today.reviews), (1, 1, 1));
        assert_eq!((progress.streak.days, progress.learned, progress.seconds), (1, 1, 120.0 + 60.0), "a review counts for 2 minutes at most");
        assert_eq!(progress.talking.map(|t| (t.talks, t.lines)), Some((1, 2)), "saved twice, one conversation");
        assert_eq!((store.reviewed_today(now).unwrap(), store.streak(now).unwrap().days), (1, 1));
        // Deleting the card keeps the day it was practised.
        store.delete(ids[0]).unwrap();
        let progress = store.progress(now, 10).unwrap();
        assert_eq!((progress.streak.days, progress.reviews, progress.learned), (1, 1, 0));
    }
}
