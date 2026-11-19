//! Toevoegen: paste a piece of conversation and get draft cards.

use dutch_flashcards_core::drafting::{self, DEFAULT_LIMIT};
use dutch_flashcards_core::model::NewCard;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex, Disableable as _};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::app::{FlashcardsApp, Screen};
use crate::ui::{heading, hint};

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
            steer: cx.new(|cx| InputState::new(window, cx).placeholder("bijvoorbeeld: alleen de correcties, of: max 3 kaarten")),
            busy: false,
            error: None,
        }
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
        cx.notify();
        let limit = limit_from(&steer);
        let job = {
            let (snippet, steer) = (snippet.clone(), steer.clone());
            cx.background_executor().spawn(async move { drafting::draft_cards(&snippet, &steer, limit) })
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.add.busy = false;
                match result {
                    Ok(cards) if cards.is_empty() => {
                        this.add.error = Some("Claude vond niets om te leren in dit stuk.".into());
                    }
                    Ok(cards) => this.store_drafts(&snippet, &steer, cards, window, cx),
                    Err(err) => this.add.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn store_drafts(&mut self, snippet: &str, steer: &str, cards: Vec<NewCard>, window: &mut Window, cx: &mut Context<Self>) {
        let now = Self::now();
        let stored = self
            .store
            .add_snippet(snippet, steer, now)
            .and_then(|id| self.store.add_drafts(Some(id), &cards, now));
        match stored {
            Ok(ids) => {
                self.add.snippet.update(cx, |s, cx| s.set_value("", window, cx));
                self.add.steer.update(cx, |s, cx| s.set_value("", window, cx));
                self.reload_drafts(window, cx);
                self.show(Screen::Drafts, window, cx);
                self.info(format!("{} concepten gemaakt. Controleer ze en bewaar wat je wilt leren.", ids.len()));
            }
            Err(err) => self.add.error = Some(format!("De kaarten zijn niet bewaard: {err:#}").into()),
        }
        cx.notify();
    }

    pub fn render_add(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status: SharedString = if self.add.busy {
            "Claude maakt kaarten… dit duurt meestal 5 tot 15 seconden.".into()
        } else {
            "Je kunt ook zelf regels typen als: Nederlands | English | uitleg".into()
        };
        v_flex()
            .size_full()
            .p_5()
            .gap_4()
            .child(heading("Kaarten maken van een gesprek"))
            .child(hint(
                "Plak een stuk van een gesprek. Correcties van je eigen fouten worden eerst kaarten, daarna nieuwe zinnen en woorden.",
                cx,
            ))
            .child(Textarea::new(&self.add.snippet).h(px(280.)))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().text_color(theme.muted_foreground).child("Aanwijzing (optioneel)"))
                    .child(Input::new(&self.add.steer)),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("make")
                            .primary()
                            .label(if self.add.busy { "Bezig…" } else { "Maak kaarten" })
                            .loading(self.add.busy)
                            .disabled(self.add.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.make_cards(window, cx))),
                    )
                    .child(hint(status, cx)),
            )
            .when_some(self.add.error.clone(), |d, error| d.child(div().text_sm().text_color(cx.theme().danger).child(error)))
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
