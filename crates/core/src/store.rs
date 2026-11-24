//! The deck, stored in one local Turso database file.

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Local, NaiveTime, SecondsFormat, TimeZone, Utc};
use turso::{Connection, Value};

use crate::model::{Card, CardId, Grade, NewCard, Status, normalize};
use crate::paths::Paths;
use crate::schedule::{Scheduler, count, new_schedule};
use crate::speech::Assessment;
use crate::voice::{Gender, VOICES, voices_for};
use crate::writing::Written;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS snippets (
    id INTEGER PRIMARY KEY,
    text TEXT NOT NULL,
    steer TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS cards (
    id INTEGER PRIMARY KEY,
    dutch TEXT NOT NULL,
    dutch_key TEXT NOT NULL,
    english TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    tags TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    snippet_id INTEGER,
    created_at TEXT NOT NULL,
    activated_at TEXT,
    first_review_at TEXT,
    due TEXT,
    schedule TEXT NOT NULL,
    reviews INTEGER NOT NULL DEFAULT 0,
    misses INTEGER NOT NULL DEFAULT 0,
    peeks INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS cards_by_status_due ON cards (status, due);
CREATE INDEX IF NOT EXISTS cards_by_key ON cards (dutch_key);
CREATE TABLE IF NOT EXISTS reviews (
    id INTEGER PRIMARY KEY,
    card_id INTEGER NOT NULL,
    grade INTEGER NOT NULL,
    mode TEXT NOT NULL,
    peeked INTEGER NOT NULL,
    reviewed_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Columns added after the first release, added to older deck files on open.
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    // The learner's own grade; `grade` holds the one that scheduled the card.
    ("reviews", "own_grade", "INTEGER"),
    // How understandable the learner was when reading aloud, if recorded.
    ("reviews", "speech_grade", "INTEGER"),
    ("reviews", "speech_accuracy", "REAL"),
    ("reviews", "heard", "TEXT"),
    // The latest understandability, for the deck list.
    ("cards", "speech_accuracy", "REAL"),
    // How much of the stability gain the review earned (see schedule::count).
    ("reviews", "share", "REAL"),
    // How well the learner wrote the card, if they did.
    ("reviews", "written_grade", "INTEGER"),
    ("reviews", "written_accuracy", "REAL"),
];

const CARD_COLUMNS: &str =
    "id, dutch, english, note, tags, status, snippet_id, due, schedule, reviews, misses, peeks, speech_accuracy";

/// How a review was done, kept with each review for later analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The card's text is shown; the learner may test their reading aloud.
    Read,
    /// The card is heard first with its text hidden.
    Listen,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Read => "read",
            Mode::Listen => "listen",
        }
    }

    /// Reads a saved mode. The earlier "speak" and "silent" modes are both reading now.
    pub fn parse(value: &str) -> Mode {
        match value {
            "listen" => Mode::Listen,
            _ => Mode::Read,
        }
    }
}

/// Light or dark window colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    /// Follow the desktop's light or dark setting.
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::System => "systeem",
            Appearance::Light => "licht",
            Appearance::Dark => "donker",
        }
    }

    pub fn parse(value: &str) -> Option<Appearance> {
        Appearance::ALL.into_iter().find(|a| a.as_str() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "Systeem",
            Appearance::Light => "Licht",
            Appearance::Dark => "Donker",
        }
    }
}

/// The input boost steps offered in Instellingen.
pub const INPUT_BOOSTS: [f32; 5] = [0.5, 1.0, 1.5, 2.0, 2.5];

/// The allowed input boost nearest to `value`.
pub fn snap_input_boost(value: f32) -> f32 {
    let distance = |step: f32| (step - value).abs();
    INPUT_BOOSTS.into_iter().fold(INPUT_BOOSTS[0], |best, step| if distance(step) < distance(best) { step } else { best })
}

/// An input boost as Dutch readers write it: `0,5×`, `1×`, `2,5×`.
pub fn boost_label(boost: f32) -> String {
    format!("{boost}×").replace('.', ",")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub new_per_day: u32,
    pub gender: Gender,
    pub male_voice: String,
    pub female_voice: String,
    pub mode: Mode,
    pub appearance: Appearance,
    /// How much recordings are amplified before they are judged; one of [`INPUT_BOOSTS`].
    pub input_boost: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            new_per_day: 10,
            gender: Gender::Male,
            male_voice: first_voice(Gender::Male),
            female_voice: first_voice(Gender::Female),
            mode: Mode::Read,
            appearance: Appearance::System,
            input_boost: 1.0,
        }
    }
}

/// The first voice on offer for `gender`.
fn first_voice(gender: Gender) -> String {
    voices_for(gender).next().map(|voice| voice.id.to_string()).unwrap_or_default()
}

impl Settings {
    /// The voice id for the chosen gender.
    pub fn voice_id(&self) -> &str {
        match self.gender {
            Gender::Male => &self.male_voice,
            Gender::Female => &self.female_voice,
        }
    }
}

/// What the learner did with a card before scoring it.
#[derive(Clone, Copy, Debug)]
pub struct Review<'a> {
    /// How well the learner says they knew the card.
    pub own: Grade,
    /// How understandable they were, when they read it aloud.
    pub speech: Option<&'a Assessment>,
    /// How well they wrote it, when they did.
    pub written: Option<Written>,
    pub mode: Mode,
    /// Whether they opened the meaning.
    pub peeked: bool,
}

/// What is waiting in today's session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Cards studied before and due now.
    pub due: usize,
    /// Never-studied cards still allowed today.
    pub new: usize,
}

pub struct Store {
    _db: turso::Database,
    conn: Connection,
}

impl Store {
    pub fn open(paths: &Paths) -> Result<Store> {
        paths.ensure().with_context(|| format!("creating {}", paths.root().display()))?;
        Store::open_file(&paths.database().to_string_lossy())
    }

    fn open_file(path: &str) -> Result<Store> {
        pollster::block_on(async {
            let db = turso::Builder::new_local(path).build().await.with_context(|| format!("opening {path}"))?;
            let conn = db.connect()?;
            conn.execute_batch(SCHEMA).await.context("creating tables")?;
            let store = Store { _db: db, conn };
            store.add_missing_columns()?;
            Ok(store)
        })
    }

    fn add_missing_columns(&self) -> Result<()> {
        for (table, column, kind) in ADDED_COLUMNS {
            let existing = self.rows(&format!("PRAGMA table_info({table})"), vec![])?;
            if !existing.iter().any(|row| text(&row[1]) == *column) {
                self.exec(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"), vec![])?;
            }
        }
        Ok(())
    }

    // ---- snippets and drafts ----

    pub fn add_snippet(&self, text: &str, steer: &str, now: DateTime<Utc>) -> Result<i64> {
        self.exec(
            "INSERT INTO snippets (text, steer, created_at) VALUES (?1, ?2, ?3)",
            vec![text.into(), steer.into(), stamp(now).into()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn snippet(&self, id: i64) -> Result<Option<String>> {
        let rows = self.rows("SELECT text FROM snippets WHERE id = ?1", vec![Value::Integer(id)])?;
        Ok(rows.into_iter().next().map(|row| text(&row[0])))
    }

    /// Stores drafted cards for review. Returns their ids.
    pub fn add_drafts(&self, snippet_id: Option<i64>, cards: &[NewCard], now: DateTime<Utc>) -> Result<Vec<CardId>> {
        let schedule = serde_json::to_string(&new_schedule(now))?;
        let mut ids = Vec::new();
        for card in cards {
            let dutch = card.dutch.trim();
            if dutch.is_empty() {
                continue;
            }
            self.exec(
                "INSERT INTO cards (dutch, dutch_key, english, note, tags, status, snippet_id, created_at, schedule)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'draft', ?6, ?7, ?8)",
                vec![
                    dutch.into(),
                    normalize(dutch).into(),
                    card.english.trim().into(),
                    card.note.trim().into(),
                    card.tag.trim().into(),
                    snippet_id.map_or(Value::Null, Value::Integer),
                    stamp(now).into(),
                    schedule.clone().into(),
                ],
            )?;
            ids.push(self.conn.last_insert_rowid());
        }
        Ok(ids)
    }

    pub fn drafts(&self) -> Result<Vec<Card>> {
        self.cards_where("status = 'draft' ORDER BY id", vec![])
    }

    /// Makes a draft part of the deck, due straight away.
    pub fn accept(&self, id: CardId, now: DateTime<Utc>) -> Result<()> {
        self.exec(
            "UPDATE cards SET status = 'active', activated_at = ?2, due = ?2, schedule = ?3 WHERE id = ?1 AND status = 'draft'",
            vec![Value::Integer(id), stamp(now).into(), serde_json::to_string(&new_schedule(now))?.into()],
        )
    }

    // ---- editing ----

    pub fn card(&self, id: CardId) -> Result<Option<Card>> {
        Ok(self.cards_where("id = ?1", vec![Value::Integer(id)])?.into_iter().next())
    }

    pub fn update_text(&self, id: CardId, dutch: &str, english: &str, note: &str, tags: &str) -> Result<()> {
        let dutch = dutch.trim();
        if dutch.is_empty() {
            return Err(anyhow!("De Nederlandse tekst mag niet leeg zijn."));
        }
        self.exec(
            "UPDATE cards SET dutch = ?2, dutch_key = ?3, english = ?4, note = ?5, tags = ?6 WHERE id = ?1",
            vec![
                Value::Integer(id),
                dutch.into(),
                normalize(dutch).into(),
                english.trim().into(),
                note.trim().into(),
                tags.trim().into(),
            ],
        )
    }

    pub fn delete(&self, id: CardId) -> Result<()> {
        self.exec("DELETE FROM reviews WHERE card_id = ?1", vec![Value::Integer(id)])?;
        self.exec("DELETE FROM cards WHERE id = ?1", vec![Value::Integer(id)])
    }

    pub fn set_suspended(&self, id: CardId, suspended: bool) -> Result<()> {
        let (from, to) = if suspended { ("active", "suspended") } else { ("suspended", "active") };
        self.exec(
            "UPDATE cards SET status = ?2 WHERE id = ?1 AND status = ?3",
            vec![Value::Integer(id), to.into(), from.into()],
        )
    }

    /// Forgets a card's progress so it is treated as new again.
    pub fn reset(&self, id: CardId, now: DateTime<Utc>) -> Result<()> {
        self.exec(
            "UPDATE cards SET schedule = ?2, due = ?3, reviews = 0, misses = 0, peeks = 0, first_review_at = NULL
             WHERE id = ?1 AND status != 'draft'",
            vec![Value::Integer(id), serde_json::to_string(&new_schedule(now))?.into(), stamp(now).into()],
        )
    }

    /// Another card (in the deck or among the drafts) with the same Dutch.
    pub fn duplicate_of(&self, card: &Card) -> Result<Option<CardId>> {
        let rows = self.rows(
            "SELECT id FROM cards WHERE dutch_key = ?1 AND id != ?2 AND (status != 'draft' OR id < ?2) ORDER BY id LIMIT 1",
            vec![normalize(&card.dutch).into(), Value::Integer(card.id)],
        )?;
        Ok(rows.into_iter().next().map(|row| integer(&row[0])))
    }

    // ---- the deck list ----

    /// Every card in the deck, newest first, optionally filtered by text or tag.
    pub fn deck(&self, search: &str) -> Result<Vec<Card>> {
        let search = search.trim().to_lowercase();
        let mut cards = self.cards_where("status != 'draft' ORDER BY id DESC", vec![])?;
        if !search.is_empty() {
            cards.retain(|c| {
                c.dutch.to_lowercase().contains(&search)
                    || c.tags.to_lowercase().contains(&search)
                    || c.note.to_lowercase().contains(&search)
            });
        }
        Ok(cards)
    }

    // ---- studying ----

    pub fn counts(&self, now: DateTime<Utc>, new_per_day: u32) -> Result<Counts> {
        let due = self.rows(
            "SELECT COUNT(*) FROM cards WHERE status = 'active' AND reviews > 0 AND due <= ?1",
            vec![stamp(now).into()],
        )?;
        let new = self.rows("SELECT COUNT(*) FROM cards WHERE status = 'active' AND reviews = 0", vec![])?;
        let allowed = self.new_allowed(now, new_per_day)?;
        Ok(Counts {
            due: integer(&due[0][0]) as usize,
            new: (integer(&new[0][0]) as usize).min(allowed),
        })
    }

    /// The next card to study: due reviews first (longest due first), then new
    /// cards up to the daily limit, picked at random so cards from one
    /// conversation are spread out. Nothing is shown before it is due, so once
    /// every card has had its turn, only cards scheduled for later are left.
    /// The `skipped` cards are left out.
    pub fn next_card(&self, now: DateTime<Utc>, new_per_day: u32, skipped: &[CardId]) -> Result<Option<Card>> {
        let ids: Vec<String> = skipped.iter().map(ToString::to_string).collect();
        let not_skipped = if ids.is_empty() { String::new() } else { format!(" AND id NOT IN ({})", ids.join(", ")) };
        let due = self.cards_where(
            &format!("status = 'active' AND reviews > 0 AND due <= ?1{not_skipped} ORDER BY due, id LIMIT 1"),
            vec![stamp(now).into()],
        )?;
        if let Some(card) = due.into_iter().next() {
            return Ok(Some(card));
        }
        if self.new_allowed(now, new_per_day)? > 0 {
            let new = self.cards_where(&format!("status = 'active' AND reviews = 0{not_skipped} ORDER BY random() LIMIT 1"), vec![])?;
            return Ok(new.into_iter().next());
        }
        Ok(None)
    }

    /// How many active cards have never been studied, whatever the daily limit.
    pub fn unstudied(&self) -> Result<usize> {
        let rows = self.rows("SELECT COUNT(*) FROM cards WHERE status = 'active' AND reviews = 0", vec![])?;
        Ok(integer(&rows[0][0]) as usize)
    }

    /// Saves a review and reschedules the card. [`count`] decides how the review counts.
    pub fn record_review(&self, id: CardId, review: Review, scheduler: &Scheduler, now: DateTime<Utc>) -> Result<Card> {
        let Review { own, speech, written, mode, peeked } = review;
        let card = self.card(id)?.ok_or_else(|| anyhow!("card {id} no longer exists"))?;
        // Reading aloud and writing are both tests; the weaker one counts.
        let tested = [speech.map(|s| s.grade), written.map(|w| w.grade)].into_iter().flatten().min();
        let counted = count(own, tested, peeked, mode);
        let schedule = scheduler.apply(&card.schedule, now, counted);
        let accuracy = || speech.map_or(Value::Null, |s| Value::Real(f64::from(s.accuracy)));
        self.exec(
            "UPDATE cards SET schedule = ?2, due = ?3, reviews = reviews + 1,
                 misses = misses + ?4, peeks = peeks + ?5, first_review_at = COALESCE(first_review_at, ?6),
                 speech_accuracy = COALESCE(?7, speech_accuracy)
             WHERE id = ?1",
            vec![
                Value::Integer(id),
                serde_json::to_string(&schedule)?.into(),
                stamp(schedule.due).into(),
                Value::Integer(i64::from(own == Grade::DontKnow)),
                Value::Integer(i64::from(peeked)),
                stamp(now).into(),
                accuracy(),
            ],
        )?;
        self.exec(
            "INSERT INTO reviews (card_id, grade, own_grade, speech_grade, speech_accuracy, heard, mode, peeked, reviewed_at, share, written_grade, written_accuracy)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            vec![
                Value::Integer(id),
                Value::Integer(i64::from(counted.grade.number())),
                Value::Integer(i64::from(own.number())),
                speech.map_or(Value::Null, |s| Value::Integer(i64::from(s.grade.number()))),
                accuracy(),
                speech.map_or(Value::Null, |s| s.heard.as_str().into()),
                mode.as_str().into(),
                Value::Integer(i64::from(peeked)),
                stamp(now).into(),
                Value::Real(counted.share),
                written.map_or(Value::Null, |w| Value::Integer(i64::from(w.grade.number()))),
                written.map_or(Value::Null, |w| Value::Real(f64::from(w.accuracy))),
            ],
        )?;
        self.card(id)?.ok_or_else(|| anyhow!("card {id} disappeared"))
    }

    /// The earliest moment a deck card falls due, for the "come back" message.
    pub fn next_due_after(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
        let rows = self.rows(
            "SELECT MIN(due) FROM cards WHERE status = 'active' AND due > ?1",
            vec![stamp(now).into()],
        )?;
        Ok(rows.into_iter().next().and_then(|row| parse_stamp(&row[0])))
    }

    fn new_allowed(&self, now: DateTime<Utc>, new_per_day: u32) -> Result<usize> {
        let started = self.rows(
            "SELECT COUNT(*) FROM cards WHERE first_review_at >= ?1",
            vec![stamp(start_of_local_day(now)).into()],
        )?;
        Ok((new_per_day as usize).saturating_sub(integer(&started[0][0]) as usize))
    }

    // ---- settings ----

    pub fn settings(&self) -> Result<Settings> {
        let mut settings = Settings::default();
        for row in self.rows("SELECT key, value FROM settings", vec![])? {
            let value = text(&row[1]);
            match text(&row[0]).as_str() {
                "new_per_day" => settings.new_per_day = value.parse().unwrap_or(settings.new_per_day),
                "gender" => settings.gender = Gender::parse(&value),
                "male_voice" if VOICES.iter().any(|v| v.id == value) => settings.male_voice = value,
                "female_voice" if VOICES.iter().any(|v| v.id == value) => settings.female_voice = value,
                "mode" => settings.mode = Mode::parse(&value),
                "appearance" => settings.appearance = Appearance::parse(&value).unwrap_or(settings.appearance),
                "input_boost" => settings.input_boost = value.parse().map(snap_input_boost).unwrap_or(settings.input_boost),
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let pairs: [(&str, Value); 7] = [
            ("new_per_day", settings.new_per_day.to_string().into()),
            ("gender", settings.gender.as_str().into()),
            ("male_voice", settings.male_voice.as_str().into()),
            ("female_voice", settings.female_voice.as_str().into()),
            ("mode", settings.mode.as_str().into()),
            ("appearance", settings.appearance.as_str().into()),
            ("input_boost", settings.input_boost.to_string().into()),
        ];
        for (key, value) in pairs {
            self.exec(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                vec![key.into(), value],
            )?;
        }
        Ok(())
    }

    // ---- plumbing ----

    fn cards_where(&self, clause: &str, params: Vec<Value>) -> Result<Vec<Card>> {
        let sql = format!("SELECT {CARD_COLUMNS} FROM cards WHERE {clause}");
        self.rows(&sql, params)?.iter().map(|row| card_from_row(row)).collect()
    }

    fn exec(&self, sql: &str, params: Vec<Value>) -> Result<()> {
        pollster::block_on(self.conn.execute(sql, params)).with_context(|| format!("running {sql}"))?;
        Ok(())
    }

    fn rows(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>> {
        pollster::block_on(async {
            let mut rows = self.conn.query(sql, params).await.with_context(|| format!("running {sql}"))?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push((0..row.column_count()).map(|i| row.get_value(i)).collect::<turso::Result<Vec<_>>>()?);
            }
            Ok(out)
        })
    }
}

fn card_from_row(row: &[Value]) -> Result<Card> {
    Ok(Card {
        id: integer(&row[0]),
        dutch: text(&row[1]),
        english: text(&row[2]),
        note: text(&row[3]),
        tags: text(&row[4]),
        status: Status::parse(&text(&row[5])),
        snippet_id: row[6].as_integer().copied(),
        due: parse_stamp(&row[7]),
        schedule: serde_json::from_str(&text(&row[8])).context("reading a card's schedule")?,
        reviews: integer(&row[9]),
        misses: integer(&row[10]),
        peeks: integer(&row[11]),
        speech_accuracy: real(&row[12]),
    })
}

fn real(value: &Value) -> Option<f32> {
    match value {
        Value::Real(n) => Some(*n as f32),
        Value::Integer(n) => Some(*n as f32),
        _ => None,
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::Text(s) => s.clone(),
        Value::Integer(n) => n.to_string(),
        Value::Real(n) => n.to_string(),
        _ => String::new(),
    }
}

fn integer(value: &Value) -> i64 {
    match value {
        Value::Integer(n) => *n,
        Value::Real(n) => *n as i64,
        Value::Text(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// Fixed-width UTC timestamps, so text comparison in SQL orders them correctly.
fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_stamp(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Text(s) => DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
}

fn start_of_local_day(now: DateTime<Utc>) -> DateTime<Utc> {
    let local = now.with_timezone(&Local).date_naive().and_time(NaiveTime::MIN);
    Local.from_local_datetime(&local).earliest().map_or(now, |t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::{
        Appearance, CardId, DateTime, Gender, Grade, INPUT_BOOSTS, Mode, NewCard, Paths, Review, SCHEMA, Scheduler,
        Settings, Store, Utc, Value, boost_label, integer, snap_input_boost, text,
    };
    use crate::speech::{Assessment, assess};
    use crate::writing::Written;
    use chrono::Duration;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&Paths::at(dir.path())).unwrap();
        (dir, store)
    }

    fn drafts(store: &Store, phrases: &[&str], now: DateTime<Utc>) -> Vec<CardId> {
        let cards: Vec<NewCard> = phrases.iter().map(|dutch| NewCard { dutch: dutch.to_string(), ..NewCard::default() }).collect();
        store.add_drafts(None, &cards, now).unwrap()
    }

    /// Cards in the deck, due now.
    fn deck(store: &Store, phrases: &[&str], now: DateTime<Utc>) -> Vec<CardId> {
        let ids = drafts(store, phrases, now);
        ids.iter().for_each(|id| store.accept(*id, now).unwrap());
        ids
    }

    fn read(own: Grade, speech: Option<&Assessment>, peeked: bool) -> Review<'_> {
        Review { own, speech, written: None, mode: Mode::Read, peeked }
    }

    fn reviews(store: &Store, columns: &str) -> Vec<Vec<Value>> {
        store.rows(&format!("SELECT {columns} FROM reviews ORDER BY id"), vec![]).unwrap()
    }

    #[test]
    fn grading_moves_a_card_into_the_future_and_counts_misses_and_peeks() {
        let (_dir, store) = store();
        let (now, scheduler) = (Utc::now(), Scheduler::exact());
        let id = deck(&store, &["Ik heb er zin in."], now)[0];
        let card = store.record_review(id, read(Grade::DontKnow, None, true), &scheduler, now).unwrap();
        assert_eq!((card.reviews, card.misses, card.peeks), (1, 1, 1));
        assert!(card.due.unwrap() > now);
        assert_eq!(store.counts(now, 10).unwrap().due, 0, "comes back later in the session, not now");
        assert!(store.next_card(now, 10, &[]).unwrap().is_none(), "not before it is due, so the session can end");
        assert_eq!(store.next_due_after(now).unwrap(), card.due);
        assert_eq!(store.next_card(card.due.unwrap(), 10, &[]).unwrap().map(|c| c.id), Some(id), "back once due");
        let card = store.record_review(id, read(Grade::Easy, None, false), &scheduler, card.due.unwrap()).unwrap();
        assert!(card.due.unwrap() - now > Duration::hours(12));
        assert!(store.next_card(now + Duration::hours(1), 10, &[]).unwrap().is_none());
        assert!(store.next_due_after(now).unwrap().is_some());
    }

    #[test]
    fn new_cards_respect_the_daily_limit() {
        let (_dir, store) = store();
        let (now, scheduler) = (Utc::now(), Scheduler::exact());
        let ids = deck(&store, &["een", "twee", "drie"], now);
        assert_eq!(store.counts(now, 2).unwrap().new, 2);
        for id in &ids[..2] {
            store.record_review(*id, read(Grade::Easy, None, false), &scheduler, now).unwrap();
        }
        assert_eq!(store.counts(now, 2).unwrap().new, 0);
        assert!(store.next_card(now, 2, &[]).unwrap().is_none());
    }

    #[test]
    fn new_cards_come_shuffled_and_skipped_cards_are_left_out() {
        let (_dir, store) = store();
        let now = Utc::now();
        let ids = deck(&store, &["een", "twee", "drie", "vier", "vijf", "zes"], now);
        let next = |skipped: &[CardId]| store.next_card(now, 10, skipped).unwrap().map(|card| card.id).unwrap_or(0);
        let picks: std::collections::HashSet<CardId> = (0..40).map(|_| next(&[])).collect();
        assert!(picks.len() > 1, "not always the first one saved");
        assert!(picks.iter().all(|id| ids.contains(id)));
        let first = next(&[]);
        assert_ne!(next(&[first]), first);
        assert_eq!(next(&ids), 0, "all skipped");
        let card = store.card(ids[0]).unwrap().unwrap();
        assert_eq!((card.reviews, card.is_new(), store.unstudied().unwrap()), (0, true, 6), "skipping saves nothing");
    }

    #[test]
    fn suspension_reset_search_and_blank_edits() {
        let (_dir, store) = store();
        let now = Utc::now();
        let id = deck(&store, &["Mag ik de rekening, alstublieft?"], now)[0];
        assert!(store.update_text(id, "  ", "", "", "").is_err(), "the Dutch can't be blank");
        store.record_review(id, read(Grade::Hard, None, false), &Scheduler::exact(), now).unwrap();
        store.set_suspended(id, true).unwrap();
        assert!(store.next_card(now + Duration::days(30), 10, &[]).unwrap().is_none());
        store.reset(id, now).unwrap();
        assert!(store.card(id).unwrap().unwrap().is_new());
        assert_eq!(store.deck("REKENING").unwrap().len(), 1, "search ignores case");
    }

    #[test]
    fn each_review_saves_how_it_counted() {
        let (_dir, store) = store();
        let (now, scheduler) = (Utc::now(), Scheduler::exact());
        let ids = deck(&store, &["Ik heb kaas gekocht.", "Ik heb brood gekocht."], now);
        let clear = assess("Ik heb kaas gekocht.", "Ik heb kaas gekocht.");
        let unclear = assess("Ik heb brood gekocht.", "Ik heb gekocht.");
        let a = store.record_review(ids[0], read(Grade::Comfortable, Some(&clear), false), &scheduler, now).unwrap();
        let b = store.record_review(ids[1], read(Grade::Easy, Some(&unclear), false), &scheduler, now).unwrap();
        assert!(b.due.unwrap() < a.due.unwrap(), "unclear speech brings a known card back sooner");
        assert_eq!((a.speech_accuracy, b.misses), (Some(1.0), 0), "a miss means not knowing the card");
        let a = store.record_review(ids[0], read(Grade::Comfortable, None, false), &scheduler, a.due.unwrap()).unwrap();
        store.record_review(ids[0], read(Grade::Easy, None, true), &scheduler, a.due.unwrap()).unwrap();

        let saved = reviews(&store, "share, grade, own_grade, speech_grade, heard");
        let shares: Vec<_> = saved.iter().map(|row| row[0].clone()).collect();
        let share = Value::Real(crate::schedule::SCORE_ONLY_SHARE);
        assert_eq!(shares, [Value::Real(1.0), Value::Real(1.0), share, Value::Real(0.0)]);
        assert_eq!((integer(&saved[1][1]), integer(&saved[1][2])), (integer(&saved[1][3]), 4), "pronunciation replaced the score");
        assert_eq!(text(&saved[1][4]), "Ik heb gekocht.");
        assert_eq!((integer(&saved[3][1]), integer(&saved[3][2])), (2, 4), "Makkelijk with the meaning open counts as Moeilijk");
    }

    #[test]
    fn writing_counts_as_a_test_and_the_weaker_test_wins() {
        let (_dir, store) = store();
        let (now, scheduler) = (Utc::now(), Scheduler::exact());
        let ids = deck(&store, &["Ik heb kaas gekocht."], now);
        let clear = assess("Ik heb kaas gekocht.", "Ik heb kaas gekocht.");
        let written = Written { accuracy: 0.7, grade: Grade::Hard };
        let review = Review { written: Some(written), ..read(Grade::Easy, Some(&clear), false) };
        store.record_review(ids[0], review, &scheduler, now).unwrap();
        let saved = reviews(&store, "share, grade, speech_grade, written_grade, written_accuracy");
        assert_eq!(saved[0][0], Value::Real(1.0), "tested, so it counts in full");
        assert_eq!((integer(&saved[0][1]), integer(&saved[0][2]), integer(&saved[0][3])), (2, 4, 2), "the writing set the grade");
        assert!(matches!(saved[0][4], Value::Real(a) if (a - 0.7).abs() < 1e-6));
    }

    #[test]
    fn settings_persist_and_old_values_load_sensibly() {
        let (dir, store) = store();
        assert_eq!(store.settings().unwrap(), Settings::default());
        let settings = Settings {
            gender: Gender::Female,
            new_per_day: 4,
            mode: Mode::Listen,
            appearance: Appearance::Dark,
            input_boost: 1.5,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        store.save_settings(&settings).unwrap();
        drop(store);
        let store = Store::open(&Paths::at(dir.path())).unwrap();
        assert_eq!(store.settings().unwrap(), settings);

        // A retired voice, the old speaking and silent modes, and boosts between steps.
        store.save_settings(&Settings { female_voice: "mls-25".into(), ..settings }).unwrap();
        assert_eq!(store.settings().unwrap().voice_id(), "nathalie");
        for old in ["speak", "silent"] {
            store.exec("UPDATE settings SET value = ?1 WHERE key = 'mode'", vec![old.into()]).unwrap();
            assert_eq!(store.settings().unwrap().mode, Mode::Read, "{old}");
        }
        let snapped = [0.1, 1.4, 2.2, 9.0].map(snap_input_boost);
        assert_eq!(snapped, [0.5, 1.5, 2.0, 2.5]);
        assert_eq!(INPUT_BOOSTS.map(boost_label), ["0,5×", "1×", "1,5×", "2×", "2,5×"]);
    }

    #[test]
    fn opens_a_deck_made_before_speech_was_judged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cards.db").to_string_lossy().to_string();
        // SCHEMA is the original layout; the speech columns are added on open.
        pollster::block_on(async {
            let conn = turso::Builder::new_local(&path).build().await.unwrap().connect().unwrap();
            conn.execute_batch(SCHEMA).await.unwrap();
            conn.execute("INSERT INTO reviews (card_id, grade, mode, peeked, reviewed_at) VALUES (1, 3, 'speak', 0, 'x')", ())
                .await
                .unwrap();
        });
        let store = Store::open_file(&path).unwrap();
        let rows = reviews(&store, "grade, own_grade, speech_accuracy");
        assert_eq!((integer(&rows[0][0]), &rows[0][1]), (3, &Value::Null));
        drop(store);
        Store::open_file(&path).expect("opening again finds the columns already there");
    }
}
