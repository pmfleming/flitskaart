//! The app's own icons, drawn for it on a 24-pixel grid. They are line
//! drawings in `currentColor`, so they follow the theme. The speech bubble of
//! `speak` is shared by the conversation's bubble icons, so they read as a set.

use gpui_kit::component::Icon;

macro_rules! icons {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        pub fn $name() -> Icon {
            Icon::default().data(include_bytes!(concat!("../assets/icons/", stringify!($name), ".svg")))
        }
    )*};
}

icons! {
    /// Test mijn uitspraak, Spreek: a speech bubble with your voice inside it.
    speak,
    /// Hoor de zin: an ear with sound arriving.
    listen,
    /// Betekenis: a word card with its explanation card.
    meaning,
    /// Langzaam: a snail.
    slow,
    /// Verstuur: the speech bubble with an arrow carrying your answer off.
    send,
    /// Stop a recording: the speech bubble with a stop square inside.
    stop,
    /// Said in English: the speech bubble with EN inside.
    english,
    /// Not understood: the speech bubble with a question mark.
    unclear,
    /// Afronden: a new card laid on the stack.
    cards,
    /// An explanation: a lit bulb.
    tip,
    /// Opnieuw versturen: an arrow coming round again.
    retry,
    /// Het weekend: the sun coming up over the horizon.
    weekend,
    /// Boodschappen doen: a market basket.
    shopping,
    /// Op het werk: a briefcase.
    work,
    /// In een restaurant: a fork and a knife.
    restaurant,
    /// Bij de dokter: a stethoscope.
    doctor,
    /// Op vakantie: a palm tree.
    holiday,
}
