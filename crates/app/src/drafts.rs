//! Concepten: check drafted cards before they enter the deck.

use dutch_flashcards_core::voice::Speed;
use dutch_flashcards_core::{Card, CardId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex, Sizable as _};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::app::FlashcardsApp;
use crate::ui::{empty_state, heading, hint};

/// One draft with its editable fields.
pub struct DraftRow {
    pub id: CardId,
    pub dutch: Entity<InputState>,
    pub english: Entity<InputState>,
    pub note: Entity<InputState>,
    pub tags: Entity<InputState>,
    pub snippet_id: Option<i64>,
    pub show_english: bool,
    pub show_source: bool,
    pub duplicate: bool,
}

impl DraftRow {
    pub fn new(card: &Card, duplicate: bool, window: &mut Window, cx: &mut App) -> Self {
        let field = |value: &str, placeholder: &'static str, window: &mut Window, cx: &mut App| {
            let value = value.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder))
        };
        DraftRow {
            id: card.id,
            dutch: field(&card.dutch, "Nederlands", window, cx),
            english: field(&card.english, "Vertaling (verborgen bij het oefenen)", window, cx),
            note: field(&card.note, "Uitleg in eenvoudig Nederlands (optioneel)", window, cx),
            tags: field(&card.tags, "Onderwerp", window, cx),
            snippet_id: card.snippet_id,
            show_english: false,
            show_source: false,
            duplicate,
        }
    }

    pub fn values(&self, cx: &App) -> [String; 4] {
        [&self.dutch, &self.english, &self.note, &self.tags].map(|state| state.read(cx).value().to_string())
    }
}

impl FlashcardsApp {
    pub fn reload_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.drafts() {
            Ok(cards) => {
                self.drafts = cards
                    .iter()
                    .map(|card| {
                        let duplicate = self.store.duplicate_of(card).ok().flatten().is_some();
                        DraftRow::new(card, duplicate, window, cx)
                    })
                    .collect();
            }
            Err(err) => self.error(format!("{err:#}")),
        }
    }

    fn row(&self, id: CardId) -> Option<&DraftRow> {
        self.drafts.iter().find(|row| row.id == id)
    }

    /// Saves the edited text, then moves the draft into the deck.
    fn accept_draft(&mut self, id: CardId, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.row(id) else { return false };
        let [dutch, english, note, tags] = row.values(cx);
        let result = self
            .store
            .update_text(id, &dutch, &english, &note, &tags)
            .and_then(|_| self.store.accept(id, Self::now()));
        match result {
            Ok(()) => {
                self.drafts.retain(|row| row.id != id);
                self.prepare_audio(dutch, cx);
                true
            }
            Err(err) => {
                self.error(format!("{err:#}"));
                false
            }
        }
    }

    fn accept_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<CardId> = self.drafts.iter().map(|row| row.id).collect();
        let accepted = ids.into_iter().filter(|id| self.accept_draft(*id, cx)).count();
        self.info(format!("{accepted} kaarten bewaard."));
        self.after_deck_change(cx);
    }

    fn discard_draft(&mut self, id: CardId, cx: &mut Context<Self>) {
        match self.store.delete(id) {
            Ok(()) => self.drafts.retain(|row| row.id != id),
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }

    /// Refreshes everything that depends on the deck.
    pub fn after_deck_change(&mut self, cx: &mut Context<Self>) {
        self.reload_deck();
        if self.study.card.is_none() {
            self.load_next_card(cx);
        } else {
            self.study.counts = self.store.counts(Self::now(), self.settings.new_per_day).unwrap_or_default();
        }
        cx.notify();
    }

    pub fn render_drafts(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.drafts.is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(empty_state("Geen concepten", "Plak een stuk van een gesprek bij Toevoegen om kaarten te maken.", cx))
                .into_any_element();
        }
        let rows: Vec<AnyElement> = self.drafts.iter().map(|row| self.render_draft_row(row, cx).into_any_element()).collect();
        v_flex()
            .size_full()
            .p_5()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(heading(format!("{} concepten", self.drafts.len())))
                            .child(hint("Controleer elke kaart voordat hij in je stapel komt.", cx)),
                    )
                    .child(
                        Button::new("accept-all")
                            .primary()
                            .label("Alles bewaren")
                            .on_click(cx.listener(|this, _, _, cx| this.accept_all(cx))),
                    ),
            )
            .child(
                v_flex()
                    .id("draft-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_3()
                    .pb_4()
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_draft_row(&self, row: &DraftRow, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let id = row.id;
        let source = if row.show_source {
            row.snippet_id.and_then(|sid| self.store.snippet(sid).ok().flatten())
        } else {
            None
        };
        v_flex()
            .p_3()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(Input::new(&row.dutch).large())
            .child(Input::new(&row.note))
            .when(row.show_english, |d| d.child(Input::new(&row.english)))
            .child(h_flex().gap_2().child(div().text_sm().text_color(theme.muted_foreground).child("Onderwerp")).child(div().w(px(220.)).child(Input::new(&row.tags).small())))
            .when(row.duplicate, |d| {
                d.child(div().text_sm().text_color(theme.warning).child("Deze zin staat al in je stapel of bij de concepten."))
            })
            .when_some(source, |d, text| {
                d.child(
                    div()
                        .p_2()
                        .rounded(px(6.))
                        .bg(theme.background)
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(text),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(Button::new(("accept", id as u64)).primary().small().label("Bewaar").on_click(cx.listener(
                        move |this, _, _, cx| {
                            if this.accept_draft(id, cx) {
                                this.after_deck_change(cx);
                            }
                        },
                    )))
                    .child(Button::new(("discard", id as u64)).outline().small().label("Weggooien").on_click(cx.listener(
                        move |this, _, _, cx| this.discard_draft(id, cx),
                    )))
                    .child(Button::new(("listen", id as u64)).ghost().small().label("Luister").on_click(cx.listener(
                        move |this, _, _, cx| {
                            if let Some(row) = this.row(id) {
                                let text = row.dutch.read(cx).value().to_string();
                                this.speak(text, this.current_voice(), Speed::Normal, cx);
                            }
                        },
                    )))
                    .child(
                        Button::new(("english", id as u64))
                            .ghost()
                            .small()
                            .label(if row.show_english { "Verberg vertaling" } else { "Vertaling" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(row) = this.drafts.iter_mut().find(|r| r.id == id) {
                                    row.show_english = !row.show_english;
                                }
                                cx.notify();
                            })),
                    )
                    .when(row.snippet_id.is_some(), |d| {
                        d.child(
                            Button::new(("source", id as u64))
                                .ghost()
                                .small()
                                .label(if row.show_source { "Verberg bron" } else { "Bron" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(row) = this.drafts.iter_mut().find(|r| r.id == id) {
                                        row.show_source = !row.show_source;
                                    }
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }
}
