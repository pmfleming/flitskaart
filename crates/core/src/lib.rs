//! Everything the app does except drawing the window: cards, scheduling,
//! storage, the Dutch voice, practice conversations and turning conversations
//! into cards.

pub mod claude;
pub mod conversation;
pub mod drafting;
pub mod inputs;
pub mod model;
pub mod paths;
pub mod schedule;
pub mod speech;
pub mod store;
pub mod voice;
pub mod writing;

pub use model::{Card, CardId, Grade, Status};
pub use store::Store;
