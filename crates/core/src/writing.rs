//! Writing the card: its Dutch becomes blanks, and the learner types it letter
//! by letter. Spaces and punctuation are given; only letters and digits are
//! typed. A wrong key marks the letter, which stays marked once found; after
//! [`TRIES`] wrong keys the letter is shown and the next one waits.

use crate::model::Grade;
use crate::speech::grade_for;

/// Wrong keys a letter takes before it is shown.
pub const TRIES: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// A space or punctuation mark, shown from the start.
    Given,
    /// A letter still to type.
    Blank,
    /// A letter still to type, after a wrong key.
    Missed,
    /// Typed right at the first try.
    Right,
    /// Found after a wrong key, or shown after too many.
    Wrong,
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    Right,
    Wrong,
    /// Not a letter, or nothing left to type.
    Ignored,
}

/// How a finished writing went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Written {
    /// Share of the letters typed right at the first try, 0.0–1.0.
    pub accuracy: f32,
    pub grade: Grade,
}

#[derive(Clone, Debug)]
pub struct Writing {
    cells: Vec<(char, Slot)>,
    /// The letter waiting to be typed; `None` once all are done.
    cursor: Option<usize>,
    tries: u8,
}

impl Writing {
    pub fn new(text: &str) -> Writing {
        let cells: Vec<(char, Slot)> =
            text.trim().chars().map(|c| (c, if c.is_alphanumeric() { Slot::Blank } else { Slot::Given })).collect();
        let cursor = cells.iter().position(|(_, slot)| *slot == Slot::Blank);
        Writing { cells, cursor, tries: 0 }
    }

    /// Every character of the card with how far it has come.
    pub fn cells(&self) -> &[(char, Slot)] {
        &self.cells
    }

    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    pub fn is_done(&self) -> bool {
        self.cursor.is_none()
    }

    /// Checks one typed character against the waiting letter, ignoring case
    /// and accents (e for é).
    pub fn type_char(&mut self, typed: char) -> Typed {
        let Some(at) = self.cursor.filter(|_| typed.is_alphanumeric()) else { return Typed::Ignored };
        let (want, slot) = &mut self.cells[at];
        if fold(typed) == fold(*want) {
            *slot = if *slot == Slot::Missed { Slot::Wrong } else { Slot::Right };
            self.advance(at);
            return Typed::Right;
        }
        *slot = Slot::Missed;
        self.tries += 1;
        if self.tries >= TRIES {
            *slot = Slot::Wrong;
            self.advance(at);
        }
        Typed::Wrong
    }

    fn advance(&mut self, from: usize) {
        self.tries = 0;
        self.cursor = self.cells[from + 1..].iter().position(|(_, slot)| *slot == Slot::Blank).map(|i| from + 1 + i);
    }

    /// The result, once every letter is done.
    pub fn result(&self) -> Option<Written> {
        if !self.is_done() {
            return None;
        }
        let letters = self.cells.iter().filter(|(_, slot)| *slot != Slot::Given).count();
        let right = self.cells.iter().filter(|(_, slot)| *slot == Slot::Right).count();
        let accuracy = if letters == 0 { 1.0 } else { right as f32 / letters as f32 };
        Some(Written { accuracy, grade: grade_for(accuracy) })
    }
}

/// A letter in lower case without its accent.
fn fold(c: char) -> char {
    match c.to_lowercase().next().unwrap_or(c) {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ç' => 'c',
        'ñ' => 'n',
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use super::{Grade, Slot, TRIES, Typed, Writing};

    fn type_all(writing: &mut Writing, keys: &str) -> Vec<Typed> {
        keys.chars().map(|c| writing.type_char(c)).collect()
    }

    #[test]
    fn spaces_and_punctuation_are_given() {
        let writing = Writing::new(" Ja, één! ");
        let slots: Vec<Slot> = writing.cells().iter().map(|(_, slot)| *slot).collect();
        assert_eq!(slots, [Slot::Blank, Slot::Blank, Slot::Given, Slot::Given, Slot::Blank, Slot::Blank, Slot::Blank, Slot::Given]);
        assert_eq!(writing.cursor(), Some(0));
    }

    #[test]
    fn right_letters_ignoring_case_and_accents() {
        let mut writing = Writing::new("Ja, één!");
        assert_eq!(type_all(&mut writing, "j a,een"), [Typed::Right, Typed::Ignored, Typed::Right, Typed::Ignored, Typed::Right, Typed::Right, Typed::Right]);
        let result = writing.result().unwrap();
        assert_eq!((result.accuracy, result.grade), (1.0, Grade::Easy));
    }

    #[test]
    fn a_wrong_key_marks_the_letter_for_good() {
        let mut writing = Writing::new("ik");
        assert_eq!(type_all(&mut writing, "x"), [Typed::Wrong]);
        assert_eq!((writing.cells()[0].1, writing.cursor()), (Slot::Missed, Some(0)), "it waits for the right letter");
        type_all(&mut writing, "ik");
        assert_eq!(writing.cells().iter().map(|(_, slot)| *slot).collect::<Vec<_>>(), [Slot::Wrong, Slot::Right]);
        assert_eq!(writing.result().unwrap().accuracy, 0.5);
    }

    #[test]
    fn too_many_wrong_keys_show_the_letter() {
        let mut writing = Writing::new("ik");
        for _ in 0..TRIES {
            writing.type_char('q');
        }
        assert_eq!((writing.cells()[0].1, writing.cursor()), (Slot::Wrong, Some(1)));
        assert!(writing.result().is_none(), "not done yet");
        writing.type_char('k');
        assert_eq!(writing.result().unwrap().grade, Grade::DontKnow);
        assert_eq!(writing.type_char('k'), Typed::Ignored);
    }
}
