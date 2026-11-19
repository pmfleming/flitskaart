//! Everything the app does except drawing the window: cards, scheduling,
//! storage, the Dutch voice and turning conversation snippets into cards.

pub mod drafting;
pub mod model;
pub mod paths;
pub mod schedule;
pub mod speech;
pub mod store;
pub mod voice;

pub use model::{Card, CardId, Grade, Status};
pub use store::Store;
