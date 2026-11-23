//! Instellingen: the voice, the microphone test, the colours and the daily
//! limit for new cards.

use dutch_flashcards_core::store::Appearance;
use dutch_flashcards_core::voice::{self, Gender, SAMPLE, Speed, VoiceOption};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app::FlashcardsApp;
use crate::ui::{hint, section};

/// The microphone test is the third section of Instellingen.
pub const MICROPHONE_SECTION: usize = 2;

impl FlashcardsApp {
    fn set_gender(&mut self, gender: Gender, cx: &mut Context<Self>) {
        self.settings.gender = gender;
        self.save_settings();
        self.say(SAMPLE.into(), cx);
    }

    fn choose_voice(&mut self, choice: &'static VoiceOption, cx: &mut Context<Self>) {
        let slot = match choice.gender {
            Gender::Male => &mut self.settings.male_voice,
            Gender::Female => &mut self.settings.female_voice,
        };
        *slot = choice.id.into();
        self.settings.gender = choice.gender;
        self.save_settings();
        cx.notify();
    }

    fn set_appearance(&mut self, appearance: Appearance, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.appearance = appearance;
        self.save_settings();
        self.apply_appearance(window, cx);
        cx.notify();
    }

    fn change_new_per_day(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.settings.new_per_day = self.settings.new_per_day.saturating_add_signed(delta).min(100);
        self.save_settings();
        self.after_deck_change(cx);
    }

    pub fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let recognizer = if self.listener.is_installed() {
            hint("Spraakherkenning is geïnstalleerd.", cx)
        } else {
            hint("Spraakherkenning ontbreekt. Installeer die met scripts/fetch-models.sh.", cx).text_color(cx.theme().danger)
        };
        v_flex()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .p_5()
            .gap_6()
            .child(self.render_voices(cx))
            .child(
                section(
                    "Uitspraak",
                    "Met Test mijn uitspraak lees je een kaart hardop voor. Whisper luistert op deze laptop en geeft je uitspraak een cijfer van 1 tot 4.",
                    cx,
                )
                .child(recognizer),
            )
            .child(self.render_mic_test(cx))
            .child(section("Weergave", "Systeem volgt de lichte of donkere stand van je bureaublad.", cx).child(
                h_flex().gap_1().children(Appearance::ALL.map(|appearance| {
                    Button::new(appearance.as_str())
                        .outline()
                        .label(appearance.label())
                        .selected(self.settings.appearance == appearance)
                        .on_click(cx.listener(move |this, _, window, cx| this.set_appearance(appearance, window, cx)))
                })),
            ))
            .child(section("Nieuwe kaarten per dag", "Hoeveel nieuwe kaarten je per dag leert. Herhalingen komen altijd.", cx).child(
                h_flex()
                    .gap_2()
                    .child(Button::new("fewer").outline().small().label("−").on_click(cx.listener(|this, _, _, cx| this.change_new_per_day(-1, cx))))
                    .child(div().w(px(40.)).text_center().text_lg().child(self.settings.new_per_day.to_string()))
                    .child(Button::new("more").outline().small().label("+").on_click(cx.listener(|this, _, _, cx| this.change_new_per_day(1, cx)))),
            ))
            .child(section("Gegevens", format!("Je kaarten en geluid staan in {}", self.paths.root().display()), cx))
    }

    /// Man or woman, then the voices of that gender to try and choose.
    fn render_voices(&self, cx: &mut Context<Self>) -> Div {
        let gender = self.settings.gender;
        let genders = h_flex().gap_1().children([Gender::Male, Gender::Female].map(|g| {
            Button::new(g.as_str()).outline().label(g.label()).selected(gender == g).on_click(cx.listener(move |this, _, _, cx| this.set_gender(g, cx)))
        }));
        let missing = voice::VOICES.iter().any(|v| !self.sound.speaker.is_installed(v));
        section("Stem", "Kies een mannen- of vrouwenstem. Pim, Ronnie en Alex spreken Nederlands uit Nederland, Nathalie spreekt Vlaams.", cx)
            .child(genders)
            .child(v_flex().gap_1().children(voice::voices_for(gender).map(|option| self.render_voice(option, cx)).collect::<Vec<_>>()))
            .when(missing, |d| d.child(hint("Ontbrekende stemmen installeer je met scripts/fetch-models.sh.", cx)))
    }

    fn render_voice(&self, option: &'static VoiceOption, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let installed = self.sound.speaker.is_installed(option);
        let chosen = option.id == self.settings.voice_id();
        let (note, note_color) = if installed { (option.note, theme.muted_foreground) } else { ("Niet geïnstalleerd", theme.danger) };
        let (border, background, pick) = if chosen { (theme.primary, theme.accent, "Gekozen") } else { (theme.border, theme.background, "Kies") };
        h_flex()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(6.))
            .border_1()
            .border_color(border)
            .bg(background)
            .child(div().w(px(110.)).font_weight(FontWeight::MEDIUM).child(option.label))
            .child(div().flex_1().text_sm().text_color(note_color).child(note))
            .child(
                Button::new(SharedString::from(format!("try-{}", option.id)))
                    .ghost()
                    .small()
                    .label("Probeer")
                    .disabled(!installed)
                    .on_click(cx.listener(move |this, _, _, cx| this.speak(SAMPLE.into(), option, Speed::Normal, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("pick-{}", option.id)))
                    .small()
                    .outline()
                    .selected(chosen)
                    .disabled(!installed)
                    .label(pick)
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_voice(option, cx))),
            )
    }
}
