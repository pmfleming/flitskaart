//! The deck, stored in one local Turso database file.

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Duration, Local, SecondsFormat, TimeZone, Utc};
use turso::{Connection, Value};

use crate::model::{Card, CardId, Grade, NewCard, Status, normalize};
use crate::paths::Paths;
use crate::schedule::{Scheduler, new_schedule};
use crate::voice::{Gender, VOICES};

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

const CARD_COLUMNS: &str = "id, dutch, english, note, tags, status, snippet_id, created_at, due, schedule, reviews, misses, peeks";

/// How a review was done, kept with each review for later analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Speak,
    Silent,
    Listen,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Speak => "speak",
            Mode::Silent => "silent",
            Mode::Listen => "listen",
        }
    }

    pub fn parse(value: &str) -> Mode {
        match value {
            "silent" => Mode::Silent,
            "listen" => Mode::Listen,
            _ => Mode::Speak,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub new_per_day: u32,
    pub gender: Gender,
    pub male_voice: String,
    pub female_voice: String,
    pub mode: Mode,
    pub appearance: Appearance,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            new_per_day: 10,
            gender: Gender::Male,
            male_voice: VOICES.iter().find(|v| v.gender == Gender::Male).unwrap().id.into(),
            female_voice: VOICES.iter().find(|v| v.gender == Gender::Female).unwrap().id.into(),
            mode: Mode::Speak,
            appearance: Appearance::System,
        }
    }
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

    pub fn open_file(path: &str) -> Result<Store> {
        pollster::block_on(async {
            let db = turso::Builder::new_local(path).build().await.with_context(|| format!("opening {path}"))?;
            let conn = db.connect()?;
            conn.execute_batch(SCHEMA).await.context("creating tables")?;
            Ok(Store { _db: db, conn })
        })
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

    /// The next card to study: due reviews first, then new cards up to the daily
    /// limit, then cards from earlier in the session that come back within 20 minutes.
    pub fn next_card(&self, now: DateTime<Utc>, new_per_day: u32) -> Result<Option<Card>> {
        let due = self.cards_where(
            "status = 'active' AND reviews > 0 AND due <= ?1 ORDER BY due, id LIMIT 1",
            vec![stamp(now).into()],
        )?;
        if let Some(card) = due.into_iter().next() {
            return Ok(Some(card));
        }
        if self.new_allowed(now, new_per_day)? > 0 {
            let new = self.cards_where("status = 'active' AND reviews = 0 ORDER BY activated_at, id LIMIT 1", vec![])?;
            if let Some(card) = new.into_iter().next() {
                return Ok(Some(card));
            }
        }
        let soon = self.cards_where(
            "status = 'active' AND reviews > 0 AND due <= ?1 ORDER BY due, id LIMIT 1",
            vec![stamp(now + Duration::minutes(20)).into()],
        )?;
        Ok(soon.into_iter().next())
    }

    /// Saves a review and reschedules the card.
    pub fn record_review(
        &self,
        id: CardId,
        grade: Grade,
        mode: Mode,
        peeked: bool,
        scheduler: &Scheduler,
        now: DateTime<Utc>,
    ) -> Result<Card> {
        let card = self.card(id)?.ok_or_else(|| anyhow!("card {id} no longer exists"))?;
        let schedule = scheduler.apply(&card.schedule, now, grade);
        self.exec(
            "UPDATE cards SET schedule = ?2, due = ?3, reviews = reviews + 1,
                 misses = misses + ?4, peeks = peeks + ?5, first_review_at = COALESCE(first_review_at, ?6)
             WHERE id = ?1",
            vec![
                Value::Integer(id),
                serde_json::to_string(&schedule)?.into(),
                stamp(schedule.due).into(),
                Value::Integer(i64::from(grade == Grade::DontKnow)),
                Value::Integer(i64::from(peeked)),
                stamp(now).into(),
            ],
        )?;
        self.exec(
            "INSERT INTO reviews (card_id, grade, mode, peeked, reviewed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            vec![
                Value::Integer(id),
                Value::Integer(i64::from(grade.number())),
                mode.as_str().into(),
                Value::Integer(i64::from(peeked)),
                stamp(now).into(),
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
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let pairs = [
            ("new_per_day", settings.new_per_day.to_string()),
            ("gender", settings.gender.as_str().to_string()),
            ("male_voice", settings.male_voice.clone()),
            ("female_voice", settings.female_voice.clone()),
            ("mode", settings.mode.as_str().to_string()),
            ("appearance", settings.appearance.as_str().to_string()),
        ];
        for (key, value) in pairs {
            self.exec(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                vec![key.into(), value.into()],
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
    let schedule_json = text(&row[9]);
    Ok(Card {
        id: integer(&row[0]),
        dutch: text(&row[1]),
        english: text(&row[2]),
        note: text(&row[3]),
        tags: text(&row[4]),
        status: Status::parse(&text(&row[5])),
        snippet_id: match row[6] {
            Value::Integer(id) => Some(id),
            _ => None,
        },
        created_at: parse_stamp(&row[7]).unwrap_or_else(Utc::now),
        due: parse_stamp(&row[8]),
        schedule: serde_json::from_str(&schedule_json).context("reading a card's schedule")?,
        reviews: integer(&row[10]),
        misses: integer(&row[11]),
        peeks: integer(&row[12]),
    })
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
    let local = now.with_timezone(&Local).date_naive().and_hms_opt(0, 0, 0).unwrap();
    Local.from_local_datetime(&local).earliest().map_or(now, |t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&Paths::at(dir.path())).unwrap();
        (dir, store)
    }

    fn new_card(dutch: &str) -> NewCard {
        NewCard { dutch: dutch.into(), english: "english".into(), note: "".into(), tag: "".into() }
    }

    #[test]
    fn starts_empty() {
        let (_dir, store) = store();
        assert!(store.deck("").unwrap().is_empty());
        assert!(store.drafts().unwrap().is_empty());
        assert_eq!(store.next_card(Utc::now(), 10).unwrap().map(|c| c.id), None);
    }

    #[test]
    fn drafts_are_not_studied_until_accepted() {
        let (_dir, store) = store();
        let now = Utc::now();
        let snippet = store.add_snippet("Ik: hallo", "", now).unwrap();
        let ids = store.add_drafts(Some(snippet), &[new_card("Hallo daar."), new_card("  ")], now).unwrap();
        assert_eq!(ids.len(), 1, "blank drafts are skipped");
        assert_eq!(store.snippet(snippet).unwrap().as_deref(), Some("Ik: hallo"));
        assert!(store.next_card(now, 10).unwrap().is_none());
        store.accept(ids[0], now).unwrap();
        assert_eq!(store.next_card(now, 10).unwrap().map(|c| c.id), Some(ids[0]));
        assert!(store.drafts().unwrap().is_empty());
    }

    #[test]
    fn grading_moves_a_card_into_the_future_and_counts_misses_and_peeks() {
        let (_dir, store) = store();
        let now = Utc::now();
        let id = store.add_drafts(None, &[new_card("Ik heb er zin in.")], now).unwrap()[0];
        store.accept(id, now).unwrap();
        let scheduler = Scheduler::default();
        let card = store.record_review(id, Grade::DontKnow, Mode::Speak, true, &scheduler, now).unwrap();
        assert_eq!((card.reviews, card.misses, card.peeks), (1, 1, 1));
        assert!(card.due.unwrap() > now);
        // Comes back later in the session, not now.
        assert_eq!(store.counts(now, 10).unwrap().due, 0);
        assert_eq!(store.next_card(now, 10).unwrap().map(|c| c.id), Some(id), "learn-ahead within 20 minutes");
        let card = store.record_review(id, Grade::Easy, Mode::Silent, false, &scheduler, card.due.unwrap()).unwrap();
        assert!(card.due.unwrap() - now > Duration::hours(12));
        assert!(store.next_card(now + Duration::hours(1), 10).unwrap().is_none());
    }

    #[test]
    fn new_cards_respect_the_daily_limit() {
        let (_dir, store) = store();
        let now = Utc::now();
        let scheduler = Scheduler::default();
        let ids = store.add_drafts(None, &[new_card("een"), new_card("twee"), new_card("drie")], now).unwrap();
        for id in &ids {
            store.accept(*id, now).unwrap();
        }
        assert_eq!(store.counts(now, 2).unwrap().new, 2);
        store.record_review(ids[0], Grade::Easy, Mode::Speak, false, &scheduler, now).unwrap();
        store.record_review(ids[1], Grade::Easy, Mode::Speak, false, &scheduler, now).unwrap();
        assert_eq!(store.counts(now, 2).unwrap().new, 0);
        assert!(store.next_card(now, 2).unwrap().is_none());
    }

    #[test]
    fn finds_duplicates_and_edits_text() {
        let (_dir, store) = store();
        let now = Utc::now();
        let first = store.add_drafts(None, &[new_card("Mag ik de rekening, alstublieft?")], now).unwrap()[0];
        store.accept(first, now).unwrap();
        let second = store.add_drafts(None, &[new_card("mag ik de rekening alstublieft")], now).unwrap()[0];
        let draft = store.card(second).unwrap().unwrap();
        assert_eq!(store.duplicate_of(&draft).unwrap(), Some(first));
        store.update_text(second, "Mag ik betalen?", "Can I pay?", "", "restaurant").unwrap();
        let draft = store.card(second).unwrap().unwrap();
        assert_eq!(store.duplicate_of(&draft).unwrap(), None);
        assert!(store.update_text(second, "  ", "", "", "").is_err());
    }

    #[test]
    fn suspend_reset_delete_and_search() {
        let (_dir, store) = store();
        let now = Utc::now();
        let scheduler = Scheduler::default();
        let id = store.add_drafts(None, &[new_card("Het huis is groot.")], now).unwrap()[0];
        store.accept(id, now).unwrap();
        store.record_review(id, Grade::Hard, Mode::Speak, false, &scheduler, now).unwrap();
        store.set_suspended(id, true).unwrap();
        assert!(store.next_card(now + Duration::days(30), 10).unwrap().is_none());
        store.set_suspended(id, false).unwrap();
        store.reset(id, now).unwrap();
        assert!(store.card(id).unwrap().unwrap().is_new());
        assert_eq!(store.deck("HUIS").unwrap().len(), 1);
        assert!(store.deck("auto").unwrap().is_empty());
        store.delete(id).unwrap();
        assert!(store.card(id).unwrap().is_none());
    }

    #[test]
    fn a_retired_voice_loads_as_the_current_female_voice() {
        let (_dir, store) = store();
        let saved = Settings { gender: Gender::Female, female_voice: "mls-25".into(), ..Settings::default() };
        store.save_settings(&saved).unwrap();
        let loaded = store.settings().unwrap();
        assert_eq!(loaded.voice_id(), "nathalie");
        assert_eq!(loaded.gender, Gender::Female);
    }

    #[test]
    fn settings_persist() {
        let (dir, store) = store();
        let mut settings = store.settings().unwrap();
        assert_eq!(settings, Settings::default());
        settings.gender = Gender::Female;
        settings.new_per_day = 4;
        settings.mode = Mode::Listen;
        settings.appearance = Appearance::Dark;
        store.save_settings(&settings).unwrap();
        store.save_settings(&settings).unwrap();
        drop(store);
        let reopened = Store::open(&Paths::at(dir.path())).unwrap();
        assert_eq!(reopened.settings().unwrap(), settings);
    }
}
