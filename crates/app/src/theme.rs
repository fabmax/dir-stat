//! Color tokens, the type scale and style functions (UX §11).
//!
//! `App::theme` returns `None`, so iced picks the built-in Light or Dark theme and follows the
//! OS live. The built-in palettes differ from our tokens, so every style reads `tokens(theme)`.
//!
//! The tokens and the type scale are the complete UX §11 set; later views use the rest.
#![allow(dead_code)]

use iced::font::Font;
use iced::widget::{button, container, progress_bar, scrollable, text};
use iced::{Background, Border, Color, Theme, color};

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

pub fn text_primary(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).text_primary),
    }
}

pub fn text_accent(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).accent),
    }
}

pub fn text_accent_alpha(theme: &Theme, alpha: f32) -> text::Style {
    text::Style {
        color: Some(tokens(theme).accent.scale_alpha(alpha)),
    }
}

pub fn text_good(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).status_good),
    }
}

pub fn text_critical(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).status_critical),
    }
}

/// The tree and other panes.
pub fn surface(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_surface.into()),
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// A hairline rule between bars.
pub fn hairline(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(tokens(theme).border_hairline.into()),
        ..container::Style::default()
    }
}

pub fn activity_field(theme: &Theme) -> container::Style {
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

pub fn accent_fill(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(tokens(theme).accent.into()),
        ..container::Style::default()
    }
}

/// The error banner on the start screen.
pub fn banner(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.chip_critical.into()),
        border: Border {
            color: tokens.status_critical.scale_alpha(0.4),
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// "Drop to scan": a 2 px accent frame over the whole window.
pub fn drop_overlay(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.scrim.scale_alpha(0.35).into()),
        border: Border {
            color: tokens.accent,
            width: 2.0,
            radius: 0.0.into(),
        },
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

pub fn raised(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_raised.into()),
        border: Border {
            color: tokens.border_hairline,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// A start-screen location card: the whole card is the button (UX §4).
pub fn card_button(theme: &Theme, status: button::Status, focused: bool) -> button::Style {
    let tokens = tokens(theme);
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let border = if focused {
        Border {
            color: tokens.accent,
            width: 2.0,
            radius: 8.0.into(),
        }
    } else {
        Border {
            color: if hovered {
                tokens.border_strong
            } else {
                tokens.border_hairline
            },
            width: 1.0,
            radius: 8.0.into(),
        }
    };
    button::Style {
        background: Some(Background::Color(if hovered {
            tokens.bg_hover
        } else {
            tokens.bg_surface
        })),
        text_color: tokens.text_primary,
        border,
        ..button::Style::default()
    }
}

pub fn secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    let tokens = tokens(theme);
    let background = match status {
        button::Status::Hovered => tokens.bg_hover,
        button::Status::Pressed => tokens.bg_selected_unfocused,
        _ => tokens.bg_raised,
    };
    button::Style {
        background: Some(background.into()),
        text_color: if status == button::Status::Disabled {
            tokens.text_muted
        } else {
            tokens.text_primary
        },
        border: Border {
            color: tokens.border_strong,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    }
}

/// A text link ("Open Privacy Settings").
pub fn link_button(theme: &Theme, status: button::Status) -> button::Style {
    let tokens = tokens(theme);
    button::Style {
        background: None,
        text_color: if status == button::Status::Hovered {
            tokens.accent
        } else {
            tokens.accent_text
        },
        ..button::Style::default()
    }
}

/// The capacity meter on volume cards (UX §11.6).
pub fn meter(theme: &Theme, critical: bool) -> progress_bar::Style {
    let tokens = tokens(theme);
    progress_bar::Style {
        background: tokens.meter_track.into(),
        bar: if critical {
            tokens.status_critical
        } else {
            tokens.accent
        }
        .into(),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
    }
}

/// The column header above the tree: hairline below.
pub fn header(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_surface.into()),
        border: Border {
            color: tokens.border_hairline,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn placeholder_bar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(tokens(theme).bar_track.into()),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Tree rows keep their text tokens when selected (no inversion, bars stay visible).
pub fn tree_row(theme: &Theme, selected: bool) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: selected.then(|| tokens.bg_selected.into()),
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// Hover wash drawn over a row: ink 4% (light) / white 5% (dark), so text stays readable.
pub fn row_hover(theme: &Theme) -> container::Style {
    let wash = if theme.extended_palette().is_dark {
        Color::from_rgba(1.0, 1.0, 1.0, 0.05)
    } else {
        Color::from_rgba(0.043, 0.043, 0.043, 0.04)
    };
    container::Style {
        background: Some(wash.into()),
        ..container::Style::default()
    }
}

/// The share bar (UX §11.6): 6 px, radius 3, accent on `bar.track`.
pub fn share_bar(theme: &Theme) -> progress_bar::Style {
    let tokens = tokens(theme);
    progress_bar::Style {
        background: tokens.bar_track.into(),
        bar: tokens.accent.into(),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
    }
}

pub fn thin_scrollbar(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let tokens = tokens(theme);
    let mut style = scrollable::default(theme, status);
    let scroller = scrollable::Scroller {
        background: tokens.border_strong.into(),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
    };
    style.vertical_rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller,
    };
    style
}

pub fn text_warning(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).status_warning),
    }
}

/// The dark "!" on the filled amber badge.
pub fn text_on_warning(_theme: &Theme) -> text::Style {
    text::Style {
        color: Some(LIGHT.text_primary),
    }
}

/// Filled amber badge: the folder itself couldn't be read.
pub fn warning_badge(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(tokens(theme).status_warning.into()),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Outline badge: something inside couldn't be read.
pub fn warning_outline(theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: tokens(theme).status_warning,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

/// "Show 50 more" and other clickable text.
pub fn text_accent_link(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(tokens(theme).accent_text),
    }
}

/// The selection bar: window background below a hairline.
pub fn selection_bar(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_window.into()),
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// A toast: raised, radius 8, with the token shadow.
pub fn toast(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_raised.into()),
        border: Border {
            color: tokens.border_hairline,
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: iced::Shadow {
            color: tokens.shadow,
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// The × on a sticky error toast.
pub fn icon_button(theme: &Theme, status: button::Status) -> button::Style {
    let tokens = tokens(theme);
    button::Style {
        background: (status == button::Status::Hovered).then(|| tokens.bg_hover.into()),
        text_color: tokens.text_secondary,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// A list row that is a button (recent locations): hover background only.
pub fn row_button(theme: &Theme, status: button::Status) -> button::Style {
    let tokens = tokens(theme);
    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then(|| tokens.bg_hover.into()),
        text_color: tokens.text_primary,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Popovers: raised, radius 8, with shadow (UX §11.2).
pub fn popover(theme: &Theme) -> container::Style {
    toast(theme)
}

/// The Full Disk Access info bar at the bottom of the tree pane.
pub fn info_bar(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_raised.into()),
        border: Border {
            color: tokens.border_hairline,
            width: 1.0,
            radius: 0.0.into(),
        },
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// Dims the window behind a dialog.
pub fn scrim(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(tokens(theme).scrim.into()),
        ..container::Style::default()
    }
}

/// A dialog card: raised, radius 12, shadow.
pub fn dialog(theme: &Theme) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(tokens.bg_raised.into()),
        border: Border {
            color: tokens.border_hairline,
            width: 1.0,
            radius: 12.0.into(),
        },
        shadow: iced::Shadow {
            color: tokens.shadow,
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// Dialog buttons: the destructive one is filled critical with white text; the focused one
/// gets a 2 px accent ring.
pub fn dialog_button(
    theme: &Theme,
    status: button::Status,
    focused: bool,
    destructive: bool,
) -> button::Style {
    let tokens = tokens(theme);
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let (background, text_color) = if destructive {
        let fill = if hovered {
            tokens.status_critical.scale_alpha(0.85)
        } else {
            tokens.status_critical
        };
        (fill, Color::WHITE)
    } else {
        (
            if hovered {
                tokens.bg_hover
            } else {
                tokens.bg_raised
            },
            tokens.text_primary,
        )
    };
    let border = if focused {
        Border {
            color: tokens.accent,
            width: 2.0,
            radius: 6.0.into(),
        }
    } else {
        Border {
            color: if destructive {
                tokens.status_critical
            } else {
                tokens.border_strong
            },
            width: 1.0,
            radius: 6.0.into(),
        }
    };
    button::Style {
        background: Some(background.into()),
        text_color,
        border,
        ..button::Style::default()
    }
}

/// Tree rows: selected (focused or not), and the reveal flash.
pub fn tree_row_state(
    theme: &Theme,
    selected: bool,
    focused: bool,
    flashing: bool,
    hovered: bool,
) -> container::Style {
    let tokens = tokens(theme);
    let background = if flashing {
        Some(tokens.accent.scale_alpha(0.35))
    } else if selected && focused {
        Some(tokens.bg_selected)
    } else if selected {
        Some(tokens.bg_selected_unfocused)
    } else if hovered {
        Some(tokens.bg_hover)
    } else {
        None
    };
    container::Style {
        background: background.map(Into::into),
        text_color: Some(tokens.text_primary),
        ..container::Style::default()
    }
}

/// Cleanup pills: status color at 14% behind 11 px text (UX §6.2).
pub fn pill(theme: &Theme, safe: bool) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(
            if safe {
                tokens.chip_good
            } else {
                tokens.chip_warning
            }
            .into(),
        ),
        border: Border {
            radius: 9.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The 6 px dot in a pill: good for Safe, warning for Review first.
pub fn status_dot(theme: &Theme, safe: bool) -> container::Style {
    let tokens = tokens(theme);
    container::Style {
        background: Some(
            if safe {
                tokens.status_good
            } else {
                tokens.status_warning
            }
            .into(),
        ),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Treemap fills and their label colors (UX §11.4). `slot` is the top-level group (None =
/// "Other" gray); levels alternate base / 72% / base; "N files" blocks use a 52% blend.
pub fn map_fill(theme: &Theme, slot: Option<u8>, level: u8, files: bool) -> (Color, Color) {
    const INK: Color = color!(0x0b0b0b);
    let dark = theme.extended_palette().is_dark;
    // (base, base label, 72% blend, its label)
    let table: [(Color, Color, Color, Color); 4] = if dark {
        [
            (
                color!(0x3987e5),
                Color::WHITE,
                color!(0x3068ac),
                Color::WHITE,
            ),
            (
                color!(0xd95926),
                Color::WHITE,
                color!(0xa44722),
                Color::WHITE,
            ),
            (
                color!(0x199e70),
                Color::WHITE,
                color!(0x197958),
                Color::WHITE,
            ),
            (
                color!(0x5f5e59),
                Color::WHITE,
                color!(0x4c4b47),
                Color::WHITE,
            ),
        ]
    } else {
        [
            (
                color!(0x2a78d6),
                Color::WHITE,
                color!(0x659de0),
                Color::WHITE,
            ),
            (color!(0xeb6834), Color::WHITE, color!(0xf0916c), INK),
            (color!(0x1baf7a), Color::WHITE, color!(0x5ac59e), INK),
            (color!(0xa8a69e), INK, color!(0xc0beb8), INK),
        ]
    };
    let (base, base_label, mid, mid_label) = table[slot.map_or(3, |s| usize::from(s.min(2)))];
    if files {
        let fill = blend(base, tokens(theme).bg_surface, 0.52);
        let label = if luminance(fill) > 0.36 {
            INK
        } else {
            Color::WHITE
        };
        return (fill, label);
    }
    if level == 2 {
        (mid, mid_label)
    } else {
        (base, base_label)
    }
}

/// `over` at `alpha` on top of `under`.
pub fn blend(over: Color, under: Color, alpha: f32) -> Color {
    let mix = |a: f32, b: f32| a * alpha + b * (1.0 - alpha);
    Color::from_rgb(
        mix(over.r, under.r),
        mix(over.g, under.g),
        mix(over.b, under.b),
    )
}

/// Relative luminance (sRGB), to pick white or ink labels on computed fills.
fn luminance(c: Color) -> f32 {
    let lin = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// The "N files" fill in the UX table: light blue 52% ≈ #8fb7e8 (ink), dark ≈ #2a5383.
#[cfg(test)]
mod map_color_tests {
    use super::*;

    #[test]
    fn files_blend_matches_the_spec_table() {
        let light = Theme::Light;
        let (fill, label) = map_fill(&light, Some(0), 1, true);
        let hex = |c: Color| {
            let [r, g, b, _] = c.into_rgba8();
            (r, g, b)
        };
        let (r, g, b) = hex(fill);
        let (er, eg, eb) = (0x8f, 0xb7, 0xe8);
        assert!(
            r.abs_diff(er) <= 6 && g.abs_diff(eg) <= 6 && b.abs_diff(eb) <= 6,
            "{:?}",
            hex(fill)
        );
        assert_eq!(label, color!(0x0b0b0b));
        let (fill, label) = map_fill(&Theme::Dark, Some(0), 1, true);
        let (r, g, b) = hex(fill);
        assert!(
            r.abs_diff(0x2a) <= 6 && g.abs_diff(0x53) <= 6 && b.abs_diff(0x83) <= 6,
            "{:?}",
            hex(fill)
        );
        assert_eq!(label, Color::WHITE);
        assert_eq!(map_fill(&light, None, 2, false).0, color!(0xc0beb8));
        assert_eq!(
            map_fill(&light, Some(2), 3, false).0,
            color!(0x1baf7a),
            "level 3 = base"
        );
    }
}
