//! Application state, messages, update dispatch, subscriptions and the top-level views.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dirstat_core::catalog::{Engine, Resolver};
use dirstat_core::platform::{self, Mount};
use dirstat_core::scan::{Options, ScanRequest, StartError, Update};
use dirstat_core::tree::NodeId;
use dirstat_core::volumes::{self, CardKind, VolumeCard};
use iced::widget::{button, center, column, container, row, space, stack};
use iced::{Center, Element, Fill, Size, Subscription, Task, Theme, event, keyboard, time, window};

use crate::keys::{self, KeyAction};
use crate::prefs::{JsonPrefs, Prefs, PrefsStore};
use crate::results::scan_task::{self, ScanIdGen};
use crate::results::{self, Results, ScanOutcome, StopOutcome, activity};
use crate::start::{self, StartState};
use crate::{Cli, format, theme};

/// Everything the app gets from the outside world. `App::boot` builds the real one; tests
/// build one without disk or settings.
pub struct Env {
    pub engine: Arc<Engine>,
    pub prefs_store: Box<dyn PrefsStore>,
    pub cards: Vec<VolumeCard>,
    /// The mount table at startup, for progress estimates (never blocks).
    pub mounts: Vec<Mount>,
    pub fda: Option<bool>,
    pub system_name: String,
}

pub enum Screen {
    Start(StartState),
    Results(Box<Results>),
}

pub struct App {
    env: Env,
    prefs: Prefs,
    pub screen: Screen,
    window_size: Size,
    drag_hover: bool,
    scan_ids: ScanIdGen,
    #[cfg(feature = "screenshot")]
    script: Option<crate::script::Script>,
    /// Where the screenshot in flight goes.
    #[cfg(feature = "screenshot")]
    shot_target: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Message {
    // start screen
    ScanPath(PathBuf),
    ChooseFolder,
    FolderChosen(Option<PathBuf>),
    FileHovered,
    FilesHoveredLeft,
    FileDropped(PathBuf),
    OpenPrivacySettings,
    // scanning
    Scan(scan_task::ScanId, Update),
    Stop,
    NewScan,
    RescanAll,
    /// 50 ms, only while something is live (design §6.5).
    Tick(Instant),
    // chrome
    Key(KeyAction),
    WindowResized(Size),
    PlatformResult(Result<(), String>),
    #[cfg(feature = "screenshot")]
    Screenshot(window::Screenshot),
    #[cfg(feature = "screenshot")]
    ScreenshotSaved(Result<PathBuf, String>),
}

impl App {
    /// The real environment: catalog, prefs file, mount table, privacy state.
    pub fn boot(
        cli: Cli,
        #[cfg(feature = "screenshot")] script: Option<crate::script::Script>,
    ) -> (App, Task<Message>) {
        let mounts = platform::mounts();
        let system_name = platform::system_disk_name();
        let env = Env {
            engine: Arc::new(Engine::new(&Resolver::from_env().canonicalized())),
            prefs_store: Box::new(JsonPrefs::in_config_dir()),
            cards: volumes::cards_from(platform::home_dir().as_deref(), &mounts, &system_name),
            mounts,
            fda: platform::full_disk_access(),
            system_name,
        };
        let (app, task) = App::new(env, cli);
        #[cfg(feature = "screenshot")]
        let app = App { script, ..app };
        (app, task)
    }

    /// No I/O: tests construct the app with a fake `Env`.
    pub fn new(env: Env, cli: Cli) -> (App, Task<Message>) {
        let prefs = env.prefs_store.load();
        let start = StartState {
            cards: env.cards.clone(),
            banner: None,
            focused: None,
            fda: env.fda,
            system_name: env.system_name.clone(),
        };
        let app = App {
            env,
            prefs,
            screen: Screen::Start(start),
            window_size: Size::new(1200.0, 800.0),
            drag_hover: false,
            scan_ids: ScanIdGen::default(),
            #[cfg(feature = "screenshot")]
            script: None,
            #[cfg(feature = "screenshot")]
            shot_target: None,
        };
        let task = cli
            .path
            .map_or_else(Task::none, |path| Task::done(Message::ScanPath(path)));
        (app, task)
    }

    pub fn title(&self) -> String {
        match &self.screen {
            Screen::Results(r) => format!("{} — dir-stat", r.title),
            Screen::Start(_) => "dir-stat".into(),
        }
    }

    /// Always `None`: iced then uses its Light/Dark theme and follows the OS live.
    pub fn theme(&self) -> Option<Theme> {
        None
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ScanPath(path) => return self.start_scan(path),
            Message::ChooseFolder => return choose_folder(),
            Message::FolderChosen(Some(path)) => return self.start_scan(path),
            Message::FolderChosen(None) => {}
            Message::FileHovered => self.drag_hover = true,
            Message::FilesHoveredLeft => self.drag_hover = false,
            Message::FileDropped(path) => {
                self.drag_hover = false;
                return self.start_scan(path);
            }
            Message::OpenPrivacySettings => {
                if let Some(url) = platform::PRIVACY_SETTINGS_URL {
                    return blocking(
                        move || opener::open(url).map_err(|e| e.to_string()),
                        Message::PlatformResult,
                    );
                }
            }
            Message::Scan(id, update) => self.on_scan(id, update),
            Message::Stop => self.stop(),
            Message::NewScan => self.show_start(None),
            Message::RescanAll => {
                if let Screen::Results(r) = &self.screen
                    && r.can_rescan()
                {
                    let root = r.tree.root_path().to_path_buf();
                    return self.start_scan(root);
                }
            }
            Message::Tick(now) => return self.tick(now),
            Message::Key(action) => return self.key(action),
            Message::WindowResized(size) => self.window_size = size,
            Message::PlatformResult(result) => {
                if let Err(error) = result {
                    eprintln!("dir-stat: {error}");
                }
            }
            #[cfg(feature = "screenshot")]
            Message::Screenshot(screenshot) => {
                let Some(target) = self.shot_target.take() else {
                    return Task::none();
                };
                return blocking(
                    move || crate::script::save_png(&screenshot, &target),
                    Message::ScreenshotSaved,
                );
            }
            #[cfg(feature = "screenshot")]
            Message::ScreenshotSaved(result) => match result {
                Ok(path) => {
                    eprintln!("dir-stat: screenshot saved to {}", path.display());
                    if let Some(script) = &mut self.script {
                        script.shot_saved(Instant::now());
                    }
                }
                Err(error) => {
                    eprintln!("dir-stat: screenshot failed: {error}");
                    return iced::exit();
                }
            },
        }
        Task::none()
    }

    /// Switches to the Results screen at once ("Opening…") and starts the scan.
    fn start_scan(&mut self, path: PathBuf) -> Task<Message> {
        let id = self.scan_ids.next();
        let title = title_for(&path, &self.env.cards);
        let estimate = volumes::estimate(&path, &self.env.mounts);
        let (active, task) = scan_task::launch(
            id,
            ScanRequest::new(path.clone()),
            Options::new(Arc::clone(&self.env.engine)),
        );
        // Replacing the screen drops a previous scan, which cancels it.
        self.screen = Screen::Results(Box::new(Results::new(path, title, estimate, active)));
        task
    }

    fn on_scan(&mut self, id: scan_task::ScanId, update: Update) {
        let Screen::Results(results) = &mut self.screen else {
            return;
        };
        match results.on_scan(id, update, Instant::now()) {
            ScanOutcome::Ignored | ScanOutcome::Applied => {}
            ScanOutcome::Started(root) => {
                self.prefs.add_recent(root, unix_now());
                self.env.prefs_store.save(&self.prefs);
            }
            ScanOutcome::Failed(error) => {
                let message = failure_banner(results.tree.root_path(), &error);
                self.show_start(Some(message));
            }
        }
    }

    fn stop(&mut self) {
        if let Screen::Results(results) = &mut self.screen
            && results.stop() == StopOutcome::BackToStart
        {
            self.show_start(None);
        }
    }

    fn show_start(&mut self, banner: Option<String>) {
        self.screen = Screen::Start(StartState {
            cards: self.env.cards.clone(),
            banner,
            focused: None,
            fda: self.env.fda,
            system_name: self.env.system_name.clone(),
        });
    }

    fn key(&mut self, action: KeyAction) -> Task<Message> {
        match (&mut self.screen, action) {
            (_, KeyAction::ChooseFolder) => return choose_folder(),
            (Screen::Start(start), action) => {
                if let Some(message) = start.key(action) {
                    return Task::done(message);
                }
            }
            (Screen::Results(_), KeyAction::Escape | KeyAction::Stop) => self.stop(),
            (Screen::Results(_), KeyAction::NewScan) => self.show_start(None),
            (Screen::Results(_), KeyAction::RescanAll) => return self.update(Message::RescanAll),
            _ => {}
        }
        Task::none()
    }

    fn tick(&mut self, now: Instant) -> Task<Message> {
        if let Screen::Results(results) = &mut self.screen {
            results.tick(now);
        }
        #[cfg(feature = "screenshot")]
        if let Some(script) = &mut self.script {
            use crate::script::Due;
            match script.poll(now) {
                Due::Nothing => {}
                Due::Shot(target) => {
                    self.shot_target = Some(target);
                    return window::latest()
                        .and_then(window::screenshot)
                        .map(Message::Screenshot);
                }
                Due::Path(path) => return self.start_scan(path),
                Due::Key(action) => return self.key(action),
                Due::Done => return iced::exit(),
            }
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            keyboard::listen().filter_map(keys::map),
            event::listen_with(window_event),
        ];
        if self.needs_ticks() {
            subscriptions.push(time::every(Duration::from_millis(50)).map(Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

    /// When idle, nothing wakes the app.
    fn needs_ticks(&self) -> bool {
        #[cfg(feature = "screenshot")]
        if self.script.is_some() {
            return true;
        }
        matches!(&self.screen, Screen::Results(r) if r.is_scanning())
    }

    pub fn view(&self) -> Element<'_, Message> {
        let base = match &self.screen {
            Screen::Start(start) => start::view(start),
            Screen::Results(results) => self.results_view(results),
        };
        let overlay = self.drag_hover.then(|| {
            container(center(theme::TITLE.text("Drop to scan")))
                .width(Fill)
                .height(Fill)
                .style(theme::drop_overlay)
        });
        stack![base].push(overlay).into()
    }

    fn results_view<'a>(&'a self, results: &'a Results) -> Element<'a, Message> {
        let field_width = (self.window_size.width * 0.5).clamp(360.0, 640.0);
        let new_scan = button(theme::BODY_STRONG.text("+ New Scan"))
            .padding([4, 12])
            .style(theme::secondary_button)
            .on_press(Message::NewScan);
        let toolbar = row![
            container(new_scan).width(Fill),
            activity::view(results, &results.title, field_width, Instant::now()),
            space().width(Fill),
        ]
        .align_y(Center)
        .padding([0, 12])
        .height(44);

        let total = if results.tree.is_started() {
            let alloc = results.tree.node(NodeId::ROOT).total().alloc;
            if results.tree.is_lower_bound(NodeId::ROOT) || results.is_scanning() {
                format::size_at_least(alloc)
            } else {
                format::size(alloc)
            }
        } else {
            format::PENDING.into()
        };
        let path_bar = row![
            theme::BODY
                .text(results.tree.root_path().display().to_string())
                .width(Fill),
            theme::BODY_STRONG.text(total),
        ]
        .padding([0, 16])
        .height(32)
        .align_y(Center);

        column![
            toolbar,
            container(space())
                .width(Fill)
                .height(1)
                .style(theme::hairline),
            path_bar,
            container(space())
                .width(Fill)
                .height(1)
                .style(theme::hairline),
            results::top_level_list(results),
        ]
        .into()
    }
}

/// Drop and resize events, regardless of which widget captured them.
fn window_event(
    event: iced::Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(window::Event::FileHovered(_)) => Some(Message::FileHovered),
        iced::Event::Window(window::Event::FilesHoveredLeft) => Some(Message::FilesHoveredLeft),
        iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
        iced::Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
        _ => None,
    }
}

/// The folder picker must be created on the main thread, i.e. here in `update` (API §g).
fn choose_folder() -> Task<Message> {
    let dialog = rfd::AsyncFileDialog::new().set_title("Choose a folder to analyze");
    let dialog = match platform::home_dir() {
        Some(home) => dialog.set_directory(home),
        None => dialog,
    };
    Task::perform(dialog.pick_folder(), |handle| {
        Message::FolderChosen(handle.map(|h| h.path().to_path_buf()))
    })
}

/// Runs blocking work (filesystem, processes) off the UI thread.
fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(Result<T, String>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                platform::init_io_thread();
                work()
            })
            .await
            .map_err(|e| e.to_string())?
        },
        done,
    )
}

/// "Macintosh HD" for a volume card's root, else the folder's name.
fn title_for(path: &Path, cards: &[VolumeCard]) -> String {
    if let Some(card) = cards
        .iter()
        .find(|c| c.path == path && c.kind != CardKind::Home)
    {
        return card.name.clone();
    }
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// "Can't scan “/foo”: No such folder."
pub fn failure_banner(path: &Path, error: &StartError) -> String {
    use std::io::ErrorKind;
    let reason = match error.kind {
        ErrorKind::NotFound => "No such folder".into(),
        ErrorKind::NotADirectory => "Not a folder".into(),
        ErrorKind::PermissionDenied => "Permission denied".into(),
        _ => error.message.clone(),
    };
    format!("Can't scan “{}”: {reason}.", path.display())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
impl App {
    /// Shows the Results screen for a scan with no thread behind it; the test feeds its
    /// updates as `Message::Scan(id, ..)`.
    pub fn start_simulated(&mut self, path: &str) -> scan_task::ScanId {
        let id = self.scan_ids.next();
        let active = scan_task::ActiveScan::inert(id);
        self.screen = Screen::Results(Box::new(Results::new(
            path.into(),
            "test".into(),
            None,
            active,
        )));
        id
    }

    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }
}
