//! Small shared pieces of the interface.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{App, Div, FontWeight, ParentElement as _, SharedString, Styled as _, div, px};

/// A key hint such as `Spatie` or `V`.
pub fn keycap(label: &str, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1p5()
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_b_2()
        .border_color(theme.border)
        .bg(theme.background)
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label.to_string())
}

/// Muted helper text.
pub fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text.into())
}

/// A red recording dot and a bar showing the input loudness (0.0–1.0).
pub fn level_meter(level: f32, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .gap_3()
        .items_center()
        .child(div().size(px(10.)).rounded_full().bg(theme.danger))
        .child(
            div()
                .w(px(160.))
                .h(px(6.))
                .rounded_full()
                .bg(theme.secondary)
                .child(div().h_full().rounded_full().bg(theme.danger).w(px(160. * level.clamp(0.0, 1.0)))),
        )
}

/// A heading for a screen.
pub fn heading(text: impl Into<SharedString>) -> Div {
    div().text_lg().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

/// A titled part of a screen with its explanation; add the controls as children.
pub fn section(title: &str, explanation: impl Into<SharedString>, cx: &App) -> Div {
    v_flex().gap_3().max_w(px(640.)).child(heading(title.to_string())).child(hint(explanation, cx))
}

/// A centred message for empty screens, with optional detail.
pub fn empty_state(title: &str, detail: &str, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .py_12()
        .child(div().text_xl().font_weight(FontWeight::MEDIUM).child(title.to_string()))
        .child(hint(detail.to_string(), cx).max_w(px(460.)).text_center())
}
