//! Concepten: check drafted cards before they enter the deck.

use dutch_flashcards_core::{Card, CardId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Div, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

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
        let mut field = |value: &str, placeholder: &'static str| {
            let value = value.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder))
        };
        DraftRow {
            id: card.id,
            dutch: field(&card.dutch, "Nederlands"),
            english: field(&card.english, "Vertaling (verborgen bij het oefenen)"),
            note: field(&card.note, "Uitleg in eenvoudig Nederlands (optioneel)"),
            tags: field(&card.tags, "Onderwerp"),
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

    /// The Dutch and the note, and the translation when `english` is on.
    pub fn fields(&self, english: bool) -> Div {
        v_flex()
            .gap_2()
            .child(Input::new(&self.dutch).large())
            .child(Input::new(&self.note))
            .when(english, |d| d.child(Input::new(&self.english)))
    }

    /// Reads the Dutch as typed so far.
    pub fn listen_button(&self, id: impl Into<ElementId>, cx: &mut Context<FlashcardsApp>) -> Button {
        let dutch = self.dutch.clone();
        Button::new(id).ghost().small().label("Luister").on_click(cx.listener(move |this, _, _, cx| {
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
            let empty = empty_state("Geen concepten", "Plak een stuk van een gesprek bij Toevoegen om kaarten te maken.", cx);
            return v_flex().size_full().items_center().justify_center().child(empty).into_any_element();
        }
        let rows: Vec<_> = self.drafts.iter().map(|row| self.render_draft_row(row, cx)).collect();
        let title = v_flex()
            .gap_1()
            .child(heading(format!("{} concepten", self.drafts.len())))
            .child(hint("Controleer elke kaart voordat hij in je stapel komt.", cx));
        let accept_all = Button::new("accept-all").primary().label("Alles bewaren").on_click(cx.listener(|this, _, _, cx| this.accept_all(cx)));
        v_flex()
            .size_full()
            .p_5()
            .gap_3()
            .child(h_flex().justify_between().gap_3().child(title).child(accept_all))
            .child(v_flex().id("draft-list").flex_1().min_h_0().overflow_y_scroll().gap_3().pb_4().children(rows))
            .into_any_element()
    }

    fn render_draft_row(&self, row: &DraftRow, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let source = row.snippet_id.filter(|_| row.show_source).and_then(|id| self.store.snippet(id).ok().flatten());
        let tags = h_flex()
            .gap_2()
            .child(div().text_sm().text_color(theme.muted_foreground).child("Onderwerp"))
            .child(div().w(px(220.)).child(Input::new(&row.tags).small()));
        v_flex()
            .p_3()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(row.fields(row.show_english))
            .child(tags)
            .when(row.duplicate, |d| d.child(div().text_sm().text_color(theme.warning).child("Deze zin staat al in je stapel of bij de concepten.")))
            .when_some(source, |d, text| d.child(div().p_2().rounded(px(6.)).bg(theme.background).text_sm().text_color(theme.muted_foreground).child(text)))
            .child(self.render_draft_buttons(row, cx))
    }

    /// Bewaar, Weggooien, Luister, and the translation and source switches.
    fn render_draft_buttons(&self, row: &DraftRow, cx: &mut Context<Self>) -> impl IntoElement {
        let id = row.id;
        let switch = |name: &'static str, label: &'static str, flip: fn(&mut DraftRow) -> &mut bool, cx: &mut Context<Self>| {
            Button::new((name, id as u64)).ghost().small().label(label).on_click(cx.listener(move |this, _, _, cx| this.toggle_draft(id, flip, cx)))
        };
        let english = if row.show_english { "Verberg vertaling" } else { "Vertaling" };
        let source = if row.show_source { "Verberg bron" } else { "Bron" };
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(Button::new(("accept", id as u64)).primary().small().label("Bewaar").on_click(cx.listener(move |this, _, _, cx| {
                if this.accept_draft(id, cx) {
                    this.after_deck_change(cx);
                }
            })))
            .child(Button::new(("discard", id as u64)).outline().small().label("Weggooien").on_click(cx.listener(move |this, _, _, cx| this.discard_draft(id, cx))))
            .child(row.listen_button(("listen", id as u64), cx))
            .child(switch("english", english, |row| &mut row.show_english, cx))
            .when(row.snippet_id.is_some(), |d| d.child(switch("source", source, |row| &mut row.show_source, cx)))
    }
}
