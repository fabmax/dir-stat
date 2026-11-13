mod app;
mod fonts;
mod format;
#[cfg(feature = "screenshot")]
mod script;
mod specimen;
mod theme;

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
        App::new(
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

/// Command line: `dir-stat [--help | --version]`. A path argument arrives with scanning.
#[derive(Debug, Clone, Default)]
pub struct Cli {}

impl Cli {
    fn parse_env() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            None => Self {},
            Some("-h" | "--help") => {
                println!("Usage: dir-stat [--help | --version]");
                std::process::exit(0);
            }
            Some("-V" | "--version") => {
                println!("dir-stat {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            Some(other) => {
                eprintln!("dir-stat: unexpected argument {other:?} (try --help)");
                std::process::exit(2);
            }
        }
    }
}
