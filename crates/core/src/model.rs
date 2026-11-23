use rs_fsrs::Rating;

pub type CardId = i64;

/// Where a card is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Made from a snippet and waiting for review. Never studied.
    Draft,
    /// In the deck and scheduled.
    Active,
    /// In the deck but left out of study.
    Suspended,
}

impl Status {
    pub fn parse(value: &str) -> Status {
        match value {
            "active" => Status::Active,
            "suspended" => Status::Suspended,
            _ => Status::Draft,
        }
    }
}

/// The four self-assessment buttons, from hardest to easiest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Grade {
    DontKnow = 1,
    Hard = 2,
    Comfortable = 3,
    Easy = 4,
}

impl Grade {
    pub const ALL: [Grade; 4] = [Grade::DontKnow, Grade::Hard, Grade::Comfortable, Grade::Easy];
    const LABELS: [&str; 4] = ["Weet ik niet", "Moeilijk", "Gaat goed", "Makkelijk"];
    const RATINGS: [Rating; 4] = [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy];

    pub fn from_number(n: u8) -> Option<Grade> {
        Grade::ALL.get(usize::from(n).checked_sub(1)?).copied()
    }

    pub fn number(self) -> u8 {
        self as u8
    }

    /// Its place in [`Grade::ALL`].
    pub fn index(self) -> usize {
        usize::from(self.number() - 1)
    }

    /// The label on the button.
    pub fn label(self) -> &'static str {
        Grade::LABELS[self.index()]
    }

    pub(crate) fn rating(self) -> Rating {
        Grade::RATINGS[self.index()]
    }
}

#[derive(Clone, Debug)]
pub struct Card {
    pub id: CardId,
    pub dutch: String,
    /// Hidden until the learner asks for it.
    pub english: String,
    /// A short explanation in simple Dutch. May be empty.
    pub note: String,
    pub tags: String,
    pub status: Status,
    pub snippet_id: Option<i64>,
    pub due: Option<chrono::DateTime<chrono::Utc>>,
    pub schedule: rs_fsrs::Card,
    pub reviews: i64,
    pub misses: i64,
    pub peeks: i64,
    /// How understandable the learner was the last time they read it aloud, 0.0–1.0.
    pub speech_accuracy: Option<f32>,
}

impl Card {
    pub fn is_new(&self) -> bool {
        self.reviews == 0
    }
}

/// A card as it comes out of drafting, before it is stored.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct NewCard {
    pub dutch: String,
    #[serde(default)]
    pub english: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub tag: String,
}

/// Lower-cases and strips punctuation so near-identical phrases compare equal.
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{Grade, normalize};

    #[test]
    fn normalize_ignores_case_punctuation_and_spacing() {
        assert_eq!(normalize("Mag ik de rekening,  alstublieft?"), normalize("mag ik de rekening alstublieft"));
        assert_ne!(normalize("Ik woon al drie jaar hier."), normalize("Ik woon drie jaar hier."));
    }

    #[test]
    fn grades_round_trip_through_numbers_in_order() {
        for grade in Grade::ALL {
            assert_eq!(Grade::from_number(grade.number()), Some(grade));
            assert_eq!(Grade::ALL[grade.index()], grade);
        }
        assert_eq!([Grade::from_number(0), Grade::from_number(5)], [None, None]);
        assert!(Grade::ALL.is_sorted());
    }
}
