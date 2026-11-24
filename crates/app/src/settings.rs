//! Instellingen: the voice, the microphone & speech check, the colours and
//! the daily limit for new cards.

use dutch_flashcards_core::store::Appearance;
use dutch_flashcards_core::voice::{self, Gender, SAMPLE, Speed, VoiceOption};
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app::FlashcardsApp;
use crate::icons;
use crate::ui::{explained, hint, icon_button, section, segment_button, segmented_bar, surface};

/// The microphone & speech section is the second item inside Instellingen.
pub const MICROPHONE_SECTION: usize = 1;
/// The language model and usage sections, for `--keys taalmodel` and `verbruik`.
pub const LLM_SECTION: usize = 3;
pub const USAGE_SECTION: usize = 4;

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
        let appearance_icons = [icons::system, icons::light, icons::dark];
        let appearance_card = section(icons::light(), "Weergave", cx)
            .flex_1()
            .min_w(px(280.))
            .child(h_flex().child(segmented_bar(cx).children(Appearance::ALL.map(|appearance| {
                segment_button(appearance.as_str(), appearance.label(), self.settings.appearance == appearance, cx)
                    .icon(appearance_icons[appearance as usize]())
                    .when(appearance == Appearance::System, |b| b.tooltip("Volgt je bureaublad"))
                    .on_click(cx.listener(move |this, _, window, cx| this.set_appearance(appearance, window, cx)))
            }))));

        let per_day = self.settings.new_per_day;
        let limit_card = section(icons::per_day(), "Nieuw per dag", cx)
            .flex_1()
            .min_w(px(240.))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("fewer")
                            .outline()
                            .small()
                            .icon(icons::minus())
                            .tooltip("Minder")
                            .disabled(per_day == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.change_new_per_day(-1, cx))),
                    )
                    .child(
                        explained("per-day", "Nieuwe kaarten per dag. Herhalingen komen altijd.")
                            .w(px(44.))
                            .text_center()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(per_day.to_string()),
                    )
                    .child(
                        Button::new("more")
                            .outline()
                            .small()
                            .icon(icons::plus())
                            .tooltip("Meer")
                            .disabled(per_day >= 100)
                            .on_click(cx.listener(|this, _, _, cx| this.change_new_per_day(1, cx))),
                    ),
            );
        let data_card = section(icons::folder(), "Gegevens", cx).child(
            explained("data-folder", "Hier staan je kaarten en geluid")
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(self.paths.root().display().to_string()),
        );

        // The sections are the scroll area's own children, so it can bring
        // one into view by its place (see MICROPHONE_SECTION).
        let sections = [
            self.render_voices(cx),
            self.render_mic_test(cx),
            h_flex().flex_wrap().items_stretch().gap_4().child(appearance_card).child(limit_card),
            self.render_llm(cx),
            self.render_usage(cx),
            data_card,
        ];
        v_flex()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .p_5()
            .pb_10()
            .gap_4()
            .children(sections.map(|section| section.w_full().max_w(px(720.)).mx_auto()))
    }

    /// Man or woman, then the voices of that gender to try and choose.
    fn render_voices(&self, cx: &mut Context<Self>) -> Div {
        let gender = self.settings.gender;
        let genders = h_flex().child(segmented_bar(cx).children([Gender::Male, Gender::Female].map(|g| {
            segment_button(g.as_str(), g.label(), gender == g, cx).on_click(cx.listener(move |this, _, _, cx| this.set_gender(g, cx)))
        })));
        let missing = voice::VOICES.iter().any(|v| !self.sound.speaker.is_installed(v));
        section(icons::voice(), "Stem", cx)
            .child(genders)
            .child(v_flex().gap_1p5().children(voice::voices_for(gender).map(|option| self.render_voice(option, cx)).collect::<Vec<_>>()))
            .when(missing, |d| d.child(hint("Ontbrekende stemmen: scripts/fetch-models.sh", cx)))
    }

    fn render_voice(&self, option: &'static VoiceOption, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let installed = self.sound.speaker.is_installed(option);
        let chosen = option.id == self.settings.voice_id();
        let (note, note_color) = if installed { (option.note, theme.muted_foreground) } else { ("Niet geïnstalleerd", theme.danger) };
        let (border, background) = if chosen { (theme.primary, theme.accent) } else { (theme.border, surface(cx)) };
        let radio = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(16.))
            .rounded_full()
            .border_1()
            .border_color(if chosen { theme.primary } else { theme.border })
            .when(chosen, |d| d.child(div().size(px(8.)).rounded_full().bg(theme.primary)));

        let select_area = h_flex()
            .id(SharedString::from(format!("pick-{}", option.id)))
            .flex_1()
            .min_w_0()
            .gap_3()
            .py_2()
            .pl_3()
            .items_center()
            .when(installed, |d| {
                d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.choose_voice(option, cx)))
            })
            .child(radio)
            .child(
                div()
                    .font_weight(if chosen { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                    .text_color(if chosen { theme.accent_foreground } else { theme.foreground })
                    .child(option.label),
            )
            .when(!note.is_empty(), |d| d.child(div().text_sm().text_color(note_color).child(note)));

        h_flex()
            .gap_2()
            .pr_2()
            .items_center()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .bg(background)
            .when(!installed, |d| d.opacity(0.6))
            .when(installed && !chosen, |d| d.hover(move |s| s.bg(theme.muted)))
            .child(select_area)
            .child(
                icon_button(SharedString::from(format!("try-{}", option.id)), icons::listen(), "Probeer")
                    .disabled(!installed)
                    .on_click(cx.listener(move |this, _, _, cx| this.speak(SAMPLE.into(), option, Speed::Normal, cx))),
            )
    }
}
