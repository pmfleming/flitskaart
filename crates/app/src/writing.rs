//! Writing the card: its Dutch turns into blanks in the panel and the learner
//! types it. Every key fills the next blank. A right letter flashes green and
//! settles; a wrong one shows in red. Backspace goes back to fix it, and Enter
//! finishes, showing the missed letters corrected in red.

use std::time::Duration;

use dutch_flashcards_core::writing::{Slot, Writing};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, h_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, Context, Div, FontWeight, Hsla, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px, relative,
};

use crate::app::FlashcardsApp;
use crate::icons;
use crate::speaking::grade_chip;
use crate::ui::{explained, hint, keycap};

/// How long a right letter takes to go from green back to the text colour.
const SETTLE: Duration = Duration::from_millis(900);

impl FlashcardsApp {
    /// Whether the learner is in the middle of writing the card.
    pub fn writing_now(&self) -> bool {
        self.study.attempt.writing.as_ref().is_some_and(|writing| !writing.is_done())
    }

    pub fn start_writing(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.study.card.as_ref().filter(|_| !self.busy_listening()) else { return };
        self.study.write_round += 1;
        self.study.attempt.writing = Some(Writing::new(&card.dutch));
        cx.notify();
    }

    pub fn stop_writing(&mut self, cx: &mut Context<Self>) {
        self.study.attempt.writing = None;
        cx.notify();
    }

    /// Ends the writing as it stands, showing what was missed.
    pub fn finish_writing(&mut self, cx: &mut Context<Self>) {
        if let Some(writing) = self.study.attempt.writing.as_mut() {
            writing.finish();
        }
        self.after_writing(cx);
    }

    /// One key while writing: a character, Backspace, Enter to finish, Esc to stop.
    pub fn write_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(writing) = self.study.attempt.writing.as_mut() else { return };
        match key {
            "escape" => return self.stop_writing(cx),
            "enter" => return self.finish_writing(cx),
            "backspace" => _ = writing.back(),
            _ => {
                let mut chars = key.chars();
                if let (Some(typed), None) = (chars.next(), chars.next()) {
                    writing.type_char(typed);
                }
            }
        }
        self.after_writing(cx);
    }

    /// Keeps the first finished result; in Luisteren, a written sentence has been shown.
    fn after_writing(&mut self, cx: &mut Context<Self>) {
        let attempt = &mut self.study.attempt;
        if let Some(result) = attempt.writing.as_ref().and_then(Writing::result) {
            attempt.written.get_or_insert(result);
            attempt.revealed = true;
        }
        cx.notify();
    }

    /// The card as blanks, words kept whole when the line wraps. The blank
    /// the next key fills is in the accent colour.
    pub fn render_writing(&self, writing: &Writing, cx: &App) -> Div {
        let theme = cx.theme();
        let colors = Colors { text: theme.foreground, right: theme.success, wrong: theme.danger, blank: theme.muted_foreground.opacity(0.45), cursor: theme.primary };
        let mut words = Vec::new();
        let mut word = Vec::new();
        for (index, (letter, slot)) in writing.cells().enumerate() {
            if letter.is_whitespace() {
                words.push(std::mem::take(&mut word));
                continue;
            }
            let id = SharedString::from(format!("write-{}-{index}-{}", self.study.write_round, writing.typed_count(index)));
            word.push(cell(letter, slot, writing.cursor() == Some(index), id, colors));
        }
        words.push(word);
        div()
            .flex()
            .flex_wrap()
            .justify_center()
            .max_w(px(620.))
            .gap_x(px(12.))
            .gap_y_1()
            .text_size(px(32.))
            .line_height(relative(1.3))
            .font_weight(FontWeight::MEDIUM)
            .children(words.into_iter().filter(|word| !word.is_empty()).map(|word| h_flex().children(word)))
    }

    /// Under the writing: how to fix or finish once every blank is filled,
    /// then the grade and the share right first time.
    pub fn render_written(&self, cx: &App) -> Option<Div> {
        let writing = self.study.attempt.writing.as_ref()?;
        if writing.is_full() {
            return Some(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(keycap("Backspace", cx))
                    .child(hint("verbeter", cx))
                    .child(hint("·", cx))
                    .child(keycap("Enter", cx))
                    .child(hint("toon de verbetering", cx)),
            );
        }
        let result = writing.result()?;
        Some(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    explained("written-grade", "Schrijven")
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(cx.theme().muted_foreground)
                        .child(icons::write().size_4())
                        .child(grade_chip(result.grade, cx)),
                )
                .child(explained("written-accuracy", "Zoveel letters goed; een verbeterde letter telt half").child(hint(format!("{:.0}%", result.accuracy * 100.0), cx))),
        )
    }
}

#[derive(Clone, Copy)]
struct Colors {
    text: Hsla,
    right: Hsla,
    wrong: Hsla,
    blank: Hsla,
    cursor: Hsla,
}

/// One character, as wide as its letter. A blank is a short line under where
/// the letter goes, inset so neighbouring blanks stay apart.
fn cell(letter: char, slot: Slot, at_cursor: bool, id: SharedString, colors: Colors) -> AnyElement {
    let line = match slot {
        Slot::Blank if at_cursor => Some(colors.cursor),
        Slot::Blank => Some(colors.blank),
        // A fixed letter keeps a faint mark of the mistake.
        Slot::Fixed => Some(colors.wrong.opacity(0.5)),
        _ => None,
    };
    let base = div()
        .relative()
        .child(letter.to_string())
        .children(line.map(|color| div().absolute().bottom(px(2.)).left(px(1.5)).right(px(1.5)).h(px(2.)).rounded_full().bg(color)));
    match slot {
        Slot::Given => base.text_color(colors.text).into_any_element(),
        Slot::Blank => base.text_color(gpui_kit::transparent_black()).into_any_element(),
        // The cell keeps the width of the right letter, so a wrong one of
        // another width never shifts the words or rewraps the line.
        Slot::Wrong(typed) if typed != letter => base
            .text_color(gpui_kit::transparent_black())
            .child(div().absolute().inset_0().flex().justify_center().whitespace_nowrap().text_color(colors.wrong).child(typed.to_string()))
            .into_any_element(),
        Slot::Wrong(_) | Slot::Shown => base.text_color(colors.wrong).into_any_element(),
        // Green for a moment, then back to the text colour.
        Slot::Right | Slot::Fixed => base
            .text_color(colors.right)
            .with_animation(id, Animation::new(SETTLE), move |cell, t| cell.text_color(colors.right.mix_oklab(colors.text, 1. - ((t - 0.35) / 0.65).clamp(0., 1.))))
            .into_any_element(),
    }
}
