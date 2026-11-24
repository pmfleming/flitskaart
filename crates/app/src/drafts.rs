//! Concepten: check drafted cards before they enter the deck.

use dutch_flashcards_core::{Card, CardId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Div, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app::{FlashcardsApp, Screen, go_buttons};
use crate::icons;
use crate::ui::{card_box, empty_state, explained, icon_button, stat, titled};

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

    /// The Dutch, the note, the translation when `english` is on, and the
    /// topic, each marked by its icon.
    pub fn fields(&self, english: bool, cx: &App) -> Div {
        let id = self.id as u64;
        let mark = |name: &'static str, icon: Icon, tip: &'static str| {
            explained((name, id), tip).flex_none().text_color(cx.theme().muted_foreground).child(icon.size_4())
        };
        v_flex()
            .gap_2()
            .child(Input::new(&self.dutch).large().prefix(mark("field-dutch", icons::dutch(), "Nederlands")))
            .child(Input::new(&self.note).prefix(mark("field-note", icons::tip(), "Uitleg (optioneel)")))
            .when(english, |d| d.child(Input::new(&self.english).prefix(mark("field-english", icons::meaning(), "Vertaling"))))
            .child(div().w(px(280.)).child(Input::new(&self.tags).small().prefix(mark("field-tag", icons::tag(), "Onderwerp"))))
    }

    /// Reads the Dutch as typed so far.
    pub fn listen_button(&self, id: impl Into<ElementId>, cx: &mut Context<FlashcardsApp>) -> Button {
        let dutch = self.dutch.clone();
        icon_button(id, icons::listen(), "Luister").on_click(cx.listener(move |this, _, _, cx| {
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
            self.study.counts = self.store.counts(Self::now(), self.new_limit()).unwrap_or_default();
        }
        cx.notify();
    }

    pub fn render_drafts(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let making = self.render_making(cx);
        if self.drafts.is_empty() && making.is_some() {
            return v_flex()
                .id("drafts-making")
                .size_full()
                .p_5()
                .overflow_y_scroll()
                .child(div().w_full().max_w(px(820.)).mx_auto().children(making))
                .into_any_element();
        }
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
                        .gap_4()
                        .items_center()
                        .child(empty_state(icons::drafts(), "Geen concepten", "Nieuwe kaarten wachten hier tot je ze bewaart.", cx))
                        .child(h_flex().gap_2().children(go_buttons(vec![Screen::Talk, Screen::Add], cx))),
                )
                .into_any_element();
        }
        let theme = cx.theme().clone();
        let total = self.drafts.len();
        let duplicates = self.drafts.iter().filter(|r| r.duplicate).count();
        let rows: Vec<_> = self.drafts.iter().map(|row| self.render_draft_row(row, cx)).collect();
        let title = h_flex()
            .items_center()
            .gap_3()
            .child(titled(icons::drafts(), if total == 1 { "1 concept".to_string() } else { format!("{total} concepten") }, cx))
            .when(duplicates > 0, |d| {
                d.child(stat("duplicates", icons::duplicate(), duplicates.to_string(), "Mogelijk dubbel", cx).text_color(theme.warning))
            });
        let accept_all = Button::new("accept-all")
            .primary()
            .icon(icons::check_all())
            .label(format!("Bewaar alle {total}"))
            .on_click(cx.listener(|this, _, _, cx| this.accept_all(cx)));
        v_flex()
            .size_full()
            .p_5()
            .gap_4()
            .children(making.map(|panel| div().w_full().max_w(px(820.)).mx_auto().child(panel)))
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
        card_box(cx)
            .p_4()
            .gap_3()
            .when(row.duplicate, |d| d.border_color(theme.warning.opacity(0.65)))
            .child(row.fields(row.show_english, cx))
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
        let theme = cx.theme().clone();
        let toggle = |name: &'static str, icon: Icon, (show, hide): (&'static str, &'static str), on: bool| {
            icon_button((name, id as u64), icon, if on { hide } else { show }).selected(on)
        };
        let english_btn = toggle("english", icons::meaning(), ("Vertaling", "Verberg vertaling"), row.show_english)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_draft(id, |row| &mut row.show_english, cx)));
        let source_btn = toggle("source", icons::source(), ("Bron", "Verberg bron"), row.show_source)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_draft(id, |row| &mut row.show_source, cx)));
        let duplicate = row.duplicate.then(|| {
            explained(("duplicate", id as u64), "Deze zin staat al in je stapel of bij de concepten.")
                .flex()
                .items_center()
                .gap_1()
                .ml_2()
                .text_xs()
                .text_color(theme.warning)
                .child(icons::duplicate().size_4())
                .child("Mogelijk dubbel")
        });
        let inspect = h_flex()
            .gap_1()
            .items_center()
            .flex_wrap()
            .child(row.listen_button(("listen", id as u64), cx))
            .child(english_btn)
            .when(row.snippet_id.is_some(), |d| d.child(source_btn))
            .children(duplicate);
        let decide = h_flex()
            .gap_2()
            .items_center()
            .child(icon_button(("discard", id as u64), icons::trash(), "Weggooien").on_click(cx.listener(move |this, _, _, cx| this.discard_draft(id, cx))))
            .child(
                Button::new(("accept", id as u64))
                    .primary()
                    .small()
                    .icon(icons::check())
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
            .border_color(theme.border)
            .justify_between()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(inspect)
            .child(decide)
    }
}
