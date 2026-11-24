//! What the language models have used: every answer's tokens, kept in the deck file and
//! added up for Instellingen.

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use turso::Value;

use crate::llm::Usage;
use crate::store::{Store, integer, parse_stamp, stamp, text};

/// Tokens added up over some answers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageTotal {
    /// How many answers.
    pub calls: u64,
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl UsageTotal {
    pub fn tokens(&self) -> u64 {
        self.input + self.cache_read + self.cache_write + self.output
    }

    fn add(&mut self, usage: &Usage) {
        self.input += usage.input;
        self.cache_read += usage.cache_read;
        self.cache_write += usage.cache_write;
        self.output += usage.output;
    }
}

/// Everything Claude used, from several sides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageSummary {
    pub total: UsageTotal,
    /// Per model (its full name), most tokens first.
    pub by_model: Vec<(String, UsageTotal)>,
    /// Per local day with any use, the latest first.
    pub by_day: Vec<(NaiveDate, UsageTotal)>,
    /// The tokens of each model on each of the last days, oldest first, for
    /// the chart. Models are in `by_model`'s order.
    pub daily: Vec<(NaiveDate, Vec<u64>)>,
}

/// Adds up `entries` (when, and what one model used for one answer). An
/// answer's entries share their moment, which counts it once.
pub fn summarize(entries: &[(DateTime<Utc>, Usage)], now: DateTime<Utc>, days: usize) -> UsageSummary {
    let mut summary = UsageSummary::default();
    let mut answers: Vec<DateTime<Utc>> = Vec::new();
    for (at, usage) in entries {
        let new_answer = !answers.contains(at);
        if new_answer {
            answers.push(*at);
        }
        let calls = u64::from(new_answer);
        summary.total.add(usage);
        summary.total.calls += calls;
        let model = entry(&mut summary.by_model, usage.model.clone());
        model.add(usage);
        model.calls += 1;
        let day = at.with_timezone(&Local).date_naive();
        let today = entry(&mut summary.by_day, day);
        today.add(usage);
        today.calls += calls;
    }
    summary.by_model.sort_by_key(|(_, total)| std::cmp::Reverse(total.tokens()));
    summary.by_day.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
    let last = now.with_timezone(&Local).date_naive();
    summary.daily = (0..days as i64)
        .rev()
        .map(|back| last - Duration::days(back))
        .map(|day| {
            let per_model = summary
                .by_model
                .iter()
                .map(|(model, _)| {
                    let on_day = entries.iter().filter(|(at, usage)| usage.model == *model && at.with_timezone(&Local).date_naive() == day);
                    on_day.map(|(_, usage)| usage.tokens()).sum()
                })
                .collect();
            (day, per_model)
        })
        .collect();
    summary
}

/// The total kept under `key`, started if there is none yet.
fn entry<K: PartialEq>(list: &mut Vec<(K, UsageTotal)>, key: K) -> &mut UsageTotal {
    let at = match list.iter().position(|(k, _)| *k == key) {
        Some(at) => at,
        None => {
            list.push((key, UsageTotal::default()));
            list.len() - 1
        }
    };
    &mut list[at].1
}

impl Store {
    /// Saves what one answer used; its models share the moment.
    pub fn add_usage(&self, usage: &[Usage], now: DateTime<Utc>) -> Result<()> {
        for used in usage {
            self.exec(
                "INSERT INTO llm_usage (at, cli, model, effort, input, cache_read, cache_write, output)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                vec![
                    stamp(now).into(),
                    used.cli.as_str().into(),
                    used.model.as_str().into(),
                    used.effort.as_str().into(),
                    Value::Integer(used.input as i64),
                    Value::Integer(used.cache_read as i64),
                    Value::Integer(used.cache_write as i64),
                    Value::Integer(used.output as i64),
                ],
            )?;
        }
        Ok(())
    }

    /// Everything saved so far, added up, with a chart of the last `days` days.
    pub fn usage_summary(&self, now: DateTime<Utc>, days: usize) -> Result<UsageSummary> {
        let rows = self.rows("SELECT at, cli, model, effort, input, cache_read, cache_write, output FROM llm_usage ORDER BY id", vec![])?;
        let count = |value: &Value| integer(value).max(0) as u64;
        let entries: Vec<(DateTime<Utc>, Usage)> = rows
            .iter()
            .filter_map(|row| {
                let usage = Usage {
                    cli: text(&row[1]),
                    model: text(&row[2]),
                    effort: text(&row[3]),
                    input: count(&row[4]),
                    cache_read: count(&row[5]),
                    cache_write: count(&row[6]),
                    output: count(&row[7]),
                };
                Some((parse_stamp(&row[0])?, usage))
            })
            .collect();
        Ok(summarize(&entries, now, days))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::summarize;
    use crate::llm::Usage;

    fn used(model: &str, input: u64, output: u64) -> Usage {
        Usage { cli: "claude".into(), model: model.into(), effort: "medium".into(), input, cache_read: 100, cache_write: 0, output }
    }

    #[test]
    fn adds_up_by_model_and_day() {
        let now = Utc::now();
        let yesterday = now - Duration::days(1);
        let entries = [
            (yesterday, used("claude-sonnet-5-5", 10, 5)),
            (now, used("claude-opus-5-5", 50, 20)),
            // The same answer also used a second model: still one answer.
            (now, used("claude-haiku-5-5", 1, 1)),
        ];
        let summary = summarize(&entries, now, 7);
        assert_eq!((summary.total.calls, summary.total.tokens()), (2, 87 + 300));
        let models: Vec<&str> = summary.by_model.iter().map(|(model, _)| model.as_str()).collect();
        assert_eq!(models, ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"], "most tokens first");
        assert_eq!(summary.by_day.len(), 2);
        assert_eq!((summary.by_day[0].1.calls, summary.by_day[0].1.tokens()), (1, 72 + 200), "today first");
        assert_eq!(summary.daily.len(), 7);
        assert_eq!(summary.daily[6].1, [170, 0, 102], "today, per model");
        assert_eq!(summary.daily[5].1, [0, 115, 0], "yesterday");
    }

    #[test]
    fn saved_usage_comes_back_added_up() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::Store::open(&crate::paths::Paths::at(dir.path())).unwrap();
        let now = Utc::now();
        assert_eq!(store.usage_summary(now, 30).unwrap().total.calls, 0);
        store.add_usage(&[used("claude-opus-5-5", 50, 20), used("claude-haiku-5-5", 1, 1)], now).unwrap();
        let summary = store.usage_summary(now, 30).unwrap();
        assert_eq!((summary.total.calls, summary.by_model.len(), summary.daily.len()), (1, 2, 30));
    }
}
