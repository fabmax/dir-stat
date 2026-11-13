//! Color tokens, the type scale and style functions (UX §11).
//!
//! `App::theme` returns `None`, so iced picks the built-in Light or Dark theme and follows the
//! OS live. The built-in palettes differ from our tokens, so every style reads `tokens(theme)`.

use iced::font::Font;
use iced::widget::{container, text};
use iced::{Border, Color, Theme, color};

use crate::fonts;

pub struct Tokens {
    pub bg_window: Color,
    pub bg_surface: Color,
    pub bg_raised: Color,
    pub bg_hover: Color,
    pub bg_selected: Color,
    pub bg_selected_unfocused: Color,
    pub border_hairline: Color,
    pub border_strong: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub accent_text: Color,
    pub accent_fill: Color,
    pub accent_fill_hover: Color,
    pub accent_fill_pressed: Color,
    pub bar_track: Color,
    pub meter_track: Color,
    pub status_good: Color,
    pub status_warning: Color,
    pub status_critical: Color,
    pub chip_good: Color,
    pub chip_warning: Color,
    pub chip_critical: Color,
    pub scrim: Color,
    /// Drawn with y offset 8 and blur 24.
    pub shadow: Color,
}

pub const LIGHT: Tokens = Tokens {
    bg_window: color!(0xf4f4f1),
    bg_surface: color!(0xfcfcfb),
    bg_raised: color!(0xffffff),
    bg_hover: color!(0xf2f2f1),
    bg_selected: color!(0xdfeaf6),
    bg_selected_unfocused: color!(0xebebea),
    border_hairline: color!(0xe1e0d9),
    border_strong: color!(0xc3c2b7),
    text_primary: color!(0x0b0b0b),
    text_secondary: color!(0x52514e),
    text_muted: color!(0x898781),
    accent: color!(0x2a78d6),
    accent_text: color!(0x256abf),
    accent_fill: color!(0x256abf),
    accent_fill_hover: color!(0x1c5cab),
    accent_fill_pressed: color!(0x184f95),
    bar_track: color!(0xebebea),
    meter_track: color!(0xe6e5df),
    status_good: color!(0x0ca30c),
    status_warning: color!(0xfab219),
    status_critical: color!(0xd03b3b),
    chip_good: color!(0xdaf0da),
    chip_warning: color!(0xfcf2db),
    chip_critical: color!(0xf6e1e0),
    scrim: color!(0x0b0b0b, 0.32),
    shadow: color!(0x0b0b0b, 0.14),
};

pub const DARK: Tokens = Tokens {
    bg_window: color!(0x141413),
    bg_surface: color!(0x1a1a19),
    bg_raised: color!(0x232321),
    bg_hover: color!(0x252525),
    bg_selected: color!(0x233b56),
    bg_selected_unfocused: color!(0x2f2f2e),
    border_hairline: color!(0x2c2c2a),
    border_strong: color!(0x383835),
    text_primary: color!(0xf0efec),
    text_secondary: color!(0xc3c2b7),
    text_muted: color!(0x898781),
    accent: color!(0x3987e5),
    accent_text: color!(0x6da7ec),
    accent_fill: color!(0x256abf),
    accent_fill_hover: color!(0x1c5cab),
    accent_fill_pressed: color!(0x184f95),
    bar_track: color!(0x2c2c2b),
    meter_track: color!(0x2c2c2a),
    status_good: color!(0x0ca30c),
    status_warning: color!(0xfab219),
    status_critical: color!(0xd03b3b),
    chip_good: color!(0x182d17),
    chip_warning: color!(0x392f19),
    chip_critical: color!(0x331f1e),
    scrim: color!(0x000000, 0.50),
    shadow: color!(0x000000, 0.50),
};

impl Tokens {
    /// Every token with its UX §11.2 name, for the specimen screen.
    pub fn named(&self) -> [(&'static str, Color); 26] {
        [
            ("bg.window", self.bg_window),
            ("bg.surface", self.bg_surface),
            ("bg.raised", self.bg_raised),
            ("bg.hover", self.bg_hover),
            ("bg.selected", self.bg_selected),
            ("bg.selected.unfocused", self.bg_selected_unfocused),
            ("border.hairline", self.border_hairline),
            ("border.strong", self.border_strong),
            ("text.primary", self.text_primary),
            ("text.secondary", self.text_secondary),
            ("text.muted", self.text_muted),
            ("accent", self.accent),
            ("accent.text", self.accent_text),
            ("accent.fill", self.accent_fill),
            ("accent.fill.hover", self.accent_fill_hover),
            ("accent.fill.pressed", self.accent_fill_pressed),
            ("bar.track", self.bar_track),
            ("meter.track", self.meter_track),
            ("status.good", self.status_good),
            ("status.warning", self.status_warning),
            ("status.critical", self.status_critical),
            ("chip.good", self.chip_good),
            ("chip.warning", self.chip_warning),
            ("chip.critical", self.chip_critical),
            ("scrim", self.scrim),
            ("shadow", self.shadow),
        ]
    }
}

pub fn tokens(theme: &Theme) -> &'static Tokens {
    if theme.extended_palette().is_dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// Window background and default text color, set on the application.
pub fn app_style<State>(_state: &State, theme: &Theme) -> iced::theme::Style {
    let tokens = tokens(theme);
    iced::theme::Style {
        background_color: tokens.bg_window,
        text_color: tokens.text_primary,
    }
}

/// One step of the type scale: size / line height in logical px, and the face.
#[derive(Debug, Clone, Copy)]
pub struct TextRole {
    pub size: f32,
    pub line_height: f32,
    pub font: Font,
}

impl TextRole {
    /// A `text` widget in this role. Symbols Inter lacks must go in their own Regular `text`
    /// (cosmic-text only falls back to faces of the exact weight).
    pub fn text<'a>(self, content: impl text::IntoFragment<'a>) -> text::Text<'a> {
        text(content)
            .size(self.size)
            .line_height(text::LineHeight::Absolute(self.line_height.into()))
            .font(self.font)
    }

    pub const fn with_font(self, font: Font) -> Self {
        Self { font, ..self }
    }
}

const fn role(size: f32, line_height: f32, font: Font) -> TextRole {
    TextRole {
        size,
        line_height,
        font,
    }
}

pub const DISPLAY: TextRole = role(24.0, 30.0, fonts::SEMIBOLD);
pub const TITLE: TextRole = role(15.0, 20.0, fonts::SEMIBOLD);
pub const STAT: TextRole = role(28.0, 34.0, fonts::SEMIBOLD);
pub const BODY: TextRole = role(13.0, 18.0, fonts::INTER);
pub const BODY_STRONG: TextRole = role(13.0, 18.0, fonts::MEDIUM);
pub const SMALL: TextRole = role(12.0, 16.0, fonts::INTER);
pub const CAPTION: TextRole = role(11.0, 14.0, fonts::MEDIUM);

pub fn text_secondary(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_secondary),
    }
}

pub fn text_muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_muted),
    }
}

/// A surface panel with a hairline border (radius 8, UX §11.5).
pub fn card(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_surface.into()),
        border: Border {
            color: tokens.border_hairline,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}
