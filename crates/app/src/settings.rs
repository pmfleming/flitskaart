//! Instellingen: the voice, the microphone test, the colours and the daily
//! limit for new cards.

use dutch_flashcards_core::store::Appearance;
use dutch_flashcards_core::voice::{self, Gender, SAMPLE, Speed, VoiceOption};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::app::FlashcardsApp;
use crate::ui::{heading, hint};

/// The microphone test is the third section of Instellingen.
pub const MICROPHONE_SECTION: usize = 2;

impl FlashcardsApp {
    fn set_gender(&mut self, gender: Gender, cx: &mut Context<Self>) {
        self.settings.gender = gender;
        self.save_settings();
        let voice = self.current_voice();
        self.speak(SAMPLE.into(), voice, Speed::Normal, cx);
    }

    fn choose_voice(&mut self, choice: &'static VoiceOption, cx: &mut Context<Self>) {
        match choice.gender {
            Gender::Male => self.settings.male_voice = choice.id.into(),
            Gender::Female => self.settings.female_voice = choice.id.into(),
        }
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
        let value = (self.settings.new_per_day as i32 + delta).clamp(0, 100) as u32;
        self.settings.new_per_day = value;
        self.save_settings();
        if self.study.card.is_none() {
            self.load_next_card(cx);
        } else {
            self.study.counts = self.store.counts(Self::now(), value).unwrap_or_default();
        }
        cx.notify();
    }

    pub fn render_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let gender = self.settings.gender;
        let chosen = self.settings.voice_id().to_string();

        let gender_buttons = h_flex().gap_1().children([Gender::Male, Gender::Female].map(|g| {
            Button::new(g.as_str())
                .outline()
                .label(g.label())
                .selected(gender == g)
                .on_click(cx.listener(move |this, _, _, cx| this.set_gender(g, cx)))
        }));

        let voices = v_flex().gap_1().children(voice::voices_for(gender).map(|option| {
            let installed = self.speaker.is_installed(option);
            let is_chosen = option.id == chosen;
            h_flex()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(px(6.))
                .border_1()
                .border_color(if is_chosen { theme.primary } else { theme.border })
                .bg(if is_chosen { theme.accent } else { theme.background })
                .child(div().w(px(110.)).font_weight(FontWeight::MEDIUM).child(option.label))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(if installed { theme.muted_foreground } else { theme.danger })
                        .child(if !installed {
                            "Niet geïnstalleerd".to_string()
                        } else {
                            option.note.to_string()
                        }),
                )
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
                        .selected(is_chosen)
                        .disabled(!installed)
                        .label(if is_chosen { "Gekozen" } else { "Kies" })
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_voice(option, cx))),
                )
        }));

        let missing = voice::VOICES.iter().any(|v| !self.speaker.is_installed(v));
        let per_day = self.settings.new_per_day;

        v_flex()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .p_5()
            .gap_6()
            .child(
                v_flex()
                    .gap_3()
                    .max_w(px(640.))
                    .child(heading("Stem"))
                    .child(hint("Kies een mannen- of vrouwenstem. Pim, Ronnie en Alex spreken Nederlands uit Nederland, Nathalie spreekt Vlaams.", cx))
                    .child(gender_buttons)
                    .child(voices)
                    .when(missing, |d| {
                        d.child(hint("Ontbrekende stemmen installeer je met scripts/fetch-models.sh.", cx))
                    }),
            )
            .child(
                v_flex()
                    .gap_3()
                    .max_w(px(640.))
                    .child(heading("Uitspraak"))
                    .child(hint(
                        "Met Test mijn uitspraak lees je een kaart hardop voor. Whisper luistert op deze laptop en geeft je uitspraak een cijfer van 1 tot 4.",
                        cx,
                    ))
                    .child(if self.listener.is_installed() {
                        hint("Spraakherkenning is geïnstalleerd.", cx)
                    } else {
                        hint("Spraakherkenning ontbreekt. Installeer die met scripts/fetch-models.sh.", cx).text_color(theme.danger)
                    }),
            )
            .child(self.render_mic_test(cx))
            .child(
                v_flex()
                    .gap_3()
                    .child(heading("Weergave"))
                    .child(hint("Systeem volgt de lichte of donkere stand van je bureaublad.", cx))
                    .child(h_flex().gap_1().children(Appearance::ALL.map(|appearance| {
                        Button::new(appearance.as_str())
                            .outline()
                            .label(appearance.label())
                            .selected(self.settings.appearance == appearance)
                            .on_click(cx.listener(move |this, _, window, cx| this.set_appearance(appearance, window, cx)))
                    }))),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(heading("Nieuwe kaarten per dag"))
                    .child(hint("Hoeveel nieuwe kaarten je per dag leert. Herhalingen komen altijd.", cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Button::new("fewer").outline().small().label("−").on_click(cx.listener(
                                |this, _, _, cx| this.change_new_per_day(-1, cx),
                            )))
                            .child(div().w(px(40.)).text_center().text_lg().child(per_day.to_string()))
                            .child(Button::new("more").outline().small().label("+").on_click(cx.listener(
                                |this, _, _, cx| this.change_new_per_day(1, cx),
                            ))),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(heading("Gegevens"))
                    .child(hint(format!("Je kaarten en geluid staan in {}", self.paths.root().display()), cx)),
            )
    }
}
