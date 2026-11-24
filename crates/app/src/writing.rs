//! Writing the card: its Dutch turns into blanks in the panel and the learner
//! types it. Right letters flash green and settle; wrong ones turn red and stay.

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
use crate::ui::{explained, hint};

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

    /// One key while writing: a character to check, or Esc to stop.
    pub fn write_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if key == "escape" {
            return self.stop_writing(cx);
        }
        let mut chars = key.chars();
        let (Some(typed), None) = (chars.next(), chars.next()) else { return };
        let attempt = &mut self.study.attempt;
        let Some(writing) = attempt.writing.as_mut() else { return };
        writing.type_char(typed);
        if let Some(result) = writing.result() {
            attempt.written.get_or_insert(result);
            // In Luisteren, a written sentence has been shown.
            attempt.revealed = true;
        }
        cx.notify();
    }

    /// The card as blanks, words kept whole when the line wraps. The waiting
    /// letter's blank is in the accent colour.
    pub fn render_writing(&self, writing: &Writing, cx: &App) -> Div {
        let theme = cx.theme();
        let colors = Colors { text: theme.foreground, right: theme.success, wrong: theme.danger, blank: theme.muted_foreground.opacity(0.45), cursor: theme.primary };
        let mut words = Vec::new();
        let mut word = Vec::new();
        for (index, &(letter, slot)) in writing.cells().iter().enumerate() {
            if letter.is_whitespace() {
                words.push(std::mem::take(&mut word));
                continue;
            }
            let id = SharedString::from(format!("write-{}-{index}", self.study.write_round));
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

    /// How the finished writing went: its grade and the share right first time.
    pub fn render_written(&self, cx: &App) -> Option<Div> {
        let result = self.study.attempt.writing.as_ref()?.result()?;
        Some(
            h_flex()
                .gap_2()
                .items_center()
                .child(explained("written-grade", "Schrijven").flex().items_center().gap_2().text_color(cx.theme().muted_foreground).child(icons::write().size_4()).child(grade_chip(result.grade, cx)))
                .child(explained("written-accuracy", "Zoveel letters in één keer goed").child(hint(format!("{:.0}%", result.accuracy * 100.0), cx))),
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
    let hidden = gpui_kit::transparent_black();
    let line = match slot {
        Slot::Blank if at_cursor => Some(colors.cursor),
        Slot::Blank => Some(colors.blank),
        Slot::Missed => Some(colors.wrong),
        _ => None,
    };
    let base = div()
        .relative()
        .child(letter.to_string())
        .children(line.map(|color| div().absolute().bottom(px(2.)).left(px(1.5)).right(px(1.5)).h(px(2.)).rounded_full().bg(color)));
    match slot {
        Slot::Given => base.text_color(colors.text).into_any_element(),
        Slot::Blank | Slot::Missed => base.text_color(hidden).into_any_element(),
        Slot::Wrong => base.text_color(colors.wrong).into_any_element(),
        // Green for a moment, then back to the text colour.
        Slot::Right => base
            .text_color(colors.right)
            .with_animation(id, Animation::new(SETTLE), move |cell, t| cell.text_color(colors.right.mix_oklab(colors.text, 1. - ((t - 0.35) / 0.65).clamp(0., 1.))))
            .into_any_element(),
    }
}
