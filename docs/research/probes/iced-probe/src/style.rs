//! Theming probe: custom palettes + style closures that read from the active Theme.
use iced::border::{self, Border};
use iced::theme::{Palette, palette};
use iced::widget::{button, container, progress_bar, scrollable, text, text_input};
use iced::{Background, Color, Shadow, Theme, Vector, color};

/// Which theme the user wants. `System` => the theme fn returns `None` and iced follows
/// the OS light/dark setting live (built-in Theme::Light / Theme::Dark).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
    Nord,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [
        ThemeChoice::System,
        ThemeChoice::Light,
        ThemeChoice::Dark,
        ThemeChoice::Nord,
    ];

    pub fn theme(self) -> Option<Theme> {
        match self {
            ThemeChoice::System => None,
            ThemeChoice::Light => Some(light()),
            ThemeChoice::Dark => Some(dark()),
            ThemeChoice::Nord => Some(Theme::Nord),
        }
    }
}

impl std::fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Nord => "Nord",
        })
    }
}

/// Theme::custom(name, Palette { .. }) -> the Extended palette is generated automatically.
pub fn dark() -> Theme {
    Theme::custom(
        "dir-stat Dark",
        Palette {
            background: color!(0x1e1f22),
            text: color!(0xe3e5e8),
            primary: color!(0x4c8dff),
            success: color!(0x3fb950),
            warning: color!(0xd29922),
            danger: color!(0xf85149),
        },
    )
}

pub fn light() -> Theme {
    Theme::custom(
        "dir-stat Light",
        Palette {
            background: color!(0xfbfbfc),
            text: color!(0x1f2328),
            primary: color!(0x0a66d8),
            success: color!(0x1a7f37),
            warning: color!(0x9a6700),
            danger: color!(0xcf222e),
        },
    )
}

/// Panel / card container.
pub fn card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.weakest.color.into()),
        text_color: Some(palette.background.weakest.text),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.08),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 4.0,
        },
        ..container::Style::default()
    }
}

/// Header bar.
pub fn header(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style::default()
        .background(palette.background.weak.color)
        .color(palette.background.weak.text)
}

/// Modal backdrop (semi transparent).
pub fn backdrop(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color { a: 0.55, ..Color::BLACK }.into()),
        ..container::Style::default()
    }
}

/// Row style: a closure factory capturing per-row state (selected / zebra).
pub fn tree_row(
    selected: bool,
    odd: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();

        let base = if selected {
            palette.primary.weak
        } else if odd {
            palette.background.weakest
        } else {
            palette.background.base
        };

        let background = match status {
            button::Status::Hovered if !selected => palette.background.weak.color,
            button::Status::Pressed => palette.background.strong.color,
            _ => base.color,
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color: base.text,
            border: border::rounded(4),
            ..button::Style::default()
        }
    }
}

/// Container background for a tree row (selected / zebra striping).
pub fn tree_row_background(selected: bool, odd: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let pair = if selected {
            palette.primary.weak
        } else if odd {
            palette.background.weakest
        } else {
            palette.background.base
        };

        container::Style {
            background: Some(pair.color.into()),
            text_color: Some(pair.text),
            border: border::rounded(4),
            ..container::Style::default()
        }
    }
}

/// Translucent overlay shown by `hover()` on the hovered row.
pub fn row_hover_overlay(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.primary.base.color.scale_alpha(0.10).into()),
        border: border::rounded(4),
        ..container::Style::default()
    }
}

/// Secondary text, derived from the palette.
pub fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().background.strongest.color),
    }
}

/// Size bar (inline share-of-parent bar) built with progress_bar + custom style.
pub fn share_bar(theme: &Theme) -> progress_bar::Style {
    let palette = theme.extended_palette();
    progress_bar::Style {
        background: palette.background.weak.color.into(),
        bar: palette.primary.base.color.into(),
        border: border::rounded(2),
    }
}

/// Thin, rounded scrollbar.
pub fn thin_scrollbar(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut style = scrollable::default(theme, status);
    let palette = theme.extended_palette();

    let rail = scrollable::Rail {
        background: None,
        border: border::rounded(4),
        scroller: scrollable::Scroller {
            background: palette.background.strong.color.into(),
            border: border::rounded(4),
        },
    };

    style.vertical_rail = match status {
        scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered: true,
            ..
        }
        | scrollable::Status::Dragged {
            is_vertical_scrollbar_dragged: true,
            ..
        } => scrollable::Rail {
            scroller: scrollable::Scroller {
                background: palette.primary.base.color.into(),
                ..rail.scroller
            },
            ..rail
        },
        _ => rail,
    };
    style
}

/// text_input style tweak on top of the default.
pub fn search_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    style.border = style.border.rounded(6);
    style
}

/// Colour for a treemap tile, cycling through palette-derived hues.
pub fn tile_color(theme: &Theme, index: usize) -> Color {
    let extended = theme.extended_palette();
    let bases = [
        extended.primary.base.color,
        extended.success.base.color,
        extended.warning.base.color,
        extended.danger.base.color,
        extended.secondary.base.color,
    ];
    let base = bases[index % bases.len()];
    // iced::theme::palette::{mix, deviate, darken, lighten, readable, is_dark} are public in 0.14
    palette::mix(
        base,
        extended.background.base.color,
        (0.15 * (index / bases.len()) as f32).min(0.6),
    )
}
