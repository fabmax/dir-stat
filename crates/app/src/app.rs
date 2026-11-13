//! Application state, messages, update dispatch, subscriptions and the top-level views.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dirstat_core::catalog::{Engine, Resolver};
use dirstat_core::git::{self, GitBinary, GitStatus};
use dirstat_core::platform::{self, Mount};
use dirstat_core::safety::Guards;
use dirstat_core::scan::{Options, ScanRequest, StartError, Update};
use dirstat_core::smallfiles::{self, SmallFile};
use dirstat_core::trash::{SystemTrasher, TrashError, TrashJobId, TrashOutcome, Trasher};
use dirstat_core::tree::{CandidateRef, NodeId};
use dirstat_core::volumes::{self, CardKind, VolumeCard};
use iced::widget::{
    button, center, column, container, mouse_area, operation, row, scrollable, space, stack,
};
use iced::{
    Center, Element, Fill, Size, Subscription, Task, Theme, event, keyboard, mouse, time, window,
};

use crate::keys::{self, KeyAction};
use crate::prefs::{JsonPrefs, Prefs, PrefsStore};
use crate::results::order::Column;
use crate::results::rows::RowKey;
use crate::results::scan_task::{self, ScanIdGen};
use crate::results::trash::{self, Focus, Modal, TrashTarget};
use crate::results::tree_view::Effect;
use crate::results::{
    Results, ScanOutcome, StopOutcome, activity, columns, errors, nav, selection_bar,
};
use crate::start::{self, StartState};
use crate::{Cli, theme};

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
    pub home: Option<PathBuf>,
    /// macOS: the terminal dir-stat was started from (it holds the privacy grants).
    pub terminal: Option<String>,
    pub guards: Arc<Guards>,
    pub trasher: Arc<dyn Trasher>,
    /// `None`: no usable git; candidates in work trees read "Couldn't check with git".
    pub git: Option<GitBinary>,
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
    /// From any mouse event, also ones rows capture (calm sorting waits while held).
    left_button_held: bool,
    pointer_in_tree: bool,
    toast: Option<Toast>,
    /// "Not Now" hides the Full Disk Access bar for the rest of the session.
    fda_dismissed: bool,
    modal: Option<Modal>,
    next_trash_job: u64,
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
    // tree
    TreeScrolled(scrollable::Viewport),
    RowPressed(RowKey),
    /// Double-click: zoom into a folder, reveal a file, open the aggregate.
    RowActivated(RowKey),
    ZoomTo(NodeId),
    SmallFilesListed(scan_task::ScanId, NodeId, Result<Vec<SmallFile>, String>),
    ToggleExpand(RowKey),
    SortBy(Column),
    /// Calm sorting inputs (design §6.7).
    PointerIn(bool),
    PointerMoved,
    LeftButton(bool),
    // selection bar
    RevealSelection,
    CopySelection,
    ScanSelection,
    // trash
    RequestTrash,
    ConfirmTrash,
    CancelModal,
    TrashFinished(TrashJobId, u32, Vec<TrashOutcome>),
    GitChecked(scan_task::ScanId, Vec<(CandidateRef, GitStatus)>),
    // errors
    ToggleErrorsPopover,
    CopyErrorList,
    RevealInTree(NodeId),
    DismissFdaBar,
    // chrome
    DismissToast,
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
            home: platform::home_dir(),
            // The bundle id is set once dir-stat ships as an app bundle (M11).
            terminal: platform::launched_from_terminal(None),
            guards: Arc::new(Guards::new()),
            trasher: Arc::new(SystemTrasher),
            git: git::find_git(),
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
            left_button_held: false,
            pointer_in_tree: false,
            toast: None,
            fda_dismissed: false,
            modal: None,
            next_trash_job: 0,
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
            Screen::Results(r) => {
                let zoom = r.view.zoom;
                let name = if zoom == NodeId::ROOT || !r.tree.is_started() {
                    r.title.clone()
                } else {
                    r.tree.node(zoom).name().to_string_lossy().into_owned()
                };
                format!("{name} — dir-stat")
            }
            Screen::Start(_) => "dir-stat".into(),
        }
    }

    /// Always `None`: iced then uses its Light/Dark theme and follows the OS live.
    pub fn theme(&self) -> Option<Theme> {
        None
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        // Rows that moved under the anchor scroll back in the same update: no flicker.
        if let Screen::Results(results) = &mut self.screen
            && let Some(y) = results.scroll_to.take()
        {
            let scroll = operation::scroll_to(
                columns::tree_id(),
                scrollable::AbsoluteOffset {
                    x: None,
                    y: Some(y),
                },
            );
            return Task::batch([task, scroll]);
        }
        task
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
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
            Message::Tick(now) => {
                let git = self.schedule_git();
                return Task::batch([git, self.tick(now)]);
            }
            Message::TreeScrolled(viewport) => {
                if let Screen::Results(r) = &mut self.screen {
                    r.view.scrolled(
                        viewport.absolute_offset().y,
                        viewport.bounds().height,
                        Instant::now(),
                    );
                }
            }
            Message::RowPressed(key) => {
                if let Screen::Results(r) = &mut self.screen {
                    let effect = r.press(key, Instant::now());
                    return self.apply(effect);
                }
            }
            Message::RowActivated(key) => {
                if let Screen::Results(r) = &mut self.screen {
                    let effect = r.activate(key, Instant::now());
                    return self.apply(effect);
                }
            }
            Message::ToggleExpand(key) => {
                if let Screen::Results(r) = &mut self.screen {
                    let effect = r.toggle(key, Instant::now());
                    return self.apply(effect);
                }
            }
            Message::ZoomTo(id) => {
                if let Screen::Results(r) = &mut self.screen {
                    r.zoom_to(id, Instant::now());
                }
            }
            Message::SmallFilesListed(id, dir, result) => {
                if let Screen::Results(r) = &mut self.screen
                    && r.scan_id == id
                {
                    r.small_files_listed(dir, result, Instant::now());
                }
            }
            Message::RevealSelection | Message::CopySelection => {
                if let Screen::Results(r) = &self.screen
                    && let Some(key) = r.view.selection
                {
                    let reveal = matches!(message, Message::RevealSelection);
                    let effect = Effect {
                        reveal: reveal.then_some(key),
                        copy: (!reveal).then_some(key),
                        ..Effect::default()
                    };
                    return self.apply(effect);
                }
            }
            Message::ScanSelection => {
                if let Screen::Results(r) = &self.screen
                    && let Some(key @ RowKey::Dir(_)) = r.view.selection
                    && let Some(path) = r.path_for(key)
                {
                    return self.start_scan(path);
                }
            }
            Message::DismissToast => self.toast = None,
            Message::ToggleErrorsPopover => {
                if let Screen::Results(r) = &mut self.screen {
                    r.errors_popover = !r.errors_popover && !r.tree.errors().is_empty();
                }
            }
            Message::CopyErrorList => {
                if let Screen::Results(r) = &self.screen {
                    let list = errors::tsv(&r.tree);
                    self.toast = Some(Toast::info("Copied the list"));
                    return iced::clipboard::write(list);
                }
            }
            Message::RevealInTree(id) => {
                if let Screen::Results(r) = &mut self.screen {
                    r.reveal_in_tree(id, Instant::now());
                }
            }
            Message::DismissFdaBar => self.fda_dismissed = true,
            Message::RequestTrash => self.request_trash(),
            Message::ConfirmTrash => return self.confirm_trash(),
            Message::CancelModal => self.modal = None,
            Message::TrashFinished(id, generation, outcomes) => {
                if let Screen::Results(r) = &mut self.screen
                    && let Some((text, sticky)) = r.trash_finished(
                        id,
                        generation,
                        outcomes,
                        self.env.home.as_deref(),
                        Instant::now(),
                    )
                {
                    self.toast = Some(if sticky {
                        Toast::error(text)
                    } else {
                        Toast::info(text)
                    });
                }
            }
            Message::GitChecked(id, statuses) => {
                if let Screen::Results(r) = &mut self.screen
                    && r.scan_id == id
                {
                    r.candidates.finished(statuses);
                }
            }
            Message::SortBy(column) => {
                if let Screen::Results(r) = &mut self.screen {
                    r.sort_by(column, Instant::now());
                }
            }
            Message::PointerIn(inside) => self.pointer_in_tree = inside,
            Message::PointerMoved => {
                if let Screen::Results(r) = &mut self.screen {
                    r.view.reorder.activity(Instant::now());
                }
            }
            Message::LeftButton(held) => self.left_button_held = held,
            Message::Key(action) => return self.key(action),
            Message::WindowResized(size) => {
                self.window_size = size;
                if let Screen::Results(r) = &mut self.screen {
                    r.view.set_height(tree_height(size));
                }
            }
            Message::PlatformResult(result) => {
                if let Err(error) = result {
                    self.toast = Some(Toast::error(error));
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
        let mut results = Results::new(path, title, estimate, active);
        results.view.set_height(tree_height(self.window_size));
        self.screen = Screen::Results(Box::new(results));
        Task::batch([
            task,
            operation::scroll_to(
                columns::tree_id(),
                scrollable::AbsoluteOffset {
                    x: None,
                    y: Some(0.0),
                },
            ),
        ])
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
        if let Screen::Results(r) = &mut self.screen {
            r.view.reorder.activity(Instant::now());
        }
        // A dialog takes every key (UX §9): Esc cancels, Return picks the default button,
        // Tab moves the focus, Space presses the focused button.
        if let Some(Modal::Confirm { plan, focus }) = &mut self.modal {
            return match action {
                KeyAction::Escape => self.update(Message::CancelModal),
                KeyAction::Enter if plan.default_is_trash => self.update(Message::ConfirmTrash),
                KeyAction::Enter => self.update(Message::CancelModal),
                KeyAction::Tab | KeyAction::ShiftTab => {
                    *focus = if *focus == Focus::Cancel {
                        Focus::Trash
                    } else {
                        Focus::Cancel
                    };
                    Task::none()
                }
                KeyAction::Space if *focus == Focus::Trash => self.update(Message::ConfirmTrash),
                KeyAction::Space => self.update(Message::CancelModal),
                _ => Task::none(),
            };
        }
        if let Some(Modal::Refuse { .. }) = &self.modal {
            if matches!(
                action,
                KeyAction::Escape | KeyAction::Enter | KeyAction::Space
            ) {
                self.modal = None;
            }
            return Task::none();
        }
        if let Screen::Results(r) = &mut self.screen
            && r.errors_popover
        {
            // An open popover takes Esc; other keys still work on the tree.
            if action == KeyAction::Escape {
                r.errors_popover = false;
                return Task::none();
            }
        }
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
            (Screen::Results(_), KeyAction::Trash) => self.request_trash(),
            (Screen::Results(r), action) => {
                let effect = r.key(action, Instant::now());
                return self.apply(effect);
            }
        }
        Task::none()
    }

    fn tick(&mut self, now: Instant) -> Task<Message> {
        let button_held = self.left_button_held || self.modal.is_some();
        if let Screen::Results(results) = &mut self.screen {
            results.tick(now, button_held);
        }
        if self.toast.as_ref().is_some_and(|t| t.expired(now)) {
            self.toast = None;
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
                Due::Select(rel) => {
                    if let Screen::Results(r) = &mut self.screen
                        && let Some(id) = r.tree.find_path(&r.tree.root_path().join(&rel))
                    {
                        r.reveal_in_tree(id, now);
                    }
                }
                Due::Click(name) => {
                    let message = match name.as_str() {
                        "errors" => Some(Message::ToggleErrorsPopover),
                        "fda-not-now" => Some(Message::DismissFdaBar),
                        _ => None,
                    };
                    if let Some(message) = message {
                        return self.update(message);
                    }
                }
                Due::Expand(rel) => {
                    if let Screen::Results(r) = &mut self.screen
                        && let Some(id) = r.tree.find_path(&r.tree.root_path().join(&rel))
                    {
                        r.toggle(RowKey::Dir(id), now);
                    }
                }
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
        // Pointer moves only matter while a re-order can happen: no message per move otherwise.
        if self.pointer_in_tree && matches!(&self.screen, Screen::Results(r) if r.is_scanning()) {
            subscriptions.push(event::listen_with(pointer_moved));
        }
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
        let toast_expires = self.toast.as_ref().is_some_and(|t| !t.sticky);
        toast_expires || matches!(&self.screen, Screen::Results(r) if r.busy())
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
        let toast = self.toast.as_ref().map(|t| {
            let mut content = row![theme::BODY.text(t.text.clone())]
                .spacing(12)
                .align_y(Center);
            if t.sticky {
                content = content.push(
                    button(theme::BODY.text("×"))
                        .padding([0, 6])
                        .style(theme::icon_button)
                        .on_press(Message::DismissToast),
                );
            }
            let bottom = if matches!(self.screen, Screen::Results(_)) {
                52.0
            } else {
                24.0
            };
            container(container(content).padding([10, 16]).style(theme::toast))
                .width(Fill)
                .height(Fill)
                .align_x(Center)
                .align_bottom(Fill)
                .padding(iced::Padding {
                    bottom,
                    ..iced::Padding::ZERO
                })
        });
        let popover = match &self.screen {
            Screen::Results(r) if r.errors_popover => {
                // Anchored below the activity field; a click outside closes it.
                let card = container(errors::popover(r, self.env.home.as_deref()))
                    .width(Fill)
                    .align_x(Center)
                    .padding(iced::Padding {
                        top: 48.0,
                        ..iced::Padding::ZERO
                    });
                Some(stack![
                    mouse_area(container(space()).width(Fill).height(Fill))
                        .on_press(Message::ToggleErrorsPopover),
                    iced::widget::opaque(card),
                ])
            }
            _ => None,
        };
        let modal = self
            .modal
            .as_ref()
            .map(|m| trash::modal_view(m, self.env.home.as_deref()));
        stack![base]
            .push(popover)
            .push(overlay)
            .push(toast)
            .push(modal)
            .into()
    }

    /// Plans the selected row and opens the confirmation (or refusal) dialog. Nothing moves
    /// before ConfirmTrash.
    fn request_trash(&mut self) {
        let Screen::Results(r) = &self.screen else {
            return;
        };
        let target = match r.view.selection {
            Some(RowKey::Dir(id)) => TrashTarget::Dir(id),
            Some(RowKey::File(dir, index)) => TrashTarget::File(dir, index),
            Some(RowKey::SmallFile(dir, index)) => TrashTarget::SmallFile(dir, index),
            Some(_) => {
                self.toast = Some(Toast::info(
                    "Select a folder or file to move it to the Trash.",
                ));
                return;
            }
            None => return,
        };
        let planned = trash::plan(
            r,
            &self.env.engine,
            &self.env.guards,
            target,
            |c| r.candidates.state(c),
            std::time::SystemTime::now(),
        );
        self.modal = Some(match planned {
            Ok(plan) => {
                let focus = if plan.default_is_trash {
                    Focus::Trash
                } else {
                    Focus::Cancel
                };
                Modal::Confirm {
                    plan: Box::new(plan),
                    focus,
                }
            }
            Err(refused) => Modal::Refuse {
                title: refused.title,
                message: refused.message,
            },
        });
    }

    /// Starts the next git check. At most one runs; it batches a whole work tree.
    fn schedule_git(&mut self) -> Task<Message> {
        let Screen::Results(results) = &mut self.screen else {
            return Task::none();
        };
        let Some(job) = results.candidates.next_job() else {
            return Task::none();
        };
        let id = results.scan_id;
        match self.env.git.clone() {
            Some(git) => blocking(
                move || Ok(git::verify(&git, &job)),
                move |r: Result<_, String>| Message::GitChecked(id, r.unwrap_or_default()),
            ),
            None => Task::done(Message::GitChecked(
                id,
                job.items
                    .iter()
                    .map(|&(c, _)| (c, GitStatus::Unavailable))
                    .collect(),
            )),
        }
    }

    /// The only place a `TrashJob` is created: the confirmed dialog.
    fn confirm_trash(&mut self) -> Task<Message> {
        let Some(Modal::Confirm { plan, .. }) = self.modal.take() else {
            return Task::none();
        };
        let Screen::Results(r) = &mut self.screen else {
            return Task::none();
        };
        self.next_trash_job += 1;
        let job = r.start_trash(&plan, TrashJobId(self.next_trash_job));
        let (guards, trasher) = (Arc::clone(&self.env.guards), Arc::clone(&self.env.trasher));
        let (id, generation) = (job.id, job.tree_gen);
        blocking(
            move || Ok(dirstat_core::trash::execute(&job, &guards, &*trasher)),
            move |result: Result<Vec<TrashOutcome>, String>| {
                let outcomes = result.unwrap_or_else(|e| {
                    vec![TrashOutcome::Failed(Default::default(), TrashError::Os(e))]
                });
                Message::TrashFinished(id, generation, outcomes)
            },
        )
    }

    /// Carries out what a tree action asked for: listings, reveals, copies.
    fn apply(&mut self, effect: Effect) -> Task<Message> {
        let Screen::Results(r) = &self.screen else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        if let Some(dir) = effect.list_small {
            let (id, path) = (r.scan_id, r.tree.path_of(dir));
            tasks.push(blocking(
                move || smallfiles::list(&path),
                move |result| Message::SmallFilesListed(id, dir, result),
            ));
        }
        if let Some(path) = effect.reveal.and_then(|key| r.path_for(key)) {
            tasks.push(blocking(
                move || opener::reveal(&path).map_err(|e| format!("Couldn't reveal: {e}")),
                Message::PlatformResult,
            ));
        }
        if let Some(path) = effect.copy.and_then(|key| r.path_for(key)) {
            tasks.push(iced::clipboard::write(path.display().to_string()));
            self.toast = Some(Toast::info("Copied path"));
        }
        Task::batch(tasks)
    }

    fn results_view<'a>(&'a self, results: &'a Results) -> Element<'a, Message> {
        let width = self.window_size.width;
        let field_width = (width * 0.5).clamp(360.0, 640.0);
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
        let rule = || {
            container(space())
                .width(Fill)
                .height(1)
                .style(theme::hairline)
        };
        let home = self.env.home.as_deref();
        column![
            toolbar,
            rule(),
            nav::view(results, width, home),
            rule(),
            columns::view(results, Instant::now()),
        ]
        .push(
            (results.fda_bar && !self.fda_dismissed)
                .then(|| errors::fda_bar(self.env.terminal.as_deref())),
        )
        .push(rule())
        .push(selection_bar::view(results, width, home, unix_now()))
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
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
            Some(Message::LeftButton(true))
        }
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::LeftButton(false))
        }
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Back)) => {
            Some(Message::Key(KeyAction::Back))
        }
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Forward)) => {
            Some(Message::Key(KeyAction::Forward))
        }
        _ => None,
    }
}

fn pointer_moved(
    event: iced::Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    matches!(event, iced::Event::Mouse(mouse::Event::CursorMoved { .. }))
        .then_some(Message::PointerMoved)
}

/// The tree's viewport: the window minus toolbar, path bar, rules and the column header.
fn tree_height(window: Size) -> f32 {
    window.height - 44.0 - 1.0 - 32.0 - 1.0 - columns::HEADER_H
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

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    pub fn fda_bar_visible(&self) -> bool {
        matches!(&self.screen, Screen::Results(r) if r.fda_bar) && !self.fda_dismissed
    }

    pub fn toast_text(&self) -> Option<&str> {
        self.toast.as_ref().map(|t| t.text.as_str())
    }
}

/// A message over the selection bar. Info toasts expire after 4 s; errors stay until closed.
pub struct Toast {
    pub text: String,
    pub sticky: bool,
    shown_at: Instant,
}

impl Toast {
    pub fn info(text: impl Into<String>) -> Toast {
        Toast {
            text: text.into(),
            sticky: false,
            shown_at: Instant::now(),
        }
    }

    pub fn error(text: impl Into<String>) -> Toast {
        Toast {
            text: text.into(),
            sticky: true,
            shown_at: Instant::now(),
        }
    }

    fn expired(&self, now: Instant) -> bool {
        !self.sticky && now.saturating_duration_since(self.shown_at) >= Duration::from_secs(4)
    }
}
