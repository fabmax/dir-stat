//! iced 0.14 API probe for dir-stat.
//!
//! Run:  cargo run -- /usr/share          (scan a path given on the CLI at boot)
//! Probe automation via env vars:
//!   PROBE_SHOT=/path/shot.png  PROBE_SHOT_AFTER_MS=2500   save an iced window screenshot
//!   PROBE_EXIT_AFTER_MS=5000                              exit via iced::exit()
//!   PROBE_MODAL=1                                         open the confirm modal at boot
//!   PROBE_PLAYGROUND=1                                    show the widget playground
//!   PROBE_FONTS=1                                         show the font fallback probe
//!   PROBE_DIALOG=1                                        open the rfd folder picker at boot
//!   PROBE_SUBS=1                                          enable demo subscriptions
//!   PROBE_EXPAND_ALL=1                                    every directory is a row (stress test)
//!   PROBE_SCROLL_Y=<px>                                   operation::scroll_to after the scan
//!   PROBE_CANCEL_AFTER_MS=<ms>                            send CancelScan once
//!   PROBE_SCAN_VIA_SUB=1                                  scan via Subscription::run_with
//!   PROBE_THEME=light|dark|nord|system                    initial theme choice
//!   PROBE_SELECT=<row>                                    select a row after the scan
//!   PROBE_FOCUS=1                                         focus the filter text_input at boot
mod extras;
mod model;
mod platform;
mod rows;
mod scan;
mod style;
mod treemap;

use model::{Sort, Tree};
use style::ThemeChoice;

use iced::keyboard::{self, key};
use iced::widget::{
    button, canvas, center, column, container, mouse_area, opaque, operation, pick_list,
    progress_bar, row, rule, scrollable, space, stack, text,
};
use iced::{Center, Element, Fill, FillPortion, Font, Size, Subscription, Task, Theme};
use iced::{event, font, system, theme, time, window};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const FILTER_INPUT: &str = "filter-input";
pub const FIRA_SANS: &[u8] = include_bytes!("../assets/FiraSans-Regular.ttf");
const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

pub fn main() -> iced::Result {
    // Initial data for boot. BootFn is `Fn() -> State | (State, Task)`, so the closure may be
    // called more than once (presets/tests): clone captured data inside.
    let cli_root: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);

    iced::application(move || App::boot(cli_root.clone()), App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        // .window() OVERWRITES all window settings -> call it before window_size/centered/...
        .window(window::Settings {
            size: Size::new(1180.0, 760.0),
            min_size: Some(Size::new(720.0, 480.0)),
            // We want to confirm quitting while a scan runs.
            exit_on_close_request: false,
            ..window::Settings::default()
        })
        .centered()
        .antialiasing(true)
        .default_font(Font::DEFAULT)
        // Registers font bytes at startup (impl Into<Cow<'static, [u8]>>); use it via
        // Font::with_name("<family name inside the file>").
        .font(FIRA_SANS)
        .run()
}

#[derive(Debug, Default)]
struct Probe {
    screenshot: Option<PathBuf>,
    screenshot_after: Duration,
    screenshot_taken: bool,
    exit_after: Option<Duration>,
    open_modal: bool,
    open_dialog: bool,
    playground: bool,
    fonts: bool,
    subscriptions: bool,
    expand_all: bool,
    scroll_y: Option<f32>,
    cancel_after: Option<Duration>,
    scan_via_subscription: bool,
    theme: Option<ThemeChoice>,
    select: Option<usize>,
}

impl Probe {
    fn from_env() -> Self {
        let millis = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_millis)
        };
        let flag = |name: &str| std::env::var(name).is_ok_and(|value| value == "1");

        Self {
            screenshot: std::env::var_os("PROBE_SHOT").map(PathBuf::from),
            screenshot_after: millis("PROBE_SHOT_AFTER_MS").unwrap_or(Duration::from_millis(2500)),
            screenshot_taken: false,
            exit_after: millis("PROBE_EXIT_AFTER_MS"),
            open_modal: flag("PROBE_MODAL"),
            open_dialog: flag("PROBE_DIALOG"),
            playground: flag("PROBE_PLAYGROUND"),
            fonts: flag("PROBE_FONTS"),
            expand_all: flag("PROBE_EXPAND_ALL"),
            scroll_y: std::env::var("PROBE_SCROLL_Y").ok().and_then(|v| v.parse().ok()),
            cancel_after: millis("PROBE_CANCEL_AFTER_MS"),
            scan_via_subscription: flag("PROBE_SCAN_VIA_SUB"),
            select: std::env::var("PROBE_SELECT").ok().and_then(|v| v.parse().ok()),
            theme: std::env::var("PROBE_THEME").ok().and_then(|name| {
                ThemeChoice::ALL
                    .into_iter()
                    .find(|choice| choice.to_string().eq_ignore_ascii_case(&name))
            }),
            subscriptions: flag("PROBE_SUBS"),
        }
    }

    fn needs_ticks(&self) -> bool {
        (self.screenshot.is_some() && !self.screenshot_taken)
            || self.exit_after.is_some()
            || self.cancel_after.is_some()
    }
}

/// view() takes &self, so counters use Cell (interior mutability is fine here).
#[derive(Debug, Default)]
struct Stats {
    updates: u64,
    views: std::cell::Cell<u64>,
    view_time: std::cell::Cell<Duration>,
    last_report: Option<Instant>,
}

pub struct App {
    started: Instant,
    stats: Stats,
    root: Option<PathBuf>,
    tree: Tree,
    rows: Vec<model::Row>,
    expanded: HashSet<PathBuf>,
    rows_dirty: bool,
    sort: Sort,
    selected: Option<usize>,
    window: rows::Window,
    scan: Option<scan::Scan>,
    /// Variant B: scan driven by Subscription::run_with while Some.
    scan_request: Option<scan::ScanRequest>,
    next_scan_id: u64,
    progress: scan::Progress,
    tiles: Vec<treemap::Tile>,
    tile_paths: Vec<PathBuf>,
    treemap_cache: canvas::Cache,
    filter: String,
    theme_choice: ThemeChoice,
    system_mode: theme::Mode,
    show_details: bool,
    compact: bool,
    modal: Option<Modal>,
    status: String,
    spinner: usize,
    window_id: Option<window::Id>,
    probe: Probe,
}

#[derive(Debug, Clone)]
enum Modal {
    ConfirmTrash(PathBuf),
    ConfirmQuit(window::Id),
}

#[derive(Debug, Clone, Copy)]
pub enum KeyAction {
    Open,
    Find,
    Refresh,
    Escape,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub enum Message {
    PickFolder,
    FolderPicked(Option<PathBuf>),
    StartScan(PathBuf),
    Scan(scan::Event),
    CancelScan,
    Tick(Instant),
    ProbeTick(Instant),
    Scrolled(scrollable::Viewport),
    Toggle(usize),
    Select(usize),
    Context(usize),
    SortChanged(Sort),
    FilterChanged(String),
    FilterSubmitted,
    FocusFilter,
    ScrollToTop,
    ToggleDetails(bool),
    ToggleCompact(bool),
    ThemeChanged(ThemeChoice),
    SystemTheme(theme::Mode),
    TileClicked(usize),
    Reveal(PathBuf),
    Open(PathBuf),
    AskTrash(PathBuf),
    ConfirmTrash,
    CloseModal,
    Trashed(Result<PathBuf, String>),
    PlatformDone(Result<(), String>),
    Key(KeyAction),
    CloseRequested(window::Id),
    QuitConfirmed(window::Id),
    WindowOpened(window::Id),
    FileDropped(PathBuf),
    Screenshot(window::Screenshot),
    ScreenshotSaved(Result<PathBuf, String>),
    Heartbeat(u64),
    Log(String),
    Noop,
}

impl App {
    fn boot(cli_root: Option<PathBuf>) -> (Self, Task<Message>) {
        let probe = Probe::from_env();
        let initial_theme = probe.theme.unwrap_or_default();

        let mut app = Self {
            started: Instant::now(),
            stats: Stats::default(),
            root: None,
            tree: Tree::default(),
            rows: Vec::new(),
            expanded: HashSet::new(),
            rows_dirty: false,
            sort: Sort::default(),
            selected: None,
            window: rows::Window::default(),
            scan: None,
            scan_request: None,
            next_scan_id: 0,
            progress: scan::Progress::default(),
            tiles: Vec::new(),
            tile_paths: Vec::new(),
            treemap_cache: canvas::Cache::new(),
            filter: String::new(),
            theme_choice: initial_theme,
            system_mode: theme::Mode::None,
            show_details: true,
            compact: false,
            modal: None,
            status: "Ready".into(),
            spinner: 0,
            window_id: None,
            probe,
        };

        let mut tasks = vec![
            // Current OS light/dark mode (Task<theme::Mode>).
            system::theme().map(Message::SystemTheme),
            extras::task_combinators(),
            // Runtime font loading: Task<Result<(), font::Error>>
            font::load(FIRA_SANS).map(|result| Message::Log(format!("font::load -> {result:?}"))),
            // Is a window already known at boot? (logs Some/None)
            window::oldest().map(|id| Message::Log(format!("window::oldest() at boot: {id:?}"))),
        ];

        if let Some(root) = cli_root {
            tasks.push(Task::done(Message::StartScan(root)));
        }
        if app.probe.open_modal {
            app.modal = Some(Modal::ConfirmTrash(PathBuf::from(
                "/tmp/dirstat-work/probes/iced-probe/target",
            )));
        }
        if app.probe.open_dialog {
            tasks.push(Task::done(Message::PickFolder));
        }
        if std::env::var("PROBE_FOCUS").is_ok_and(|value| value == "1") {
            tasks.push(Task::done(Message::FocusFilter));
        }

        (app, Task::batch(tasks))
    }

    fn title(&self) -> String {
        match &self.root {
            Some(root) => format!("dir-stat — {}", root.display()),
            None => String::from("dir-stat"),
        }
    }

    /// `None` => iced picks Theme::Light/Theme::Dark from the OS and follows changes live.
    fn theme(&self) -> Option<Theme> {
        self.theme_choice.theme()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        self.stats.updates += 1;

        match message {
            Message::PickFolder => {
                // Build the dialog future here (main thread) -> see platform::pick_folder.
                return platform::pick_folder(
                    self.root
                        .clone()
                        .or_else(|| Some(PathBuf::from("/tmp/dirstat-work"))),
                    Message::FolderPicked,
                );
            }
            Message::FolderPicked(Some(path)) => return self.start_scan(path),
            Message::FolderPicked(None) => self.status = "Folder picker cancelled".into(),
            Message::StartScan(path) => return self.start_scan(path),
            Message::Scan(event) => return self.on_scan_event(event),
            Message::CancelScan => {
                if let Some(scan) = self.scan.take() {
                    scan.cancel(); // dropping it also aborts the Task stream
                    self.status = "Scan cancelled".into();
                    self.log("CancelScan: dropped scan handle (Task aborted)".into());
                    self.rebuild_rows();
                }
                if self.scan_request.take().is_some() {
                    // Variant B: no longer returned by subscription() => stream dropped.
                    self.status = "Scan cancelled".into();
                    self.log("CancelScan: subscription removed".into());
                    self.rebuild_rows();
                }
            }
            Message::Tick(now) => {
                self.spinner = self.spinner.wrapping_add(1);
                if self.rows_dirty {
                    self.rebuild_rows();
                }
                self.report_stats(now);
            }
            Message::ProbeTick(_now) => return self.probe_tick(),
            Message::Scrolled(viewport) => {
                self.window = rows::Window::from_viewport(&viewport);
            }
            Message::Toggle(index) => {
                if let Some(row) = self.rows.get(index) {
                    let path = row.path.clone();
                    if !self.expanded.remove(&path) {
                        let _ = self.expanded.insert(path);
                    }
                    self.rebuild_rows();
                }
            }
            Message::Select(index) => self.select(index),
            Message::Context(index) => {
                self.select(index);
                if let Some(row) = self.rows.get(index) {
                    self.status = format!("Right click on {}", row.path.display());
                }
            }
            Message::SortChanged(sort) => {
                self.sort = sort;
                self.rebuild_rows();
            }
            Message::FilterChanged(filter) => self.filter = filter,
            Message::FilterSubmitted => self.status = format!("Filter submitted: {}", self.filter),
            Message::FocusFilter => {
                // 0.14: widget operations live in iced::widget::operation; Ids are unified.
                // chain: is_focused runs after focus completed.
                return operation::focus(FILTER_INPUT).chain(
                    operation::is_focused(FILTER_INPUT)
                        .map(|focused| Message::Log(format!("filter input focused: {focused}"))),
                );
            }
            Message::ScrollToTop => {
                return operation::snap_to(rows::tree_id(), scrollable::RelativeOffset::START);
            }
            Message::ToggleDetails(value) => self.show_details = value,
            Message::ToggleCompact(value) => self.compact = value,
            Message::ThemeChanged(choice) => {
                self.theme_choice = choice;
                self.treemap_cache.clear(); // cached geometry contains theme colours
            }
            Message::SystemTheme(mode) => {
                self.system_mode = mode;
                self.treemap_cache.clear();
                self.log(format!("system theme mode: {mode:?}"));
            }
            Message::TileClicked(index) => {
                if let Some(path) = self.tile_paths.get(index).cloned() {
                    if let Some(parent) = path.parent() {
                        let _ = self.expanded.insert(parent.to_path_buf());
                    }
                    self.rebuild_rows();
                    if let Some(row_index) = self.rows.iter().position(|row| row.path == path) {
                        self.select(row_index);
                        return self.scroll_to_row(row_index);
                    }
                }
            }
            Message::Reveal(path) => return platform::reveal(path, Message::PlatformDone),
            Message::Open(path) => return platform::open(path, Message::PlatformDone),
            Message::AskTrash(path) => self.modal = Some(Modal::ConfirmTrash(path)),
            Message::ConfirmTrash => {
                if let Some(Modal::ConfirmTrash(path)) = self.modal.take() {
                    // Probe safety guard: only ever trash inside a sandbox dir.
                    let sandbox = Path::new("/tmp/dirstat-work/probes/iced-probe/trash-sandbox");
                    if path.starts_with(sandbox) {
                        return platform::move_to_trash(path, Message::Trashed);
                    }
                    self.status = format!("Refusing to trash {} (probe guard)", path.display());
                }
            }
            Message::CloseModal => self.modal = None,
            Message::Trashed(result) => self.status = format!("Trash: {result:?}"),
            Message::PlatformDone(result) => self.status = format!("Platform call: {result:?}"),
            Message::Key(action) => return self.on_key(action),
            Message::CloseRequested(id) => {
                if self.is_scanning() {
                    self.modal = Some(Modal::ConfirmQuit(id));
                } else {
                    return window::close(id);
                }
            }
            Message::QuitConfirmed(id) => {
                self.scan = None; // cancels the worker
                self.scan_request = None;
                return window::close(id); // last window closed => application exits
            }
            Message::WindowOpened(id) => {
                self.window_id = Some(id);
                self.log(format!("window opened: {id:?}"));
            }
            Message::FileDropped(path) => {
                if path.is_dir() {
                    return self.start_scan(path);
                }
            }
            Message::Screenshot(screenshot) => {
                let Some(target) = self.probe.screenshot.clone() else {
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || save_png(&screenshot, &target))
                            .await
                            .map_err(|error| error.to_string())?
                    },
                    Message::ScreenshotSaved,
                );
            }
            Message::ScreenshotSaved(result) => {
                self.log(format!("screenshot saved: {result:?}"));
            }
            Message::Heartbeat(beats) => self.log(format!("heartbeat {beats}")),
            Message::Log(line) => self.log(line),
            Message::Noop => {}
        }

        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            // Only events NOT captured by a widget (e.g. not typed into a focused text_input).
            keyboard::listen().filter_map(map_key),
            // Needs exit_on_close_request(false), otherwise the window just closes.
            window::close_requests().map(Message::CloseRequested),
            window::open_events().map(Message::WindowOpened),
            // listen_with takes a fn pointer: fn(Event, event::Status, window::Id) -> Option<M>
            event::listen_with(|event, _status, _window| match event {
                iced::Event::Window(window::Event::FileDropped(path)) => {
                    Some(Message::FileDropped(path))
                }
                _ => None,
            }),
            system::theme_changes().map(Message::SystemTheme),
            extras::subscriptions(self.probe.subscriptions),
        ];

        if let Some(request) = &self.scan_request {
            subscriptions.push(scan::subscription(request).map(Message::Scan));
        }

        // Throttled UI refresh while scanning: rows are rebuilt at most 10x per second.
        if self.is_scanning() || self.rows_dirty {
            subscriptions.push(time::every(Duration::from_millis(100)).map(Message::Tick));
        }

        if self.probe.needs_ticks() {
            subscriptions.push(time::every(Duration::from_millis(250)).map(Message::ProbeTick));
        }

        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Message> {
        let started = Instant::now();
        let element = self.view_inner();
        self.stats.views.set(self.stats.views.get() + 1);
        self.stats
            .view_time
            .set(self.stats.view_time.get() + started.elapsed());
        element
    }

    fn view_inner(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = if self.probe.fonts {
            extras::fonts()
        } else if self.probe.playground {
            extras::playground(self)
        } else if self.root.is_none() {
            self.view_start()
        } else {
            self.view_results()
        };

        let screen = column![self.view_header(), body, self.view_status()]
            .width(Fill)
            .height(Fill);

        match &self.modal {
            Some(modal) => with_modal(screen, self.view_modal(modal), Message::CloseModal),
            None => screen.into(),
        }
    }

    // ----------------------------------------------------------------------------- views

    fn view_header(&self) -> Element<'_, Message> {
        let title = text("dir-stat").size(18).font(Font {
            weight: font::Weight::Bold,
            ..Font::DEFAULT
        });

        let path = text(
            self.root
                .as_ref()
                .map_or_else(|| String::from("No folder selected"), |root| root.display().to_string()),
        )
        .size(13)
        .style(text::secondary)
        .wrapping(text::Wrapping::None);

        let actions = row![
            pick_list(&Sort::ALL[..], Some(self.sort), Message::SortChanged).text_size(13),
            pick_list(&ThemeChoice::ALL[..], Some(self.theme_choice), Message::ThemeChanged)
                .text_size(13),
            button(text("Open…").size(13))
                .on_press(Message::PickFolder)
                .style(button::secondary),
            button(text("↻ Rescan").size(13))
                .on_press_maybe(
                    self.root
                        .clone()
                        .filter(|_| !self.is_scanning())
                        .map(Message::StartScan),
                )
                .style(button::secondary),
            button(text("✕ Cancel").size(13))
                .on_press_maybe(self.is_scanning().then_some(Message::CancelScan))
                .style(button::danger),
        ]
        .spacing(8)
        .align_y(Center);

        container(
            row![title, container(path).width(Fill).clip(true), actions]
                .spacing(12)
                .align_y(Center),
        )
        .padding([8, 12])
        .width(Fill)
        .style(style::header)
        .into()
    }

    fn view_start(&self) -> Element<'_, Message> {
        let quick = |label: &'static str, path: &str| {
            button(text(label))
                .on_press(Message::StartScan(PathBuf::from(path)))
                .style(button::secondary)
                .padding([6, 12])
        };

        let card = container(
            column![
                text("Analyze disk usage").size(24),
                text("Pick a folder; sizes include everything nested below it.")
                    .style(text::secondary),
                button(text("Choose folder…").size(16))
                    .on_press(Message::PickFolder)
                    .padding([10, 18]),
                text("Quick picks").size(13).style(text::secondary),
                row![
                    quick("/usr", "/usr"),
                    quick("/Applications", "/Applications"),
                    quick("/opt/homebrew", "/opt/homebrew"),
                    quick("/tmp/dirstat-work", "/tmp/dirstat-work"),
                ]
                .spacing(8),
            ]
            .spacing(14)
            .align_x(Center),
        )
        .padding(28)
        .style(style::card);

        center(card).into()
    }

    fn view_results(&self) -> Element<'_, Message> {
        let tree = rows::view(
            &self.rows,
            self.window,
            self.selected,
            rows::Callbacks {
                on_toggle: Message::Toggle,
                on_select: Message::Select,
                on_context: Message::Context,
                on_scroll: Message::Scrolled,
            },
        );

        let treemap = canvas(treemap::Treemap {
            tiles: &self.tiles,
            cache: &self.treemap_cache,
            on_click: Message::TileClicked,
        })
        .width(Fill)
        .height(Fill);

        let details: Element<'_, Message> = match self.selected.and_then(|index| self.rows.get(index)) {
            Some(row) => container(
                column![
                    text(&row.name).size(16).font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    }),
                    text(model::format_bytes(row.bytes)).size(22).font(Font::MONOSPACE),
                    text(format!("{} files", row.files)).style(text::secondary),
                    row![
                        button(text("Show in Finder").size(13))
                            .on_press(Message::Reveal(row.path.clone()))
                            .style(button::secondary),
                        button(text("Open").size(13))
                            .on_press(Message::Open(row.path.clone()))
                            .style(button::secondary),
                        button(text("Move to Trash…").size(13))
                            .on_press(Message::AskTrash(row.path.clone()))
                            .style(button::danger),
                    ]
                    .spacing(6),
                ]
                .spacing(6),
            )
            .padding(12)
            .width(Fill)
            .style(style::card)
            .into(),
            None => container(text("Select a directory").style(text::secondary))
                .padding(12)
                .into(),
        };

        let side = column![
            container(treemap)
                .padding(2)
                .width(Fill)
                .height(Fill)
                .style(style::card),
            details
        ]
        .spacing(8)
        .padding(8)
        .width(FillPortion(2));

        row![
            container(tree).width(FillPortion(3)).height(Fill).padding(4),
            rule::vertical(1),
            side
        ]
        .height(Fill)
        .into()
    }

    fn view_status(&self) -> Element<'_, Message> {
        let progress = &self.progress;
        let summary = format!(
            "{} files · {} dirs · {} · {} errors · {:.1}s",
            progress.files,
            progress.dirs,
            model::format_bytes(progress.bytes),
            progress.errors,
            progress.elapsed.as_secs_f32()
        );

        let left: Element<'_, Message> = if self.is_scanning() {
            let current = progress
                .current
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();

            row![
                text(SPINNER[self.spinner % SPINNER.len()]).size(14),
                text("Scanning…").size(13),
                // Indeterminate "activity" bar driven by the tick counter.
                progress_bar(0.0..=1.0, ((self.spinner % 20) as f32) / 19.0)
                    .length(120)
                    .girth(6),
                text(summary).size(12).font(Font::MONOSPACE),
                container(text(current).size(12).style(text::secondary).wrapping(text::Wrapping::None))
                    .width(Fill)
                    .clip(true),
            ]
            .spacing(8)
            .align_y(Center)
            .into()
        } else {
            row![
                text(&self.status).size(12),
                space::horizontal(),
                text(summary).size(12).font(Font::MONOSPACE).style(text::secondary),
            ]
            .spacing(8)
            .align_y(Center)
            .into()
        };

        container(left)
            .padding([4, 12])
            .width(Fill)
            .style(style::header)
            .into()
    }

    fn view_modal(&self, modal: &Modal) -> Element<'_, Message> {
        let (title, body, confirm) = match modal {
            Modal::ConfirmTrash(path) => (
                "Move to Trash?",
                format!("“{}” will be moved to the Trash.", path.display()),
                Message::ConfirmTrash,
            ),
            Modal::ConfirmQuit(id) => (
                "Quit while scanning?",
                String::from("The scan in progress will be cancelled."),
                Message::QuitConfirmed(*id),
            ),
        };

        container(
            column![
                text(format!("⚠ {title}")).size(18),
                text(body),
                row![
                    space::horizontal(),
                    button("Cancel").on_press(Message::CloseModal).style(button::secondary),
                    button("Confirm").on_press(confirm).style(button::danger),
                ]
                .spacing(8),
            ]
            .spacing(14),
        )
        .padding(20)
        .width(420)
        .style(style::card)
        .into()
    }

    // --------------------------------------------------------------------------- helpers

    fn start_scan(&mut self, root: PathBuf) -> Task<Message> {
        self.tree = Tree::new(root.clone());
        self.expanded = HashSet::from([root.clone()]);
        self.rows.clear();
        self.selected = None;
        self.progress = scan::Progress::default();
        self.root = Some(root.clone());
        self.status = format!("Scanning {}", root.display());
        self.window = rows::Window::default();
        self.treemap_cache.clear();

        if self.probe.scan_via_subscription {
            // Variant B: just store the request; subscription() turns it into a stream.
            self.scan = None;
            self.next_scan_id += 1;
            self.scan_request = Some(scan::ScanRequest {
                id: self.next_scan_id,
                root,
            });
            return Task::none();
        }

        // Replacing self.scan drops (= cancels) any previous scan.
        let (scan, task) = scan::start(root);
        self.scan = Some(scan);

        Task::batch([
            task.map(Message::Scan),
            operation::scroll_to(
                rows::tree_id(),
                scrollable::AbsoluteOffset {
                    x: None,
                    y: Some(0.0),
                },
            ),
        ])
    }

    fn on_scan_event(&mut self, event: scan::Event) -> Task<Message> {
        match event {
            scan::Event::Batch(records) => {
                self.tree.apply(&records);
                self.rows_dirty = true; // rebuilt on the next Tick (throttle)
            }
            scan::Event::Progress(progress) => self.progress = progress,
            scan::Event::Done {
                progress,
                cancelled,
            } => {
                self.progress = progress;
                self.scan = None;
                self.scan_request = None;
                self.status = format!(
                    "{} {} in {:.2}s",
                    if cancelled { "Cancelled" } else { "Scanned" },
                    model::format_bytes(self.tree.total()),
                    self.progress.elapsed.as_secs_f32()
                );
                self.rebuild_rows();
                self.log(format!("{} ({} rows)", self.status, self.rows.len()));

                if let Some(index) = self.probe.select {
                    self.select(index);
                }

                if let Some(y) = self.probe.scroll_y {
                    self.log(format!("scroll_to y = {y}"));
                    return operation::scroll_to(
                        rows::tree_id(),
                        scrollable::AbsoluteOffset { x: None, y: Some(y) },
                    );
                }
            }
        }
        Task::none()
    }

    fn rebuild_rows(&mut self) {
        let selected_path = self
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| row.path.clone());

        let started = Instant::now();
        self.rows = self
            .tree
            .flatten_with(&self.expanded, self.probe.expand_all, self.sort);
        let elapsed = started.elapsed();
        if elapsed > Duration::from_millis(5) {
            self.log(format!("rebuild_rows: {} rows in {elapsed:?}", self.rows.len()));
        }
        self.rows_dirty = false;
        self.selected = selected_path
            .and_then(|path| self.rows.iter().position(|row| row.path == path));
        self.rebuild_tiles();
    }

    fn rebuild_tiles(&mut self) {
        let dir = self
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| row.path.clone())
            .or_else(|| self.tree.root().map(Path::to_path_buf));

        let Some(dir) = dir else {
            return;
        };

        let children = self.tree.children_by_size(&dir);
        self.tile_paths = children.iter().take(60).map(|(path, _)| path.clone()).collect();
        self.tiles = children
            .iter()
            .take(60)
            .map(|(path, bytes)| treemap::Tile {
                label: model::display_name(path),
                detail: model::format_bytes(*bytes),
                bytes: *bytes,
            })
            .collect();
        self.treemap_cache.clear();
    }

    fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = Some(index);
            self.rebuild_tiles();
        }
    }

    fn scroll_to_row(&self, index: usize) -> Task<Message> {
        let y = index as f32 * rows::ROW_HEIGHT;
        let top = self.window.offset_y;
        let bottom = top + self.window.height - rows::ROW_HEIGHT;

        if y < top || y > bottom {
            operation::scroll_to(
                rows::tree_id(),
                scrollable::AbsoluteOffset {
                    x: None,
                    y: Some((y - self.window.height / 2.0).max(0.0)),
                },
            )
        } else {
            Task::none()
        }
    }

    fn on_key(&mut self, action: KeyAction) -> Task<Message> {
        match action {
            KeyAction::Escape => self.modal = None,
            KeyAction::Open => return Task::done(Message::PickFolder),
            KeyAction::Find => return operation::focus(FILTER_INPUT),
            KeyAction::Refresh => {
                if let Some(root) = self.root.clone() {
                    return self.start_scan(root);
                }
            }
            KeyAction::Up | KeyAction::Down => {
                if self.rows.is_empty() {
                    return Task::none();
                }
                let current = self.selected.unwrap_or(0);
                let next = match action {
                    KeyAction::Up => current.saturating_sub(1),
                    _ => (current + 1).min(self.rows.len() - 1),
                };
                self.select(next);
                return self.scroll_to_row(next);
            }
            KeyAction::Left | KeyAction::Right => {
                if let Some(row) = self.selected.and_then(|index| self.rows.get(index)) {
                    let path = row.path.clone();
                    if matches!(action, KeyAction::Right) {
                        let _ = self.expanded.insert(path);
                    } else {
                        let _ = self.expanded.remove(&path);
                    }
                    self.rebuild_rows();
                }
            }
        }
        Task::none()
    }

    fn probe_tick(&mut self) -> Task<Message> {
        let elapsed = self.started.elapsed();

        if let Some(cancel_after) = self.probe.cancel_after
            && elapsed >= cancel_after
        {
            self.probe.cancel_after = None;
            self.log("probe: sending CancelScan".into());
            return Task::done(Message::CancelScan);
        }

        if let Some(exit_after) = self.probe.exit_after
            && elapsed >= exit_after
        {
            eprintln!("[probe] exiting after {elapsed:?}");
            return iced::exit();
        }

        if self.probe.screenshot.is_some()
            && !self.probe.screenshot_taken
            && elapsed >= self.probe.screenshot_after
        {
            self.probe.screenshot_taken = true;
            // window::latest(): Task<Option<Id>>; and_then only continues on Some.
            return window::latest()
                .and_then(window::screenshot)
                .map(Message::Screenshot);
        }

        Task::none()
    }

    fn is_scanning(&self) -> bool {
        self.scan.is_some() || self.scan_request.is_some()
    }

    fn report_stats(&mut self, now: Instant) {
        let last = *self.stats.last_report.get_or_insert(now);
        let window = now.saturating_duration_since(last);
        if window < Duration::from_secs(1) {
            return;
        }
        let views = self.stats.views.replace(0);
        let view_time = self.stats.view_time.replace(Duration::ZERO);
        let updates = std::mem::take(&mut self.stats.updates);
        self.stats.last_report = Some(now);
        self.log(format!(
            "last {:.1}s: {updates} update() calls, {views} view() calls, avg view {:?}, rows {}",
            window.as_secs_f32(),
            view_time.checked_div(views.max(1) as u32).unwrap_or_default(),
            self.rows.len()
        ));
    }

    fn log(&mut self, line: String) {
        eprintln!("[probe +{:>6} ms] {line}", self.started.elapsed().as_millis());
    }
}

/// Keyboard mapping for keyboard::listen().filter_map(..). Must be a fn item or a
/// non-capturing closure (iced asserts zero-sized at compile time).
fn map_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    let action = match key.as_ref() {
        keyboard::Key::Named(key::Named::Escape) => KeyAction::Escape,
        keyboard::Key::Named(key::Named::ArrowUp) => KeyAction::Up,
        keyboard::Key::Named(key::Named::ArrowDown) => KeyAction::Down,
        keyboard::Key::Named(key::Named::ArrowLeft) => KeyAction::Left,
        keyboard::Key::Named(key::Named::ArrowRight) => KeyAction::Right,
        // modifiers.command() == Cmd on macOS, Ctrl elsewhere
        keyboard::Key::Character("o") if modifiers.command() => KeyAction::Open,
        keyboard::Key::Character("f") if modifiers.command() => KeyAction::Find,
        keyboard::Key::Character("r") if modifiers.command() => KeyAction::Refresh,
        _ => return None,
    };

    Some(Message::Key(action))
}

/// Canonical 0.14 modal: stack + opaque backdrop (mouse_area on_press => blur) + opaque dialog.
fn with_modal<'a>(
    base: impl Into<Element<'a, Message>>,
    dialog: impl Into<Element<'a, Message>>,
    on_blur: Message,
) -> Element<'a, Message> {
    stack![
        base.into(),
        opaque(
            mouse_area(center(opaque(dialog)).style(style::backdrop)).on_press(on_blur)
        )
    ]
    .into()
}

fn save_png(screenshot: &window::Screenshot, target: &Path) -> Result<PathBuf, String> {
    let file = std::fs::File::create(target).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        screenshot.size.width,
        screenshot.size.height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&screenshot.rgba)
        .map_err(|error| error.to_string())?;
    Ok(target.to_path_buf())
}
