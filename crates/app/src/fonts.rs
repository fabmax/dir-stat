//! Bundled Inter 4.x static faces (UX §11.3). SF on macOS is registered only at weight 400,
//! so every weight we use must be bundled (iced API gotcha 1). Bold (700) is never used.

use iced::Font;
use iced::font::Weight;

pub const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

/// The default font, weight 400.
pub const INTER: Font = Font::with_name("Inter");
pub const MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..INTER
};
pub const SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..INTER
};
