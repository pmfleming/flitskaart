//! The study panel's own icons, drawn for this app on a 24-pixel grid.
//! They are line drawings in `currentColor`, so they follow the theme.

use gpui_kit::component::Icon;

/// Test mijn uitspraak: a speech bubble with your voice inside it.
pub fn speak() -> Icon {
    Icon::default().data(include_bytes!("../assets/icons/speak.svg"))
}

/// Hoor de zin: an ear with sound arriving.
pub fn listen() -> Icon {
    Icon::default().data(include_bytes!("../assets/icons/listen.svg"))
}

/// Betekenis: a word card with its explanation card.
pub fn meaning() -> Icon {
    Icon::default().data(include_bytes!("../assets/icons/meaning.svg"))
}
