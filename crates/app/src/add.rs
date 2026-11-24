//! Toevoegen: paste a piece of conversation and get draft cards.

use dutch_flashcards_core::drafting::{self, DEFAULT_LIMIT};
use dutch_flashcards_core::model::NewCard;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, Styled as _, Window, div, px,
};

use crate::app::{FlashcardsApp, Screen};
use crate::icons;
use crate::making::Origin;
use crate::ui::{card_box, explained, hint, keycap_primary, titled};

const STEER_PRESETS: [&str; 3] = ["alleen de correcties", "max 3 kaarten", "alleen nieuwe woorden"];

pub struct AddState {
    pub snippet: Entity<TextareaState>,
    pub steer: Entity<InputState>,
    pub busy: bool,
    pub error: Option<SharedString>,
}

impl AddState {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        AddState {
            snippet: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Plak hier een stuk van je gesprek met Claude, ChatGPT of Gemini…")
            }),
            steer: cx.new(|cx| InputState::new(window, cx).placeholder("Aanwijzing, bijvoorbeeld: max 3 kaarten")),
            busy: false,
            error: None,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.snippet.focus_handle(cx).focus(window, cx);
    }
}

/// "max 3 kaarten" in the steering line raises or lowers the limit.
fn limit_from(steer: &str) -> usize {
    steer
        .split(|c: char| !c.is_ascii_digit())
        .find_map(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=20).contains(n))
        .unwrap_or(DEFAULT_LIMIT)
}

impl FlashcardsApp {
    fn make_cards(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add.busy {
            return;
        }
        let snippet = self.add.snippet.read(cx).value().to_string();
        let steer = self.add.steer.read(cx).value().to_string();
        if snippet.trim().is_empty() {
            self.add.error = Some("Plak eerst een stuk van een gesprek.".into());
            cx.notify();
            return;
        }
        self.add.error = None;
        if let Some(cards) = drafting::parse_manual(&snippet) {
            self.store_drafts(&snippet, &steer, cards, window, cx);
            return;
        }
        self.add.busy = true;
        self.start_making(Origin::Add, &format!("{snippet}\n{steer}"), cx);
        let (limit, llm) = (limit_from(&steer), self.llm.clone());
        // The job hands the text back with its result, for storing alongside the drafts.
        let job = move || {
            let result = drafting::draft_cards(&llm, &snippet, &steer, limit);
            (snippet, steer, result)
        };
        self.in_background_with_window(cx, job, |this, (snippet, steer, result), window, cx| {
            this.add.busy = false;
            match result {
                Ok(cards) if cards.is_empty() => {
                    let message = "Het taalmodel vond niets om te leren in dit stuk.";
                    this.add.error = Some(message.into());
                    this.making_failed(Origin::Add, message);
                }
                Ok(cards) => {
                    this.making_done();
                    this.store_drafts(&snippet, &steer, cards, window, cx);
                }
                Err(err) => {
                    let message = crate::logging::error("toevoegen", format!("{err:#}"));
                    this.making_failed(Origin::Add, format!("Je tekst staat nog in Toevoegen. ({message})"));
                    this.add.error = Some(message.into());
                }
            }
        });
    }

    fn store_drafts(&mut self, snippet: &str, steer: &str, cards: Vec<NewCard>, window: &mut Window, cx: &mut Context<Self>) {
        match self.keep_drafts(snippet, steer, &cards, window, cx) {
            Ok(()) => {
                self.add.snippet.update(cx, |s, cx| s.set_value("", window, cx));
                self.add.steer.update(cx, |s, cx| s.set_value("", window, cx));
            }
            Err(err) => self.add.error = Some(crate::logging::error("toevoegen", format!("De kaarten zijn niet bewaard: {err:#}")).into()),
        }
        cx.notify();
    }

    /// Stores the source text and its drafts, then opens Concepten to check them.
    pub fn keep_drafts(&mut self, source: &str, steer: &str, cards: &[NewCard], window: &mut Window, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let now = Self::now();
        let ids = self.store.add_snippet(source, steer, now).and_then(|id| self.store.add_drafts(Some(id), cards, now))?;
        self.reload_drafts(window, cx);
        self.show(Screen::Drafts, window, cx);
        self.info(format!("{} concepten gemaakt. Controleer ze en bewaar wat je wilt leren.", ids.len()), cx);
        Ok(())
    }

    fn on_add_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &event.keystroke.modifiers;
        if (m.control || m.platform) && event.keystroke.key == "enter" && !event.is_held {
            self.make_cards(window, cx);
            cx.stop_propagation();
        }
    }

    pub fn render_add(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let snippet_text = self.add.snippet.read(cx).value();
        let manual_count = (!snippet_text.trim().is_empty())
            .then(|| drafting::parse_manual(&snippet_text).map(|cards| cards.len()))
            .flatten();

        let status = if self.add.busy {
            hint(format!("{} maakt kaarten…", self.llm.cli.name()), cx).into_any_element()
        } else if let Some(count) = manual_count {
            let label = if count == 1 { "1 kaart, zonder taalmodel".to_string() } else { format!("{count} kaarten, zonder taalmodel") };
            h_flex()
                .items_center()
                .gap_1()
                .px_2()
                .py(px(1.5))
                .rounded_full()
                .bg(theme.accent)
                .text_color(theme.accent_foreground)
                .text_xs()
                .child(icons::check().size_3p5())
                .child(label)
                .into_any_element()
        } else {
            explained("manual-tip", "Regels als  Nederlands | English | uitleg  worden direct kaarten, zonder taalmodel.")
                .text_color(theme.muted_foreground)
                .child(icons::tip().size_5())
                .into_any_element()
        };

        let presets = h_flex().flex_wrap().gap_1p5().children(STEER_PRESETS.map(|preset| {
            Button::new(SharedString::from(format!("steer-{preset}")))
                .ghost()
                .xsmall()
                .label(preset)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.add.steer.update(cx, |state, cx| state.set_value(preset, window, cx));
                    cx.notify();
                }))
        }));
        let steer_icon = explained("steer-tip", "Aanwijzing voor het taalmodel (optioneel)").text_color(theme.muted_foreground).child(icons::steer().size_4());

        let submit = Button::new("make")
            .primary()
            .loading(self.add.busy)
            .disabled(self.add.busy)
            .when(!self.add.busy, |b| b.icon(icons::cards()))
            .child(h_flex().gap_2().child("Maak kaarten").when(!self.add.busy, |d| d.child(keycap_primary("Ctrl+Enter", cx))))
            .on_click(cx.listener(|this, _, window, cx| this.make_cards(window, cx)));

        v_flex()
            .size_full()
            .p_5()
            .on_key_down(cx.listener(Self::on_add_key))
            .child(
                card_box(cx)
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .max_w(px(760.))
                    .mx_auto()
                    .p_5()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(titled(icons::paste(), "Kaarten van een gesprek", cx))
                            .child(hint("Correcties eerst, dan zinnen en woorden.", cx)),
                    )
                    .child(Textarea::new(&self.add.snippet).flex_1().min_h(px(100.)).max_h(px(400.)))
                    .child(
                        v_flex()
                            .gap_1p5()
                            .child(Input::new(&self.add.steer).prefix(steer_icon))
                            .child(presets),
                    )
                    .child(h_flex().flex_wrap().items_center().gap_3().child(submit).child(status))
                    .when_some(self.add.error.clone(), |d, error| d.child(div().text_sm().text_color(theme.danger).child(error))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_LIMIT, limit_from};

    #[test]
    fn reads_a_limit_from_the_steering_line() {
        assert_eq!(limit_from(""), DEFAULT_LIMIT);
        assert_eq!(limit_from("max 3 kaarten"), 3);
        assert_eq!(limit_from("alleen de correcties"), DEFAULT_LIMIT);
        assert_eq!(limit_from("max 50"), DEFAULT_LIMIT);
    }
}
