//! Shared visual primitives and the app's Delft-blue light/dark palette.

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, Div, ElementId, FontWeight, Hsla, InteractiveElement as _, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, hsla, px,
};

fn hex(color: u32) -> Hsla {
    let r = ((color >> 16) & 0xff) as f32 / 255.0;
    let g = ((color >> 8) & 0xff) as f32 / 255.0;
    let b = (color & 0xff) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d == 0.0 {
        return hsla(0.0, 0.0, l, 1.0);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    hsla(h, s, l, 1.0)
}

/// Applies the app's calm Delft-blue palette on top of the active light or dark mode.
pub fn apply_palette(cx: &mut App) {
    Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        let c = &mut theme.colors;
        if dark {
            c.background = hex(0x0f1420);
            c.tab_bar = hex(0x151b2b);
            c.group_box = hex(0x171f30);
            c.popover = hex(0x171f30);
            c.secondary = hex(0x1f283d);
            c.secondary_hover = hex(0x28344f);
            c.secondary_active = hex(0x313f5e);
            c.secondary_foreground = hex(0xd7e0f2);
            c.muted = hex(0x1b2336);
            c.muted_foreground = hex(0x97a5c2);
            c.foreground = hex(0xe9edf7);
            c.border = hex(0x2a354d);
            c.input = hex(0x32405c);
            c.primary = hex(0x7ba0f4);
            c.primary_hover = hex(0x8fb0f7);
            c.primary_active = hex(0x6790f0);
            c.primary_foreground = hex(0x0d1320);
            c.button = hex(0x1f283d);
            c.button_hover = hex(0x28344f);
            c.button_active = hex(0x313f5e);
            c.button_foreground = hex(0xe9edf7);
            c.button_primary = c.primary;
            c.button_primary_hover = c.primary_hover;
            c.button_primary_active = c.primary_active;
            c.button_primary_foreground = c.primary_foreground;
            c.button_secondary = c.secondary;
            c.button_secondary_hover = c.secondary_hover;
            c.button_secondary_active = c.secondary_active;
            c.button_secondary_foreground = c.secondary_foreground;
            c.accent = hex(0x1d2b4d);
            c.accent_foreground = hex(0xc5d7fb);
            c.ring = hex(0x7ba0f4);
            c.slider_bar = hex(0x7ba0f4);
            c.slider_thumb = hex(0xe9edf7);
            c.tab_foreground = hex(0x97a5c2);
            c.tab_active_foreground = hex(0xe9edf7);
            c.danger = hex(0xef7d75);
            c.danger_hover = hex(0xf3918a);
            c.danger_active = hex(0xe46860);
            c.danger_foreground = hex(0x190909);
            c.button_danger = c.danger;
            c.button_danger_hover = c.danger_hover;
            c.button_danger_active = c.danger_active;
            c.button_danger_foreground = c.danger_foreground;
            c.warning = hex(0xe4b24e);
            c.warning_hover = hex(0xebbf68);
            c.warning_active = hex(0xd5a13b);
            c.warning_foreground = hex(0x1b1305);
            c.blue = hex(0x7ba0f4);
            c.success = hex(0x62c994);
            c.success_hover = hex(0x77d4a4);
            c.success_active = hex(0x4db981);
            c.success_foreground = hex(0x071a10);
        } else {
            c.background = hex(0xf4f6fb);
            c.tab_bar = hex(0xffffff);
            c.group_box = hex(0xffffff);
            c.popover = hex(0xffffff);
            c.secondary = hex(0xe9eef7);
            c.secondary_hover = hex(0xdde5f2);
            c.secondary_active = hex(0xd0dbed);
            c.secondary_foreground = hex(0x1e2a42);
            c.muted = hex(0xeef2f8);
            c.muted_foreground = hex(0x55617a);
            c.foreground = hex(0x121a2c);
            c.border = hex(0xd4dce9);
            c.input = hex(0xc8d3e3);
            c.primary = hex(0x1d3f9e);
            c.primary_hover = hex(0x183587);
            c.primary_active = hex(0x142c72);
            c.primary_foreground = hex(0xffffff);
            c.button = hex(0xffffff);
            c.button_hover = hex(0xf1f4fa);
            c.button_active = hex(0xe5ebf5);
            c.button_foreground = hex(0x121a2c);
            c.button_primary = c.primary;
            c.button_primary_hover = c.primary_hover;
            c.button_primary_active = c.primary_active;
            c.button_primary_foreground = c.primary_foreground;
            c.button_secondary = c.secondary;
            c.button_secondary_hover = c.secondary_hover;
            c.button_secondary_active = c.secondary_active;
            c.button_secondary_foreground = c.secondary_foreground;
            c.accent = hex(0xe2e9fa);
            c.accent_foreground = hex(0x16327e);
            c.ring = hex(0x1d3f9e);
            c.slider_bar = hex(0x1d3f9e);
            c.slider_thumb = hex(0xffffff);
            c.tab_foreground = hex(0x55617a);
            c.tab_active_foreground = hex(0x121a2c);
            c.danger = hex(0xb8322c);
            c.danger_hover = hex(0xa12924);
            c.danger_active = hex(0x8c221d);
            c.danger_foreground = hex(0xffffff);
            c.button_danger = c.danger;
            c.button_danger_hover = c.danger_hover;
            c.button_danger_active = c.danger_active;
            c.button_danger_foreground = c.danger_foreground;
            c.warning = hex(0xa6630c);
            c.warning_hover = hex(0x915609);
            c.warning_active = hex(0x7c4908);
            c.warning_foreground = hex(0xffffff);
            c.blue = hex(0x1d3f9e);
            c.success = hex(0x1f7a4d);
            c.success_hover = hex(0x196740);
            c.success_active = hex(0x145534);
            c.success_foreground = hex(0xffffff);
        }
        theme.radius = px(7.);
        theme.radius_lg = px(12.);
    });
}

/// Elevated card/panel surface distinct from the app canvas.
pub fn surface(cx: &App) -> Hsla {
    cx.theme().group_box
}

/// An elevated surface card with a crisp border.
pub fn card_box(cx: &App) -> Div {
    let theme = cx.theme();
    v_flex().rounded(px(12.)).border_1().border_color(theme.border).bg(surface(cx))
}

/// A compact semantic badge/pill.
pub fn pill(text: impl Into<SharedString>, bg: Hsla, fg: Hsla) -> Div {
    div()
        .px_2()
        .py(px(1.5))
        .rounded_full()
        .bg(bg)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .child(text.into())
}

/// Container for a connected segmented control.
pub fn segmented_bar(cx: &App) -> Div {
    let theme = cx.theme();
    let track = if theme.is_dark() { theme.background } else { theme.secondary };
    h_flex()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.border)
        .bg(track)
}

/// One clickable option inside [`segmented_bar`].
pub fn segment_button(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool, cx: &App) -> Button {
    let theme = cx.theme();
    if active {
        let fill = if theme.is_dark() { theme.accent } else { surface(cx) };
        let border = if theme.is_dark() { theme.primary.opacity(0.45) } else { theme.border };
        let fg = if theme.is_dark() { theme.accent_foreground } else { theme.primary };
        let style = ButtonCustomVariant::new(cx)
            .color(fill)
            .foreground(fg)
            .hover(fill)
            .active(fill);
        Button::new(id)
            .small()
            .custom(style)
            .bg(fill)
            .border_1()
            .border_color(border)
            .font_weight(FontWeight::SEMIBOLD)
            .label(label)
    } else {
        Button::new(id)
            .small()
            .ghost()
            .text_color(theme.muted_foreground)
            .label(label)
    }
}

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
        .bg(surface(cx))
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label.to_string())
}

/// A key hint rendered inside a primary filled button.
pub fn keycap_primary(label: &str, cx: &App) -> Div {
    let fg = cx.theme().primary_foreground;
    div()
        .px_1p5()
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_b_2()
        .border_color(fg.opacity(0.32))
        .bg(fg.opacity(0.14))
        .text_xs()
        .text_color(fg)
        .child(label.to_string())
}

/// Muted helper text.
pub fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text.into())
}

/// A recording indicator and loudness-zone bar (0.0–1.0): muted when quiet,
/// green in the clear range, red near clipping.
pub fn level_meter(level: f32, cx: &App) -> Div {
    let theme = cx.theme();
    let clamped = level.clamp(0.0, 1.0);
    let fill = if clamped >= 0.88 {
        theme.danger
    } else if clamped >= 0.20 {
        theme.success
    } else {
        theme.muted_foreground
    };
    h_flex()
        .gap_3()
        .items_center()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.))
                .rounded_full()
                .bg(theme.danger.opacity(0.18))
                .child(div().size(px(8.)).rounded_full().bg(theme.danger)),
        )
        .child(
            div()
                .w(px(176.))
                .h(px(8.))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .overflow_hidden()
                .child(div().h_full().rounded_full().bg(fill).w(px(176. * clamped))),
        )
}

/// A heading for a screen.
pub fn heading(text: impl Into<SharedString>) -> Div {
    div().text_lg().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

/// A heading with its icon in front.
pub fn titled(icon: Icon, text: impl Into<SharedString>, cx: &App) -> Div {
    h_flex().items_center().gap_2().child(icon.size_5().text_color(cx.theme().primary)).child(heading(text))
}

/// A section card under its icon and title; add the controls as children.
pub fn section(icon: Icon, title: &str, cx: &App) -> Div {
    card_box(cx).p_4().gap_3().child(titled(icon, title.to_string(), cx))
}

/// A small icon-only button that names itself on hover.
pub fn icon_button(id: impl Into<ElementId>, icon: Icon, tip: impl Into<SharedString>) -> Button {
    // A custom size draws the icon at three quarters of it.
    Button::new(id).ghost().with_size(px(26.)).tab_stop(false).icon(icon).tooltip(tip)
}

/// Something that says what it is on hover, such as an icon or a count.
pub fn explained(id: impl Into<ElementId>, tip: impl Into<SharedString>) -> Stateful<Div> {
    let tip = tip.into();
    div().id(id).tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
}

/// A muted icon and a number, with what it counts on hover.
pub fn stat(id: impl Into<ElementId>, icon: Icon, value: impl Into<SharedString>, tip: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    explained(id, tip)
        .flex()
        .items_center()
        .gap_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(icon.size_4())
        .child(value.into())
}

/// A centred picture and message for empty screens, with optional detail.
pub fn empty_state(icon: Icon, title: &str, detail: &str, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .items_center()
        .gap_2()
        .pt_8()
        .pb_4()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(64.))
                .mb_2()
                .rounded_full()
                .bg(theme.accent)
                .text_color(theme.accent_foreground)
                .child(icon.size(px(32.))),
        )
        .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .when(!detail.is_empty(), |d| d.child(hint(detail.to_string(), cx).max_w(px(420.)).text_center()))
}
