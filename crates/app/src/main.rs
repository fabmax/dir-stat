mod app;
mod fonts;
mod format;
mod keys;
mod prefs;
mod results;
#[cfg(feature = "screenshot")]
mod script;
mod start;
#[cfg(test)]
mod tests;
mod theme;
mod widgets;

use std::path::PathBuf;

use iced::{Size, window};

use app::App;

fn main() -> iced::Result {
    let cli = Cli::parse_env();
    dirstat_core::platform::init_process();

    #[cfg(feature = "screenshot")]
    let script = match script::Script::from_env(std::time::Instant::now()) {
        Ok(script) => script,
        Err(error) => {
            eprintln!("dir-stat: {error}");
            std::process::exit(2);
        }
    };
    let boot = move || {
        App::boot(
            cli.clone(),
            #[cfg(feature = "screenshot")]
            script.clone(),
        )
    };

    iced::application(boot, App::update, App::view)
        // First: settings() replaces fonts and default_font.
        .settings(iced::Settings {
            default_text_size: iced::Pixels(13.0),
            ..Default::default()
        })
        // Before the other window options: window() replaces them.
        .window(window::Settings {
            size: Size::new(1200.0, 800.0),
            min_size: Some(Size::new(800.0, 500.0)),
            ..window::Settings::default()
        })
        .title(App::title)
        .theme(App::theme)
        .style(theme::app_style)
        .subscription(App::subscription)
        .font(fonts::INTER_REGULAR)
        .font(fonts::INTER_MEDIUM)
        .font(fonts::INTER_SEMIBOLD)
        .default_font(fonts::INTER)
        .run()
}

/// Command line: `dir-stat [<path>] | --help | --version`.
#[derive(Debug, Clone, Default)]
pub struct Cli {
    /// Scan this folder right away instead of showing the start screen.
    pub path: Option<PathBuf>,
}

impl Cli {
    fn parse_env() -> Self {
        let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        match args.first().and_then(|a| a.to_str()) {
            None if args.is_empty() => Self::default(),
            Some("-h" | "--help") => {
                println!(
                    "Usage: dir-stat [<folder>]\n\nShows what uses your disk. With a folder, scans it right away."
                );
                std::process::exit(0);
            }
            Some("-V" | "--version") => {
                println!("dir-stat {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            Some(flag) if flag.starts_with('-') => {
                eprintln!("dir-stat: unknown option {flag:?} (try --help)");
                std::process::exit(2);
            }
            _ if args.len() > 1 => {
                eprintln!("dir-stat: expected one folder (try --help)");
                std::process::exit(2);
            }
            _ => {
                // Relative paths resolve against the current directory; a bad path is
                // reported by the scan (start screen with a banner).
                let path = PathBuf::from(&args[0]);
                Self {
                    path: Some(std::path::absolute(&path).unwrap_or(path)),
                }
            }
        }
    }
}
