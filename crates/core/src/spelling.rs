//! Dutch spelling: which words in a text the dictionary does not know, and
//! what was probably meant.
//!
//! The dictionary is OpenTaal's Hunspell dictionary, built into the app and
//! read on first use (a fraction of a second), so call [`warm_up`] early on a
//! background thread. Once it is read, checking a sentence takes microseconds;
//! suggestions take tens of milliseconds a word, so they are remembered.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Mutex, OnceLock};

use spellbook::Dictionary;

const AFF: &str = include_str!("../dictionaries/nl_NL.aff");
const DIC: &str = include_str!("../dictionaries/nl_NL.dic");

/// At most this many suggestions per word.
const MAX_SUGGESTIONS: usize = 3;

static DICTIONARY: OnceLock<Dictionary> = OnceLock::new();

fn dictionary() -> &'static Dictionary {
    DICTIONARY.get_or_init(|| Dictionary::new(AFF, DIC).expect("the built-in Dutch dictionary parses"))
}

fn suggestion_cache() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Reads the dictionary now, so the first check does not wait for it.
pub fn warm_up() {
    dictionary();
}

/// Whether the dictionary has been read, so checking will not wait for it.
pub fn is_ready() -> bool {
    DICTIONARY.get().is_some()
}

/// Whether the dictionary knows `word`, in the case it is written.
pub fn is_known(word: &str) -> bool {
    dictionary().check(word)
}

/// The byte ranges of the words in `text` the dictionary does not know, in order.
pub fn unknown_words(text: &str) -> Vec<Range<usize>> {
    words(text).filter(|range| !is_known_word(&text[range.clone()])).collect()
}

/// What `word` probably should have been, best first; may be empty.
pub fn suggestions(word: &str) -> Vec<String> {
    if let Some(found) = known_suggestions(word) {
        return found;
    }
    let mut found = Vec::new();
    dictionary().suggest(word, &mut found);
    // Splitting off a single letter ("auto s", "à msterdam") is never what was meant.
    found.retain(|s| s.split([' ', '-']).all(|part| part.chars().count() > 1));
    found.truncate(MAX_SUGGESTIONS);
    suggestion_cache().lock().unwrap_or_else(|e| e.into_inner()).insert(word.to_string(), found.clone());
    found
}

/// The suggestions for `word` if they were worked out before, without waiting.
pub fn known_suggestions(word: &str) -> Option<Vec<String>> {
    suggestion_cache().lock().unwrap_or_else(|e| e.into_inner()).get(word).cloned()
}

/// A word is known as written, or once the punctuation around it is let go:
/// "huis." at the end of a sentence, "'s" and "o.a." with theirs.
fn is_known_word(word: &str) -> bool {
    if is_known(word) {
        return true;
    }
    let bare = word.trim_matches(is_joiner);
    !bare.is_empty() && bare != word && is_known(bare)
}

/// Characters that may sit inside a word: "zo'n", "e-mail", "o.a.".
fn is_joiner(c: char) -> bool {
    matches!(c, '\'' | '’' | '-' | '.')
}

/// The byte ranges of the words worth checking: runs of letters and joiners,
/// without a joiner on either end (except an apostrophe in front, as in
/// "'s" and "'t"), and not numbers, single letters or web addresses.
fn words(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        loop {
            let (start, _) = chars.find(|&(_, c)| c.is_alphanumeric() || c == '\'' || c == '’')?;
            let mut end = start + text[start..].chars().next()?.len_utf8();
            while let Some(&(i, c)) = chars.peek() {
                if !(c.is_alphanumeric() || is_joiner(c) || c == '/' || c == '@') {
                    break;
                }
                end = i + c.len_utf8();
                chars.next();
            }
            let token = &text[start..end];
            // Keep "huis." whole for the check but underline only "huis".
            let trimmed_end = start + token.trim_end_matches(is_joiner).len();
            let lead = token.len() - token.trim_start_matches(is_joiner).len();
            let elided = lead > 0 && token.starts_with(['\'', '’']);
            let core_start = if elided { start } else { start + lead };
            if core_start >= trimmed_end {
                continue;
            }
            let core = &text[core_start..trimmed_end];
            let letters = core.chars().filter(|c| c.is_alphabetic()).count();
            let skip = letters < 2
                || core.chars().any(|c| c.is_numeric() || c == '/' || c == '@')
                || core.starts_with("www.");
            if !skip {
                return Some(core_start..trimmed_end);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(text: &str) -> Vec<&str> {
        unknown_words(text).into_iter().map(|range| &text[range]).collect()
    }

    #[test]
    fn a_correct_sentence_has_nothing_to_flag() {
        assert!(flagged("Gisteren ben ik naar de winkel gegaan.").is_empty());
        assert!(flagged("Hoe gaat het met je? Prima, dank je wel!").is_empty());
    }

    #[test]
    fn misspelt_words_are_flagged_with_what_was_meant() {
        assert_eq!(flagged("Ik heb het gezeggd, ik doe de bootschappen."), ["gezeggd", "bootschappen"]);
        assert_eq!(known_suggestions("bootschappen"), None);
        assert_eq!(suggestions("gezeggd").first().map(String::as_str), Some("gezegd"));
        assert_eq!(suggestions("bootschappen").first().map(String::as_str), Some("boodschappen"));
        assert_eq!(known_suggestions("bootschappen"), Some(suggestions("bootschappen")));
    }

    #[test]
    fn apostrophes_hyphens_and_accents_belong_to_the_word() {
        assert!(flagged("Zo'n mooie auto's, 's ochtends één café en een e-mail.").is_empty());
        assert_eq!(flagged("Twee autos."), ["autos"]);
    }

    #[test]
    fn names_keep_their_capital() {
        assert!(flagged("Ik woon in Amsterdam, in Nederland.").is_empty());
        assert_eq!(flagged("ik woon in nederland"), ["nederland"]);
    }

    #[test]
    fn numbers_single_letters_and_addresses_are_left_alone() {
        assert!(flagged("Om 8 uur, 12,50 euro, a b c, www.nos.nl en piet@voorbeeld.nl").is_empty());
    }

    #[test]
    fn ranges_count_bytes_after_accented_letters() {
        assert_eq!(flagged("één fiets en een fietz"), ["fietz"]);
    }

    #[test]
    fn silly_splits_are_not_suggested() {
        assert!(suggestions("autos").iter().all(|s| s != "auto s"));
        assert!(suggestions("amsterdam").iter().all(|s| !s.starts_with("à ")));
    }
}
