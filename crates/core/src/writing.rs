//! Writing the card: its Dutch becomes blanks, and the learner types it letter
//! by letter. Spaces and punctuation are given; only letters and digits are
//! typed. Every key fills the next blank, right or wrong, so the learner may go
//! back (Backspace) to fix a wrong letter, or carry on and finish to see the
//! sentence corrected. The grade counts the letters right at the first try.

use crate::model::Grade;
use crate::speech::grade_for;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// A space or punctuation mark, shown from the start.
    Given,
    /// A letter still to type.
    Blank,
    /// Typed right at the first try.
    Right,
    /// Typed right after being wrong.
    Fixed,
    /// Typed wrong: the key that was typed.
    Wrong(char),
    /// Left wrong or blank when the writing finished: shown corrected.
    Shown,
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    Right,
    Wrong,
    /// Not a letter, or no blank left.
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
    /// Each character, how far it has come, whether it was ever typed wrong,
    /// and how many times it was typed.
    cells: Vec<(char, Slot, bool, u32)>,
    /// The blank the next key fills; `None` when all are filled or finished.
    cursor: Option<usize>,
    finished: bool,
}

impl Writing {
    pub fn new(text: &str) -> Writing {
        let cells: Vec<(char, Slot, bool, u32)> =
            text.trim().chars().map(|c| (c, if c.is_alphanumeric() { Slot::Blank } else { Slot::Given }, false, 0)).collect();
        let mut writing = Writing { cells, cursor: None, finished: false };
        writing.cursor = writing.next_letter(0);
        writing.finished = writing.cursor.is_none();
        writing
    }

    /// Every character of the card with how far it has come.
    pub fn cells(&self) -> impl Iterator<Item = (char, Slot)> + '_ {
        self.cells.iter().map(|&(letter, slot, _, _)| (letter, slot))
    }

    /// How many times the character at `index` was typed, so a retyped letter can be told apart.
    pub fn typed_count(&self, index: usize) -> u32 {
        self.cells.get(index).map_or(0, |cell| cell.3)
    }

    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    pub fn is_done(&self) -> bool {
        self.finished
    }

    /// Whether every blank is filled but some letters are wrong, so the
    /// learner can go back and fix them or finish.
    pub fn is_full(&self) -> bool {
        !self.finished && self.cursor.is_none()
    }

    /// Fills the next blank with a typed character, compared ignoring case
    /// and accents (e for é). Writing every letter right finishes it.
    pub fn type_char(&mut self, typed: char) -> Typed {
        let Some(at) = self.cursor.filter(|_| typed.is_alphanumeric() && !self.finished) else { return Typed::Ignored };
        let (want, slot, missed, count) = &mut self.cells[at];
        *count += 1;
        let outcome = if fold(typed) == fold(*want) {
            *slot = if *missed { Slot::Fixed } else { Slot::Right };
            Typed::Right
        } else {
            (*slot, *missed) = (Slot::Wrong(typed), true);
            Typed::Wrong
        };
        self.cursor = self.next_letter(at + 1);
        if self.cursor.is_none() && !self.cells.iter().any(|(_, slot, _, _)| matches!(slot, Slot::Wrong(_))) {
            self.finished = true;
        }
        outcome
    }

    /// Backspace: empties the letter before the next blank, to type it again.
    pub fn back(&mut self) -> bool {
        let end = self.cursor.unwrap_or(self.cells.len());
        let previous = self.cells[..end].iter().rposition(|(_, slot, _, _)| *slot != Slot::Given);
        match previous.filter(|_| !self.finished) {
            Some(at) => {
                self.cells[at].1 = Slot::Blank;
                self.cursor = Some(at);
                true
            }
            None => false,
        }
    }

    /// Ends the writing: wrong and empty letters are shown corrected.
    pub fn finish(&mut self) {
        for (_, slot, missed, _) in &mut self.cells {
            if matches!(slot, Slot::Blank | Slot::Wrong(_)) {
                (*slot, *missed) = (Slot::Shown, true);
            }
        }
        (self.cursor, self.finished) = (None, true);
    }

    fn next_letter(&self, from: usize) -> Option<usize> {
        self.cells.iter().skip(from).position(|(_, slot, _, _)| *slot == Slot::Blank).map(|i| from + i)
    }

    /// The result, once finished.
    pub fn result(&self) -> Option<Written> {
        if !self.finished {
            return None;
        }
        let letters = self.cells.iter().filter(|(_, slot, _, _)| *slot != Slot::Given).count();
        let right = self.cells.iter().filter(|(_, slot, _, _)| *slot == Slot::Right).count();
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
    use super::{Grade, Slot, Typed, Writing};

    fn type_all(writing: &mut Writing, keys: &str) -> Vec<Typed> {
        keys.chars().map(|c| writing.type_char(c)).collect()
    }

    fn slots(writing: &Writing) -> Vec<Slot> {
        writing.cells().map(|(_, slot)| slot).collect()
    }

    #[test]
    fn spaces_and_punctuation_are_given() {
        let writing = Writing::new(" Ja, één! ");
        assert_eq!(slots(&writing), [Slot::Blank, Slot::Blank, Slot::Given, Slot::Given, Slot::Blank, Slot::Blank, Slot::Blank, Slot::Given]);
        assert_eq!(writing.cursor(), Some(0));
        assert!(Writing::new("?!").is_done(), "nothing to write");
    }

    #[test]
    fn all_right_ignoring_case_and_accents_finishes_by_itself() {
        let mut writing = Writing::new("Ja, één!");
        assert_eq!(type_all(&mut writing, "j a,een"), [Typed::Right, Typed::Ignored, Typed::Right, Typed::Ignored, Typed::Right, Typed::Right, Typed::Right]);
        let result = writing.result().unwrap();
        assert_eq!((result.accuracy, result.grade), (1.0, Grade::Easy));
    }

    #[test]
    fn a_wrong_letter_fills_its_blank_and_the_next_waits() {
        let mut writing = Writing::new("ik ben");
        assert_eq!(type_all(&mut writing, "ixbem"), [Typed::Right, Typed::Wrong, Typed::Right, Typed::Right, Typed::Wrong]);
        assert_eq!(slots(&writing), [Slot::Right, Slot::Wrong('x'), Slot::Given, Slot::Right, Slot::Right, Slot::Wrong('m')]);
        assert!(writing.is_full() && !writing.is_done(), "full, waiting to be fixed or finished");
        assert_eq!(writing.type_char('n'), Typed::Ignored);
    }

    #[test]
    fn going_back_fixes_a_letter_but_the_grade_remembers() {
        let mut writing = Writing::new("ik");
        type_all(&mut writing, "x");
        assert!(writing.back());
        assert_eq!((slots(&writing)[0], writing.cursor()), (Slot::Blank, Some(0)));
        type_all(&mut writing, "ik");
        assert_eq!(slots(&writing), [Slot::Fixed, Slot::Right]);
        assert!(writing.is_done());
        assert_eq!(writing.result().unwrap().accuracy, 0.5);
        assert!(!writing.back(), "a finished writing stays finished");
    }

    #[test]
    fn finishing_shows_the_wrong_and_missing_letters_corrected() {
        let mut writing = Writing::new("ik ben");
        type_all(&mut writing, "ix");
        writing.finish();
        assert_eq!(slots(&writing), [Slot::Right, Slot::Shown, Slot::Given, Slot::Shown, Slot::Shown, Slot::Shown]);
        assert_eq!(writing.result().unwrap().grade, Grade::DontKnow);
    }
}
