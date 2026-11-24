//! The app's own icons, drawn for it on a 24-pixel grid. They are line
//! drawings in `currentColor`, so they follow the theme. The speech bubble of
//! `speak` is shared by the conversation's bubble icons, and the card of
//! `practice` by the card icons, so they read as a set.

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
    // The screens.
    /// Oefenen: a flash card with a bolt.
    practice,
    /// Gesprek: two speech bubbles, one answering the other.
    talk,
    /// Toevoegen: a clipboard with a plus.
    paste,
    /// Concepten: a card with a pencil still on it.
    drafts,
    /// Kaarten: a stack of cards seen from the side.
    deck,
    /// Instellingen: three sliders.
    sliders,

    // Practising.
    /// Test mijn uitspraak, Spreek: a speech bubble with your voice inside it.
    speak,
    /// Hoor de zin: an ear with sound arriving.
    listen,
    /// Betekenis, Vertaling: a word card with its explanation card.
    meaning,
    /// Langzaam: a snail.
    slow,
    /// Lezen: an open book.
    read,
    /// Schrijf: a pencil filling in the blanks.
    write,
    /// Overslaan: an arrow past a card to the next.
    skip,
    /// Toon tekst: an open eye.
    reveal,
    /// Mijn opname: a microphone with a play button.
    playback,
    /// De app hoorde: sound turning into lines of text.
    transcript,
    /// Te herhalen, Volgende keer: a clock.
    due,
    /// Nieuw: a sparkle.
    new,
    /// Klaar voor nu: a tick in a circle.
    done,
    /// Fout: a cross in a circle.
    miss,

    // Gesprek.
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
    /// Opnieuw: an arrow coming round again.
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

    // Cards and drafts.
    /// Nederlands: the letters NL.
    dutch,
    /// Onderwerp: a label.
    tag,
    /// Bron: the page the card came from.
    source,
    /// Aanwijzing: a signpost.
    steer,
    /// Mogelijk dubbel: two cards that say the same.
    duplicate,
    /// Zoeken: a magnifying glass.
    search,
    /// Bewaar, gedaan: a tick.
    check,
    /// Alles bewaren: two ticks.
    check_all,
    /// Bewerken: a pencil.
    edit,
    /// Pauzeren: two bars.
    pause,
    /// Hervatten: a play triangle.
    resume,
    /// Verwijderen: a bin.
    trash,
    /// Sluiten: a cross.
    close,

    // Instellingen.
    /// Stem: a voice as a sound wave.
    voice,
    /// Microfoon: a microphone on its stand.
    microphone,
    /// Gedempt: a microphone struck through.
    mic_off,
    /// A Bluetooth microphone: a headset.
    headset,
    /// The laptop's own microphone.
    laptop,
    /// A USB microphone: a plug.
    usb,
    /// Versterking: a loudspeaker growing louder.
    boost,
    /// Systeem: a screen that is half light, half dark.
    system,
    /// Licht: the sun.
    light,
    /// Donker: a crescent moon.
    dark,
    /// Nieuwe kaarten per dag: a calendar day with a plus.
    per_day,
    /// Gegevens: a folder.
    folder,
    /// Minder: a minus.
    minus,
    /// Meer: a plus.
    plus,
}
