//! Concepten: check drafted cards before they enter the deck.

use dutch_flashcards_core::{Card, CardId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Div, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app::{FlashcardsApp, Screen};
use crate::icons;
use crate::ui::{card_box, empty_state, field_label, heading, hint};

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
        let mut field = |value: &str, placeholder: &'static str| {
            let value = value.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder))
        };
        DraftRow {
            id: card.id,
            dutch: field(&card.dutch, "Nederlandse zin of uitdrukking"),
            english: field(&card.english, "Vertaling (verborgen bij het oefenen)"),
            note: field(&card.note, "Korte uitleg in eenvoudig Nederlands (optioneel)"),
            tags: field(&card.tags, "bijv. boodschappen, werk, grammatica"),
            snippet_id: card.snippet_id,
            show_english: false,
            show_source: false,
            duplicate,
        }
    }

    /// Dutch, English, note and tags as typed.
    pub fn values(&self, cx: &App) -> [String; 4] {
        [&self.dutch, &self.english, &self.note, &self.tags].map(|state| state.read(cx).value().to_string())
    }

    /// The Dutch and the note, and the translation when `english` is on, each labeled.
    pub fn fields(&self, english: bool, cx: &App) -> Div {
        v_flex()
            .gap_2p5()
            .child(v_flex().gap_1().child(field_label("Nederlands", cx)).child(Input::new(&self.dutch).large()))
            .child(v_flex().gap_1().child(field_label("Uitleg (optioneel)", cx)).child(Input::new(&self.note)))
            .when(english, |d| {
                d.child(v_flex().gap_1().child(field_label("Vertaling", cx)).child(Input::new(&self.english)))
            })
    }

    /// Reads the Dutch as typed so far.
    pub fn listen_button(&self, id: impl Into<ElementId>, cx: &mut Context<FlashcardsApp>) -> Button {
        let dutch = self.dutch.clone();
        Button::new(id)
            .ghost()
            .small()
            .icon(icons::listen())
            .label("Luister")
            .on_click(cx.listener(move |this, _, _, cx| {
                let text = dutch.read(cx).value().to_string();
                this.say(text, cx);
            }))
    }
}

impl FlashcardsApp {
    pub fn reload_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.drafts() {
            Ok(cards) => {
                let rows = cards.iter().map(|card| {
                    let duplicate = self.store.duplicate_of(card).ok().flatten().is_some();
                    DraftRow::new(card, duplicate, window, cx)
                });
                self.drafts = rows.collect();
            }
            Err(err) => self.error(format!("{err:#}")),
        }
    }

    /// Saves the edited text, then moves the draft into the deck.
    fn accept_draft(&mut self, id: CardId, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.drafts.iter().find(|row| row.id == id) else { return false };
        let [dutch, english, note, tags] = row.values(cx);
        let result = self.store.update_text(id, &dutch, &english, &note, &tags).and_then(|_| self.store.accept(id, Self::now()));
        if let Err(err) = result {
            self.error(format!("{err:#}"));
            return false;
        }
        self.drafts.retain(|row| row.id != id);
        self.prepare_audio(dutch, cx);
        true
    }

    fn accept_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<CardId> = self.drafts.iter().map(|row| row.id).collect();
        let accepted = ids.into_iter().filter(|id| self.accept_draft(*id, cx)).count();
        self.info(format!("{accepted} kaarten bewaard."), cx);
        self.after_deck_change(cx);
    }

    fn discard_draft(&mut self, id: CardId, cx: &mut Context<Self>) {
        match self.store.delete(id) {
            Ok(()) => self.drafts.retain(|row| row.id != id),
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }

    /// Flips one of a draft's switches, such as showing its translation.
    fn toggle_draft(&mut self, id: CardId, switch: fn(&mut DraftRow) -> &mut bool, cx: &mut Context<Self>) {
        if let Some(row) = self.drafts.iter_mut().find(|row| row.id == id) {
            let on = switch(row);
            *on = !*on;
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

    pub fn render_drafts(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.drafts.is_empty() {
            return v_flex()
                .size_full()
                .p_5()
                .items_center()
                .justify_center()
                .child(
                    card_box(cx)
                        .w_full()
                        .max_w(px(520.))
                        .px_6()
                        .pb_6()
                        .items_center()
                        .child(empty_state("Geen concepten", "Plak een stuk van een gesprek bij Toevoegen om kaarten te maken.", cx))
                        .child(
                            Button::new("drafts-empty-add")
                                .primary()
                                .label("Kaarten toevoegen")
                                .on_click(cx.listener(|this, _, window, cx| this.show(Screen::Add, window, cx))),
                        ),
                )
                .into_any_element();
        }
        let total = self.drafts.len();
        let duplicates = self.drafts.iter().filter(|r| r.duplicate).count();
        let subtitle = if duplicates > 0 {
            format!("Controleer elke kaart voordat hij in je stapel komt · {duplicates} mogelijk dubbel.")
        } else {
            "Controleer elke kaart voordat hij in je stapel komt.".to_string()
        };
        let rows: Vec<_> = self.drafts.iter().map(|row| self.render_draft_row(row, cx)).collect();
        let title = v_flex()
            .gap_0p5()
            .child(heading(if total == 1 { "1 concept".to_string() } else { format!("{total} concepten") }))
            .child(hint(subtitle, cx));
        let accept_all = Button::new("accept-all")
            .primary()
            .label(format!("Alles bewaren ({total})"))
            .on_click(cx.listener(|this, _, _, cx| this.accept_all(cx)));
        v_flex()
            .size_full()
            .p_5()
            .gap_4()
            .child(h_flex().w_full().max_w(px(820.)).mx_auto().justify_between().items_center().gap_3().child(title).child(accept_all))
            .child(
                v_flex()
                    .id("draft-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .max_w(px(820.))
                    .mx_auto()
                    .overflow_y_scroll()
                    .gap_3()
                    .pb_6()
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_draft_row(&self, row: &DraftRow, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let source = row.snippet_id.filter(|_| row.show_source).and_then(|id| self.store.snippet(id).ok().flatten());
        let tags = v_flex()
            .gap_1()
            .w(px(280.))
            .child(field_label("Onderwerp", cx))
            .child(Input::new(&row.tags).small());
        let duplicate_banner = row.duplicate.then(|| {
            h_flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.warning.opacity(0.45))
                .bg(theme.warning.opacity(0.14))
                .child(div().flex_none().size(px(8.)).rounded_full().bg(theme.warning))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(theme.foreground).child("Mogelijk dubbel:"))
                .child(div().text_sm().text_color(theme.foreground).child("deze zin staat al in je stapel of bij de concepten."))
        });
        card_box(cx)
            .p_4()
            .gap_3()
            .when(row.duplicate, |d| d.border_color(theme.warning.opacity(0.65)))
            .children(duplicate_banner)
            .child(row.fields(row.show_english, cx))
            .child(tags)
            .when_some(source, |d, text| {
                d.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(text),
                )
            })
            .child(self.render_draft_buttons(row, cx))
    }

    /// Inspection tools on the left; discard/keep decisions cleanly separated on the right.
    fn render_draft_buttons(&self, row: &DraftRow, cx: &mut Context<Self>) -> impl IntoElement {
        let id = row.id;
        let english_label = if row.show_english { "Verberg vertaling" } else { "Vertaling" };
        let source_label = if row.show_source { "Verberg bron" } else { "Bron" };
        let mut english_btn = Button::new(("english", id as u64))
            .ghost()
            .small()
            .icon(icons::meaning())
            .label(english_label)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_draft(id, |row| &mut row.show_english, cx)));
        if row.show_english {
            english_btn = english_btn.outline().primary();
        }
        let mut source_btn = Button::new(("source", id as u64))
            .ghost()
            .small()
            .label(source_label)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_draft(id, |row| &mut row.show_source, cx)));
        if row.show_source {
            source_btn = source_btn.outline().primary();
        }
        let inspect = h_flex()
            .gap_1p5()
            .flex_wrap()
            .child(row.listen_button(("listen", id as u64), cx))
            .child(english_btn)
            .when(row.snippet_id.is_some(), |d| d.child(source_btn));
        let decide = h_flex()
            .gap_2()
            .child(
                Button::new(("discard", id as u64))
                    .ghost()
                    .small()
                    .label("Weggooien")
                    .on_click(cx.listener(move |this, _, _, cx| this.discard_draft(id, cx))),
            )
            .child(
                Button::new(("accept", id as u64))
                    .primary()
                    .small()
                    .label("Bewaar")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.accept_draft(id, cx) {
                            this.after_deck_change(cx);
                        }
                    })),
            );
        h_flex()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .justify_between()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(inspect)
            .child(decide)
    }
}
