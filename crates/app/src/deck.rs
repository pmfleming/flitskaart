//! Kaarten: every card in the deck, with search and editing.

use dutch_flashcards_core::schedule::describe_due;
use dutch_flashcards_core::voice::Speed;
use dutch_flashcards_core::{Card, CardId, Status};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex, Sizable as _};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::app::FlashcardsApp;
use crate::drafts::DraftRow;
use crate::ui::{empty_state, hint};

pub struct DeckState {
    pub search: Entity<InputState>,
    pub cards: Vec<Card>,
    /// The card being edited, reusing the draft row's fields.
    pub editing: Option<DraftRow>,
    pub confirm_delete: Option<CardId>,
    _search_changed: Option<Subscription>,
}

impl DeckState {
    pub fn new(window: &mut Window, cx: &mut Context<FlashcardsApp>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Zoek een woord, zin of onderwerp"));
        let subscription = cx.subscribe_in(&search, window, |this: &mut FlashcardsApp, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.deck_query = state.read(cx).value().to_string();
                this.reload_deck();
                cx.notify();
            }
        });
        DeckState { search, cards: Vec::new(), editing: None, confirm_delete: None, _search_changed: Some(subscription) }
    }
}

impl FlashcardsApp {
    pub fn reload_deck(&mut self) {
        let query = self.deck_query.clone();
        match self.store.deck(&query) {
            Ok(cards) => self.deck.cards = cards,
            Err(err) => self.error(format!("{err:#}")),
        }
    }

    pub fn start_editing(&mut self, card: &Card, window: &mut Window, cx: &mut Context<Self>) {
        self.deck.editing = Some(DraftRow::new(card, false, window, cx));
        self.deck.confirm_delete = None;
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(row) = &self.deck.editing else { return };
        let [dutch, english, note, tags] = row.values(cx);
        let id = row.id;
        match self.store.update_text(id, &dutch, &english, &note, &tags) {
            Ok(()) => {
                self.deck.editing = None;
                if let Some(card) = self.study.card.as_mut().filter(|c| c.id == id) {
                    card.dutch = dutch.trim().to_string();
                    card.english = english.trim().to_string();
                    card.note = note.trim().to_string();
                    card.tags = tags.trim().to_string();
                }
                self.prepare_audio(dutch, cx);
                self.reload_deck();
                self.info("Kaart bewaard.");
            }
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }

    fn deck_action(&mut self, id: CardId, action: DeckAction, cx: &mut Context<Self>) {
        let now = Self::now();
        let result = match action {
            DeckAction::Suspend => self.store.set_suspended(id, true),
            DeckAction::Resume => self.store.set_suspended(id, false),
            DeckAction::Reset => self.store.reset(id, now),
            DeckAction::Delete => self.store.delete(id),
        };
        if let Err(err) = result {
            self.error(format!("{err:#}"));
        }
        self.deck.confirm_delete = None;
        if self.study.card.as_ref().is_some_and(|c| c.id == id) {
            self.study.card = None;
        }
        self.after_deck_change(cx);
    }

    pub fn render_deck(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Self::now();
        let body = if self.deck.cards.is_empty() {
            let (title, detail) = if self.deck_query.trim().is_empty() {
                ("Nog geen kaarten", "Kaarten die je bij Concepten bewaart, verschijnen hier.")
            } else {
                ("Niets gevonden", "Probeer een ander woord.")
            };
            empty_state(title, detail, cx).into_any_element()
        } else {
            let rows: Vec<AnyElement> = self
                .deck
                .cards
                .iter()
                .map(|card| {
                    if self.deck.editing.as_ref().is_some_and(|row| row.id == card.id) {
                        self.render_edit_row(cx).into_any_element()
                    } else {
                        self.render_deck_row(card, now, cx).into_any_element()
                    }
                })
                .collect();
            v_flex()
                .id("deck-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(rows)
                .into_any_element()
        };
        let total = self.deck.cards.len();
        v_flex()
            .size_full()
            .p_5()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(Input::new(&self.deck.search)))
                    .child(hint(format!("{total} kaarten"), cx)),
            )
            .child(
                h_flex()
                    .gap_3()
                    .px_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().child("Nederlands"))
                    .child(div().w(px(110.)).child("Volgende keer"))
                    .child(div().w(px(56.)).text_right().child("Fout"))
                    .child(div().w(px(70.)).text_right().child("Vertaling"))
                    .child(div().w(px(76.)).text_right().child("Uitspraak"))
                    .child(div().w(px(250.))),
            )
            .child(body)
    }

    fn render_deck_row(&self, card: &Card, now: chrono::DateTime<chrono::Utc>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let id = card.id;
        let suspended = card.status == Status::Suspended;
        let due = if suspended {
            "gepauzeerd".to_string()
        } else if card.is_new() {
            "nieuw".to_string()
        } else {
            card.due.map(|d| describe_due(d, now)).unwrap_or_default()
        };
        let confirming = self.deck.confirm_delete == Some(id);
        let actions = if confirming {
            h_flex()
                .gap_1()
                .child(div().text_sm().text_color(theme.danger).child("Verwijderen?"))
                .child(Button::new(("delete-yes", id as u64)).danger().xsmall().label("Ja").on_click(cx.listener(
                    move |this, _, _, cx| this.deck_action(id, DeckAction::Delete, cx),
                )))
                .child(Button::new(("delete-no", id as u64)).ghost().xsmall().label("Nee").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.deck.confirm_delete = None;
                        cx.notify();
                    },
                )))
        } else {
            let card_for_edit = card.clone();
            h_flex()
                .gap_1()
                .child(Button::new(("edit", id as u64)).ghost().xsmall().label("Bewerken").on_click(cx.listener(
                    move |this, _, window, cx| this.start_editing(&card_for_edit, window, cx),
                )))
                .child(
                    Button::new(("pause", id as u64))
                        .ghost()
                        .xsmall()
                        .label(if suspended { "Hervat" } else { "Pauzeer" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let action = if suspended { DeckAction::Resume } else { DeckAction::Suspend };
                            this.deck_action(id, action, cx)
                        })),
                )
                .when(!card.is_new(), |d| {
                    d.child(Button::new(("reset", id as u64)).ghost().xsmall().label("Opnieuw").on_click(cx.listener(
                        move |this, _, _, cx| this.deck_action(id, DeckAction::Reset, cx),
                    )))
                })
                .child(Button::new(("delete", id as u64)).ghost().xsmall().label("Verwijder").on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.deck.confirm_delete = Some(id);
                        cx.notify();
                    },
                )))
        };
        h_flex()
            .gap_3()
            .px_2()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .when(suspended, |d| d.text_color(theme.muted_foreground))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().child(card.dutch.clone()))
                    .when(!card.tags.is_empty(), |d| d.child(div().text_xs().text_color(theme.muted_foreground).child(card.tags.clone()))),
            )
            .child(div().w(px(110.)).text_sm().child(due))
            .child(div().w(px(56.)).text_sm().text_right().child(card.misses.to_string()))
            .child(div().w(px(70.)).text_sm().text_right().child(card.peeks.to_string()))
            .child(
                div()
                    .w(px(76.))
                    .text_sm()
                    .text_right()
                    .child(card.speech_accuracy.map_or("–".to_string(), |a| format!("{:.0}%", a * 100.0))),
            )
            .child(h_flex().w(px(250.)).justify_end().child(actions))
    }

    fn render_edit_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let row = self.deck.editing.as_ref().unwrap();
        v_flex()
            .my_1()
            .p_3()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.primary)
            .bg(theme.secondary)
            .child(Input::new(&row.dutch).large())
            .child(Input::new(&row.note))
            .child(Input::new(&row.english))
            .child(Input::new(&row.tags).small())
            .child(
                h_flex()
                    .gap_2()
                    .child(Button::new("edit-save").primary().small().label("Opslaan").on_click(cx.listener(
                        |this, _, _, cx| this.save_edit(cx),
                    )))
                    .child(Button::new("edit-cancel").outline().small().label("Annuleren").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.deck.editing = None;
                            cx.notify();
                        },
                    )))
                    .child(Button::new("edit-listen").ghost().small().label("Luister").on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(row) = &this.deck.editing {
                                let text = row.dutch.read(cx).value().to_string();
                                this.speak(text, this.current_voice(), Speed::Normal, cx);
                            }
                        },
                    ))),
            )
    }
}

#[derive(Clone, Copy)]
enum DeckAction {
    Suspend,
    Resume,
    Reset,
    Delete,
}
