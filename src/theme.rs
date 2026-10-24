//! App palette tokens and the apply step that paints them over gpui-component.
//!
//! `Theme::change` re-applies the current theme JSON and wipes color edits, so
//! every theme switch must land the palette after it and call `sync_base`.

use gpui::{App, Hsla, Window, rgb};
use gpui_component::{Theme as ThemeGlobal, ThemeColor, ThemeMode, ThemeTokens};

use crate::store::Theme;

pub fn theme_mode(theme: Theme) -> ThemeMode {
    if theme.is_dark() {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
}

/// Applies a color scheme, then repaints the app's own palette over it.
pub fn apply_theme(mode: ThemeMode, window: &mut Window, cx: &mut App) {
    ThemeGlobal::change(mode, Some(window), cx);
    let dark = mode.is_dark();
    let palette = if dark { DARK_PALETTE } else { LIGHT_PALETTE };
    let colors = &mut ThemeGlobal::global_mut(cx).colors;
    for (slot, hex) in colors_of(colors).into_iter().zip(palette) {
        *slot = hsl(hex);
    }
    // `Theme::change` also caches a legacy `ThemeTokens` snapshot, and buttons
    // read `tokens.button_primary` rather than `colors.primary`. Rebuild the
    // snapshot after writing the palette or primary buttons keep the upstream
    // black.
    let theme = ThemeGlobal::global_mut(cx);
    let tokens = ThemeTokens::from(&theme.colors);
    theme.tokens = tokens;
    ThemeGlobal::sync_base(cx);
    window.refresh();
}

/// Handles to the tokens this app paints with, in palette order.
///
/// `button_primary*` are separate component slots in gpui-component: they do
/// not follow `primary` once a theme JSON sets them, so a filled primary button
/// otherwise keeps the upstream black instead of the accent blue.
fn colors_of(colors: &mut ThemeColor) -> [&mut Hsla; PALETTE_LEN] {
    [
        &mut colors.background,
        &mut colors.foreground,
        &mut colors.border,
        &mut colors.input,
        &mut colors.primary,
        &mut colors.primary_foreground,
        &mut colors.primary_hover,
        &mut colors.primary_active,
        &mut colors.secondary,
        &mut colors.secondary_foreground,
        &mut colors.secondary_hover,
        &mut colors.secondary_active,
        &mut colors.accent,
        &mut colors.accent_foreground,
        &mut colors.muted,
        &mut colors.muted_foreground,
        &mut colors.selection,
        &mut colors.caret,
        &mut colors.popover,
        &mut colors.popover_foreground,
        &mut colors.sidebar,
        &mut colors.sidebar_foreground,
        &mut colors.sidebar_accent,
        &mut colors.sidebar_accent_foreground,
        &mut colors.sidebar_border,
        &mut colors.title_bar,
        &mut colors.title_bar_border,
        &mut colors.scrollbar_thumb,
        &mut colors.button_primary,
        &mut colors.button_primary_foreground,
        &mut colors.button_primary_hover,
        &mut colors.button_primary_active,
    ]
}

/// Number of tokens in [`colors_of`]; both palettes must stay this long.
const PALETTE_LEN: usize = 32;

fn hsl(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Tool-density palette (see DESIGN.md): cool blues, ink-like accent, no
/// DeepSeek website clone. Light: white paper, sidebar one step darker.
const LIGHT_PALETTE: [u32; PALETTE_LEN] = [
    0xffffff, // background
    0x1b1d22, // foreground
    0xe2e5ec, // border
    0xffffff, // input
    0x3b5bdb, // primary
    0xffffff, // primary_foreground
    0x3450c4, // primary_hover
    0x2c46ad, // primary_active
    0xeef1fb, // secondary (user bubble, cards)
    0x1b1d22, // secondary_foreground
    0xe4e9f7, // secondary_hover
    0xd8e0f2, // secondary_active
    0xe8edfb, // accent (selected rows)
    0x1b1d22, // accent_foreground
    0xf2f5fd, // muted
    0x6b7280, // muted_foreground
    0xd6e0ff, // selection
    0x3b5bdb, // caret
    0xffffff, // popover
    0x1b1d22, // popover_foreground
    0xf7f8fa, // sidebar
    0x1b1d22, // sidebar_foreground
    0xe8edfb, // sidebar_accent
    0x1b1d22, // sidebar_accent_foreground
    0xe2e5ec, // sidebar_border
    0xf7f8fa, // title_bar
    0xe2e5ec, // title_bar_border
    0xc9ceda, // scrollbar_thumb
    0x3b5bdb, // button_primary
    0xffffff, // button_primary_foreground
    0x3450c4, // button_primary_hover
    0x2c46ad, // button_primary_active
];

const DARK_PALETTE: [u32; PALETTE_LEN] = [
    0x17181c, // background
    0xe7e9ee, // foreground
    0x2a2d36, // border
    0x1e2026, // input
    0x6b85ff, // primary
    0xffffff, // primary_foreground
    0x7a91ff, // primary_hover
    0x889dff, // primary_active
    0x232838, // secondary
    0xe7e9ee, // secondary_foreground
    0x2a3046, // secondary_hover
    0x303752, // secondary_active
    0x2b3350, // accent
    0xe7e9ee, // accent_foreground
    0x21232a, // muted
    0x9aa1ae, // muted_foreground
    0x33406b, // selection
    0x6b85ff, // caret
    0x1e2026, // popover
    0xe7e9ee, // popover_foreground
    0x121316, // sidebar
    0xe7e9ee, // sidebar_foreground
    0x262c40, // sidebar_accent
    0xe7e9ee, // sidebar_accent_foreground
    0x23262e, // sidebar_border
    0x121316, // title_bar
    0x23262e, // title_bar_border
    0x3a3f4d, // scrollbar_thumb
    0x6b85ff, // button_primary
    0xffffff, // button_primary_foreground
    0x7a91ff, // button_primary_hover
    0x889dff, // button_primary_active
];

#[cfg(test)]
mod tests {
    use super::*;

    /// `zip` would silently stop at the shorter side, so a palette that lost a
    /// token would leave that slot at the upstream theme's color.
    #[test]
    fn palettes_cover_every_slot() {
        assert_eq!(LIGHT_PALETTE.len(), PALETTE_LEN);
        assert_eq!(DARK_PALETTE.len(), PALETTE_LEN);
    }
}
