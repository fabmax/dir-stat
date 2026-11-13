//! Keyboard shortcuts (UX §6.4, design §6.8). Mapping is a pure function per platform, so the
//! whole table is tested on any OS. Routing to panes and overlays happens in `update`.

use iced::keyboard::key::{Code, Named, Physical};
use iced::keyboard::{self, Key};

use crate::app::Message;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Right,
    Left,
    ExpandLargest,
    CollapseSubtree,
    Enter,
    ZoomOut,
    Back,
    Forward,
    Reveal,
    CopyPath,
    Trash,
    Rescan,
    RescanAll,
    Escape,
    Stop,
    NewScan,
    ChooseFolder,
    PaneCleanup,
    PaneMap,
    TogglePane,
    Tab,
    ShiftTab,
    Space,
    Help,
}

#[cfg(any(test, feature = "screenshot"))]
impl KeyAction {
    pub const ALL: [KeyAction; 30] = {
        use KeyAction::*;
        [
            Up,
            Down,
            PageUp,
            PageDown,
            Home,
            End,
            Right,
            Left,
            ExpandLargest,
            CollapseSubtree,
            Enter,
            ZoomOut,
            Back,
            Forward,
            Reveal,
            CopyPath,
            Trash,
            Rescan,
            RescanAll,
            Escape,
            Stop,
            NewScan,
            ChooseFolder,
            PaneCleanup,
            PaneMap,
            TogglePane,
            Tab,
            ShiftTab,
            Space,
            Help,
        ]
    };

    /// By variant name, for `DIRSTAT_SCRIPT` `key:` steps.
    pub fn from_name(name: &str) -> Option<KeyAction> {
        Self::ALL.into_iter().find(|a| format!("{a:?}") == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Other,
}

impl Platform {
    pub const CURRENT: Platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else {
        Platform::Other
    };
}

/// The subscription's `filter_map` (a fn item: iced requires non-capturing closures).
pub fn map(event: keyboard::Event) -> Option<Message> {
    map_for(Platform::CURRENT, &event).map(Message::Key)
}

pub fn map_for(platform: Platform, event: &keyboard::Event) -> Option<KeyAction> {
    use KeyAction::*;
    let keyboard::Event::KeyPressed {
        key,
        modifiers,
        physical_key,
        ..
    } = event
    else {
        return None;
    };
    let mac = platform == Platform::Mac;
    // ⌘ on macOS, Ctrl elsewhere. Not `Modifiers::command()`, which follows the build OS.
    let cmd = if mac {
        modifiers.logo()
    } else {
        modifiers.control()
    };
    let (alt, shift) = (modifiers.alt(), modifiers.shift());
    let plain = !cmd && !alt && !shift && !(mac && modifiers.control());

    if cmd && let Physical::Code(code) = physical_key {
        match code {
            Code::BracketLeft if mac => return Some(Back),
            Code::BracketRight if mac => return Some(Forward),
            _ => {}
        }
    }
    let action = match key.as_ref() {
        Key::Named(named) => match named {
            Named::ArrowUp if cmd && mac || alt && !mac => ZoomOut,
            Named::ArrowUp if plain => Up,
            Named::ArrowDown if plain => Down,
            Named::ArrowRight if alt && mac || shift && !mac => ExpandLargest,
            Named::ArrowLeft if alt && mac || shift && !mac => CollapseSubtree,
            Named::ArrowRight if alt && !mac => Forward,
            Named::ArrowLeft if alt && !mac => Back,
            Named::ArrowRight if plain => Right,
            Named::ArrowLeft if plain => Left,
            Named::PageUp => PageUp,
            Named::PageDown => PageDown,
            Named::Home => Home,
            Named::End => End,
            Named::Enter => Enter,
            Named::Backspace if cmd && mac => Trash,
            Named::Backspace if plain => ZoomOut,
            Named::Delete if !mac && plain => Trash,
            Named::Escape => Escape,
            Named::Tab if shift => ShiftTab,
            Named::Tab => Tab,
            Named::Space if plain => Space,
            Named::F5 if !mac && shift => RescanAll,
            Named::F5 if !mac => Rescan,
            Named::F1 if !mac => Help,
            _ => return None,
        },
        Key::Character(c) => {
            let c = c.to_lowercase();
            match c.as_str() {
                "r" if cmd && alt && mac || alt && shift && !mac && !cmd => Reveal,
                "r" if cmd && shift => RescanAll,
                "r" if cmd => Rescan,
                "c" if cmd => CopyPath,
                "n" if cmd => NewScan,
                "o" if cmd => ChooseFolder,
                "1" if cmd => PaneCleanup,
                "2" if cmd => PaneMap,
                "0" if cmd => TogglePane,
                "." if cmd && mac => Stop,
                "/" if cmd && mac => Help,
                "?" => Help,
                _ => return None,
            }
        }
        Key::Unidentified => return None,
    };
    Some(action)
}

#[cfg(test)]
mod tests {
    use super::KeyAction::*;
    use super::*;
    use iced::keyboard::{Location, Modifiers};

    fn press(key: Key, physical: Physical, modifiers: Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    fn named(n: Named, m: Modifiers) -> keyboard::Event {
        press(
            Key::Named(n),
            Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            m,
        )
    }

    fn ch(c: &str, m: Modifiers) -> keyboard::Event {
        press(
            Key::Character(c.into()),
            Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            m,
        )
    }

    const NONE: Modifiers = Modifiers::empty();
    const CMD: Modifiers = Modifiers::LOGO;
    const CTRL: Modifiers = Modifiers::CTRL;
    const ALT: Modifiers = Modifiers::ALT;
    const SHIFT: Modifiers = Modifiers::SHIFT;

    fn mac(e: keyboard::Event) -> Option<KeyAction> {
        map_for(Platform::Mac, &e)
    }
    fn other(e: keyboard::Event) -> Option<KeyAction> {
        map_for(Platform::Other, &e)
    }

    #[test]
    fn navigation() {
        for f in [mac, other] {
            assert_eq!(f(named(Named::ArrowUp, NONE)), Some(Up));
            assert_eq!(f(named(Named::ArrowDown, NONE)), Some(Down));
            assert_eq!(f(named(Named::PageUp, NONE)), Some(PageUp));
            assert_eq!(f(named(Named::PageDown, NONE)), Some(PageDown));
            assert_eq!(f(named(Named::Home, NONE)), Some(Home));
            assert_eq!(f(named(Named::End, NONE)), Some(End));
            assert_eq!(f(named(Named::ArrowRight, NONE)), Some(Right));
            assert_eq!(f(named(Named::ArrowLeft, NONE)), Some(Left));
            assert_eq!(f(named(Named::Enter, NONE)), Some(Enter));
            assert_eq!(f(named(Named::Backspace, NONE)), Some(ZoomOut));
            assert_eq!(f(named(Named::Escape, NONE)), Some(Escape));
            assert_eq!(f(named(Named::Tab, NONE)), Some(Tab));
            assert_eq!(f(named(Named::Tab, SHIFT)), Some(ShiftTab));
            assert_eq!(f(named(Named::Space, NONE)), Some(Space));
            assert_eq!(f(ch("?", SHIFT)), Some(Help));
        }
        assert_eq!(mac(named(Named::ArrowUp, CMD)), Some(ZoomOut));
        assert_eq!(other(named(Named::ArrowUp, ALT)), Some(ZoomOut));
        assert_eq!(mac(named(Named::ArrowRight, ALT)), Some(ExpandLargest));
        assert_eq!(mac(named(Named::ArrowLeft, ALT)), Some(CollapseSubtree));
        assert_eq!(other(named(Named::ArrowRight, SHIFT)), Some(ExpandLargest));
        assert_eq!(other(named(Named::ArrowLeft, SHIFT)), Some(CollapseSubtree));
    }

    #[test]
    fn history_uses_physical_brackets_on_mac() {
        let bracket = |code, m| press(Key::Character("ö".into()), Physical::Code(code), m);
        assert_eq!(
            mac(bracket(Code::BracketLeft, CMD)),
            Some(Back),
            "any keyboard layout"
        );
        assert_eq!(mac(bracket(Code::BracketRight, CMD)), Some(Forward));
        assert_eq!(other(named(Named::ArrowLeft, ALT)), Some(Back));
        assert_eq!(other(named(Named::ArrowRight, ALT)), Some(Forward));
        assert_eq!(
            mac(named(Named::ArrowLeft, ALT)),
            Some(CollapseSubtree),
            "not Back on macOS"
        );
    }

    #[test]
    fn actions_on_mac() {
        assert_eq!(mac(ch("r", CMD.union(ALT))), Some(Reveal));
        assert_eq!(mac(ch("c", CMD)), Some(CopyPath));
        assert_eq!(mac(ch("c", CMD.union(ALT))), Some(CopyPath));
        assert_eq!(mac(named(Named::Backspace, CMD)), Some(Trash));
        assert_eq!(mac(ch("r", CMD)), Some(Rescan));
        // With ⌘ held, winit keeps Shift in the character: ⇧⌘R arrives as "R".
        assert_eq!(mac(ch("R", CMD.union(SHIFT))), Some(RescanAll));
        assert_eq!(mac(ch(".", CMD)), Some(Stop));
        assert_eq!(mac(ch("N", CMD)), Some(NewScan));
        assert_eq!(mac(ch("o", CMD)), Some(ChooseFolder));
        assert_eq!(mac(ch("1", CMD)), Some(PaneCleanup));
        assert_eq!(mac(ch("2", CMD)), Some(PaneMap));
        assert_eq!(mac(ch("0", CMD)), Some(TogglePane));
        assert_eq!(mac(ch("/", CMD)), Some(Help));
        assert_eq!(mac(ch("o", CTRL)), None, "Ctrl is not ⌘");
        assert_eq!(mac(named(Named::Delete, NONE)), None);
    }

    #[test]
    fn actions_on_linux_and_windows() {
        assert_eq!(other(ch("R", SHIFT.union(ALT))), Some(Reveal));
        assert_eq!(other(ch("c", CTRL)), Some(CopyPath));
        assert_eq!(other(ch("C", CTRL.union(SHIFT))), Some(CopyPath));
        assert_eq!(other(named(Named::Delete, NONE)), Some(Trash));
        assert_eq!(other(ch("r", CTRL)), Some(Rescan));
        assert_eq!(other(named(Named::F5, NONE)), Some(Rescan));
        assert_eq!(other(ch("R", CTRL.union(SHIFT))), Some(RescanAll));
        assert_eq!(other(named(Named::F5, SHIFT)), Some(RescanAll));
        assert_eq!(other(ch("n", CTRL)), Some(NewScan));
        assert_eq!(other(ch("o", CTRL)), Some(ChooseFolder));
        assert_eq!(other(named(Named::F1, NONE)), Some(Help));
        assert_eq!(other(ch(".", CTRL)), None, "no ⌘. off macOS");
        assert_eq!(other(ch("o", CMD)), None, "the Windows key is not Ctrl");
    }

    #[test]
    fn script_names() {
        assert_eq!(KeyAction::from_name("RescanAll"), Some(RescanAll));
        assert_eq!(KeyAction::from_name("rescanall"), None);
        for action in KeyAction::ALL {
            assert_eq!(KeyAction::from_name(&format!("{action:?}")), Some(action));
        }
    }
}
