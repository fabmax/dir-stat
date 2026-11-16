//! The start screen (UX §4): one card per location; a card is the button that scans it.

use dirstat_core::platform::Space;
use dirstat_core::volumes::{CardKind, VolumeCard};
use iced::widget::{
    Column, button, center, column, container, progress_bar, row, scrollable, space, text, tooltip,
};
use iced::{Center, Element, Fill};

use crate::app::Message;
use crate::keys::KeyAction;
use crate::{fonts, format, theme};

const CARD_WIDTH: f32 = 560.0;

pub struct StartState {
    pub cards: Vec<VolumeCard>,
    /// "Can't scan “/foo”: No such folder."
    pub banner: Option<String>,
    /// Keyboard focus; none until an arrow key is pressed.
    pub focused: Option<usize>,
    pub fda: Option<bool>,
    pub system_name: String,
    pub recents: Vec<Recent>,
}

/// A recent scan root (UX §4): clicking it scans again; results aren't cached.
#[derive(Clone, Debug)]
pub struct Recent {
    pub path: std::path::PathBuf,
    /// `~/dev`
    pub shown: String,
    /// "yesterday"
    pub when: String,
}

impl StartState {
    /// Arrow keys move the focus ring, Return scans the focused card.
    pub fn key(&mut self, action: KeyAction) -> Option<Message> {
        let last = self.cards.len().checked_sub(1)?;
        match action {
            KeyAction::Down => self.focused = Some(self.focused.map_or(0, |f| (f + 1).min(last))),
            KeyAction::Up => self.focused = Some(self.focused.map_or(0, |f| f.saturating_sub(1))),
            KeyAction::Enter => {
                return self
                    .focused
                    .map(|f| Message::ScanPath(self.cards[f].path.clone()));
            }
            _ => {}
        }
        None
    }
}

pub fn view(state: &StartState) -> Element<'_, Message> {
    let header = column![
        theme::DISPLAY.text("dir-stat"),
        theme::BODY
            .text("Find out what is using your disk.")
            .style(theme::text_secondary),
    ]
    .spacing(4)
    .align_x(Center);

    let cards = Column::with_children(
        state
            .cards
            .iter()
            .enumerate()
            .map(|(i, c)| card(state, c, i)),
    )
    .spacing(8)
    .width(CARD_WIDTH);

    let choose = row![
        button(theme::BODY_STRONG.text("Choose Folder…"))
            .padding([6, 14])
            .style(theme::secondary_button)
            .on_press(Message::ChooseFolder),
        theme::BODY
            .text("or drop a folder onto this window")
            .style(theme::text_secondary),
    ]
    .spacing(12)
    .align_y(Center);

    let mut content = column![]
        .spacing(24)
        .align_x(Center)
        .width(Fill)
        .padding([40, 24]);
    if let Some(message) = &state.banner {
        content = content.push(banner(message));
    }
    content = content.push(header).push(cards).push(choose);
    if !state.recents.is_empty() {
        content = content.push(recents(&state.recents));
    }
    if cfg!(target_os = "macos") && state.fda != Some(true) {
        content = content.push(footnote());
    }
    scrollable(center(content).height(iced::Shrink))
        .height(Fill)
        .into()
}

fn card<'a>(state: &'a StartState, card: &'a VolumeCard, index: usize) -> Element<'a, Message> {
    let icon = if card.kind == CardKind::Home {
        "⌂"
    } else {
        "▭"
    };
    let right: Option<String> = match (card.kind, card.space) {
        (CardKind::Home, _) => None,
        (CardKind::Network, _) => Some("Network".into()),
        (CardKind::External, Some(space)) => {
            Some(format!("External · {}", format::size(space.total)))
        }
        (CardKind::External, None) => Some("External".into()),
        (CardKind::System, space) => space.map(|s| format::size(s.total)),
    };
    let title = row![
        theme::BODY_STRONG
            .text(card.name.as_str())
            .width(Fill)
            .wrapping(text::Wrapping::None),
    ]
    .push(right.map(|r| theme::SMALL.text(r).style(theme::text_secondary)))
    .align_y(Center);

    let mut details = column![title].spacing(6).width(Fill);
    match (card.kind, card.space) {
        (CardKind::Home, _) => {
            details = details.push(
                theme::SMALL
                    .text(format!(
                        "{} · on {}",
                        card.path.display(),
                        state.system_name
                    ))
                    .style(theme::text_secondary),
            );
            if cfg!(target_os = "macos") {
                details = details.push(
                    theme::SMALL
                        .text("macOS will ask for access to Desktop, Documents and Downloads")
                        .style(theme::text_muted),
                );
            }
        }
        (CardKind::Network, _) => {
            details = details.push(
                theme::SMALL
                    .text("Network volumes can be slow to scan")
                    .style(theme::text_secondary),
            );
        }
        (_, Some(space)) => details = details.push(meter(space)),
        (_, None) => {}
    }

    let content = row![
        container(theme::BODY.text(icon).style(theme::text_secondary)).width(20),
        details,
        theme::BODY.text("›").style(theme::text_muted),
    ]
    .spacing(10)
    .align_y(Center);

    let focused = state.focused == Some(index);
    button(content)
        .width(Fill)
        .padding([12, 16])
        .style(move |t, s| theme::card_button(t, s, focused))
        .on_press(Message::ScanPath(card.path.clone()))
        .into()
}

fn meter<'a>(space: Space) -> Element<'a, Message> {
    let used = space.total.saturating_sub(space.free);
    let fraction = if space.total == 0 {
        0.0
    } else {
        used as f32 / space.total as f32
    };
    // Under 10% free: critical color, always with an icon and words.
    let critical = space.total > 0 && space.free * 10 < space.total;
    let bar = progress_bar(0.0..=1.0, fraction)
        .girth(6)
        .style(move |t| theme::meter(t, critical));
    let caption: Element<'a, Message> = if critical {
        row![
            theme::SMALL.text("!").style(theme::text_critical),
            theme::SMALL
                .text(format!("Almost full · {} free", format::size(space.free)))
                .style(theme::text_secondary),
        ]
        .spacing(4)
        .into()
    } else {
        theme::SMALL
            .text(format!(
                "{} used · {} free",
                format::size(space.used),
                format::size(space.free)
            ))
            .style(theme::text_secondary)
            .into()
    };
    let tip = container(
        theme::SMALL
            .text("Finder may show more available space because it counts purgeable data as free."),
    )
    .padding([4, 8])
    .max_width(300)
    .style(theme::raised);
    tooltip(
        column![bar, caption].spacing(6),
        tip,
        tooltip::Position::Bottom,
    )
    .into()
}

fn recents(recents: &[Recent]) -> Element<'_, Message> {
    let mut list = Column::new().spacing(0).width(CARD_WIDTH);
    list = list.push(
        container(theme::BODY_STRONG.text("Recent"))
            .padding([0, 12])
            .height(28)
            .center_y(28),
    );
    for recent in recents {
        let line = row![
            theme::BODY
                .text(crate::widgets::truncate_middle(
                    &recent.shown,
                    340.0,
                    theme::BODY.size
                ))
                .wrapping(text::Wrapping::None)
                .width(Fill),
            theme::SMALL
                .text(recent.when.clone())
                .style(theme::text_secondary),
            theme::BODY.text("›").style(theme::text_muted),
        ]
        .spacing(12)
        .align_y(Center);
        let scan = button(line)
            .padding([5, 12])
            .width(Fill)
            .style(theme::row_button)
            .on_press(Message::ScanPath(recent.path.clone()));
        let remove = tooltip(
            button(theme::BODY.text("×").style(theme::text_muted))
                .padding([4, 8])
                .style(theme::row_button)
                .on_press(Message::RemoveRecent(recent.path.clone())),
            container(theme::SMALL.text("Remove from Recent"))
                .padding([4, 8])
                .style(theme::raised),
            tooltip::Position::Top,
        );
        list = list.push(row![scan, remove].align_y(Center));
    }
    list.into()
}

fn banner(message: &str) -> Element<'_, Message> {
    container(
        row![
            theme::BODY_STRONG.text("!").style(theme::text_critical),
            theme::BODY.text(message),
        ]
        .spacing(10)
        .align_y(Center),
    )
    .padding([10, 16])
    .width(CARD_WIDTH)
    .style(theme::banner)
    .into()
}

fn footnote<'a>() -> Element<'a, Message> {
    let text_line = theme::SMALL
        .text("macOS may ask for access to some folders. For complete results, give dir-stat Full Disk Access.")
        .style(theme::text_secondary);
    let link = button(
        theme::SMALL
            .with_font(fonts::MEDIUM)
            .text("Open Privacy Settings"),
    )
    .padding(0)
    .style(theme::link_button)
    .on_press(Message::OpenPrivacySettings);
    row![
        theme::SMALL.text("ⓘ").style(theme::text_secondary),
        column![text_line, link].spacing(4),
        space().width(0),
    ]
    .spacing(8)
    .width(CARD_WIDTH)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(cards: usize) -> StartState {
        let card = |i: usize| VolumeCard {
            path: format!("/v{i}").into(),
            name: format!("v{i}"),
            kind: CardKind::External,
            space: None,
        };
        StartState {
            cards: (0..cards).map(card).collect(),
            banner: None,
            focused: None,
            fda: None,
            system_name: "Disk".into(),
            recents: Vec::new(),
        }
    }

    #[test]
    fn arrow_keys_move_focus_and_return_scans() {
        let mut s = state(3);
        assert!(s.key(KeyAction::Enter).is_none(), "nothing focused yet");
        s.key(KeyAction::Down);
        assert_eq!(s.focused, Some(0));
        s.key(KeyAction::Down);
        s.key(KeyAction::Down);
        s.key(KeyAction::Down);
        assert_eq!(s.focused, Some(2), "stops at the last card");
        s.key(KeyAction::Up);
        assert!(
            matches!(s.key(KeyAction::Enter), Some(Message::ScanPath(p)) if p == std::path::Path::new("/v1"))
        );
        let mut empty = state(0);
        assert!(empty.key(KeyAction::Down).is_none());
    }
}
