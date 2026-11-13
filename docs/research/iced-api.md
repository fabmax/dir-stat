# iced 0.14 API cheat sheet for dir-stat (verified)

Everything below was checked against the real sources in
`$REG = ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` and against a probe crate that
**compiles with zero warnings** (`cargo build`, `cargo clippy --all-targets`) and was **run** on
macOS 26 / Apple Silicon with rustc 1.93.1. Snippets marked ✅ compiled in the probe. Snippets
marked 🏃 were also exercised at runtime (logs/screenshots). ❌ = verified compile *failure*
(negative tests in `negative/`, run with `./run-negative.sh`).

* Probe crate: `/tmp/dirstat-work/probes/iced-probe` (copy freely)
  * `src/main.rs` app skeleton (boot/update/view/subscription/theme, modal, keyboard, close
    request, file drop, screenshots), `src/scan.rs` (thread → UI plumbing, 3 variants),
    `src/rows.rs` (virtualized tree list), `src/treemap.rs` (canvas Program),
    `src/style.rs` (themes + style fns), `src/platform.rs` (rfd/opener/trash),
    `src/extras.rs` (Task/Subscription combinators, widget playground, font probe),
    `src/model.rs` (toy tree model), `examples/font_*.rs` (font diagnostics),
    `negative/*.rs` (expected-to-fail snippets).
  * Run: `cargo run -- /usr` (CLI path is scanned at boot). Automation env vars are listed at the
    top of `src/main.rs` (`PROBE_SHOT=…png`, `PROBE_EXIT_AFTER_MS`, `PROBE_PLAYGROUND=1`,
    `PROBE_FONTS=1`, `PROBE_MODAL=1`, `PROBE_THEME=light`, `PROBE_EXPAND_ALL=1`, …).
  * Screenshots (taken with iced's own `window::screenshot`, no OS permission needed):
    `shots/01-start.png … 11-playground-focused.png`.
* Resolved versions: iced 0.14.0, iced_widget 0.14.2, iced_winit 0.14.1, iced_core/runtime/
  futures/graphics/renderer/program/wgpu 0.14.0, iced_tiny_skia 0.14.1, cosmic-text 0.15.0,
  winit 0.30.13, wgpu 27.0.1, rfd 0.17.2, opener 0.9.0, trash 5.2.9, tokio 1.54.1,
  futures 0.3.34.

---------------------------------------------------------------------------------------------

## 0. Final Cargo.toml dependency lines ✅🏃

```toml
[package]
edition = "2024"
rust-version = "1.88"          # iced 0.14.0 declares rust-version = "1.88", edition 2024

[dependencies]
iced = { version = "0.14", features = ["tokio", "canvas"] }   # add "lazy" only if you use lazy()
rfd = "0.17"
opener = { version = "0.9", features = ["reveal"] }           # `reveal` is REQUIRED for opener::reveal
trash = "5"
tokio = { version = "1", features = ["rt"] }                  # only if you call tokio::task::spawn_blocking
```

Feature notes (from `$REG/iced-0.14.0/Cargo.toml [features]`):

| feature | default? | why |
|---|---|---|
| `wgpu`, `tiny-skia` | yes | GPU renderer + software fallback (keep both) |
| `crisp` | yes | pixel snapping; it is why `button/container::Style` have a `snap` field |
| `web-colors`, `thread-pool` | yes | harmless; `thread-pool` executor is overridden by `tokio` |
| `linux-theme-detection` | yes | Linux dark-mode detection (mundy) |
| `x11`, `wayland` | yes | Linux display backends (both loaded at runtime via dlopen) |
| `tokio` | no | **needed**: executor = `tokio::runtime::Runtime` (multi-thread) and enables `iced::time::every` / `time::repeat`; lets futures in `Task::perform` call `tokio::task::spawn_blocking` |
| `smol` | no | alternative executor; also provides `time::every` (no `repeat`) |
| `canvas` | no | **needed** for `iced::widget::canvas` (treemap) |
| `lazy` | no | `iced::widget::lazy` (+ deprecated `component`) |
| `advanced` | no | custom widgets, `.class(..)` on most widgets, `iced::advanced::*` (not needed) |
| `debug` | no | devtools overlay (F12), `Message: Debug` then required |
| `image`, `svg`, `markdown`, `highlighter`, `sipper`, `fira-sans`, … | no | not needed |

`iced::time::every` does **not exist** unless `tokio` or `smol` is enabled
(`$REG/iced_futures-0.14.0/src/backend/native/thread_pool.rs`: `pub mod time {}` is empty).
Executor selection: `$REG/iced_futures-0.14.0/src/backend/default.rs` (tokio > smol > thread-pool).

---------------------------------------------------------------------------------------------

## 1. TOP GOTCHAS (read first)

1. **Bold/semibold text on macOS falls back to random fonts** 🏃 (`shots/03-fonts.png`,
   `04-fonts-fira.png`, `examples/font_matches.rs`). `Font::DEFAULT` = `Family::SansSerif`, which
   cosmic-text maps to "Open Sans" (not installed on macOS). Normal weight then falls back to
   ".SF NS" (looks right), but SF is a *variable* font registered only as weight 400, and
   cosmic-text's fallback only accepts faces with the *exact* requested weight:
   `Weight::Bold` → **Menlo Bold (monospace!)**, `Semibold`/`Medium` → Apple SD Gothic Neo.
   `Font::with_name(".SF NS")` bold has the same problem. Works: `Font::with_name("Avenir Next")`
   (400/500/600/700), `Font::with_name("Helvetica Neue")` (400/500/700, no 600), or **bundle static
   TTFs for every weight you use** (e.g. Inter-Regular + Inter-SemiBold) via `.font(bytes)` and
   use `Font { weight, ..Font::with_name("Inter") }`. Variable fonts don't help (only their default
   instance is registered). Linux (Noto Sans/DejaVu Sans) and Windows (Segoe UI) have real bold
   faces, so this is macOS-specific — but bundling gives a consistent look everywhere.
2. `Subscription::map` / `filter_map` closures **must be non-capturing** (const-asserted ZST).
   ❌ The error only appears in `cargo build`; **`cargo check` / rust-analyzer do NOT report it**
   (post-monomorphization). Use `Subscription::with(value)` or put data in `run_with` data.
3. `Subscription::run(fn() -> S)`, `run_with(data, fn(&D) -> S)`, `event::listen_with(fn(..))`
   take **function pointers**: non-capturing closures/fn items only (❌ capturing closure).
   In edition 2024 a builder `fn f(d: &D) -> impl Stream` captures `&D`'s lifetime ❌ → write
   `-> impl Stream<Item = T> + use<>` (✅ `src/extras.rs::countdown`) or return a concrete type.
4. Widget operations moved: `iced::widget::operation::{focus, focus_next, focus_previous,
   is_focused, scroll_to, snap_to, snap_to_end, scroll_by, select_all, move_cursor_to_end, …}`.
   One unified `iced::widget::Id` (`Id::new("x")`, `Id::unique()`, `From<&'static str>`,
   `From<String>`). `scrollable::Id`, `text_input::Id`, `scrollable::scroll_to`,
   `text_input::focus` no longer exist ❌.
5. Removed/renamed (all ❌ verified): `keyboard::on_key_press` → `keyboard::listen()`;
   `horizontal_space()` → `space::horizontal()`; `vertical_space()` → `space::vertical()`;
   `Space::with_width(..)` → `space().width(..)`; `horizontal_rule(h)` → `rule::horizontal(h)`;
   `vertical_rule(w)` → `rule::vertical(w)`; `Column/Row::push_maybe(opt)` → `.push(opt)`
   (`Option<impl Into<Element>>` is `Into<Element>`); `checkbox(label, checked)` →
   `checkbox(checked).label(..)`; `toggler(..)` same; `progress_bar(..).height(h)` →
   `.girth(h)` (and `.length(w)`); canvas `Text { horizontal_alignment, vertical_alignment }` →
   `align_x` / `align_y`; `iced::application(title, update, view)` →
   `iced::application(boot, update, view).title(..)`.
6. New `snap: bool` field in `button::Style`, `container::Style`, `rule::Style` → always write
   `..button::Style::default()` / `..container::Style::default()` in struct literals ❌.
7. `Padding` converts only from `u16`, `f32`, `[u16; 2]`, `[f32; 2]`, `Pixels` (❌ `[a, b, c, d]`).
   Use `Padding { top, right, bottom, left }` or `padding::top(4).left(8)` helpers.
8. **`Column::push`/`Row::push` silently drop children whose size hint has a `Fixed(0.0)` side**
   (`$REG/iced_widget-0.14.2/src/column.rs:140`, `Size::is_void`). A spacer
   `Space::new().width(0).height(500)` vanishes! Keep the unused axis `Shrink` (default).
9. A `mouse_area` wrapping a `button` never sees left presses / double clicks (button captures
   the press). Use `mouse_area(container(..))` + `hover(base, overlay)` for hover highlight
   (✅🏃 `src/rows.rs`), or detect double-click manually.
10. Following the OS light/dark theme: return **`None`** from the theme fn (iced then uses
    `Theme::Light`/`Theme::Dark` and switches live). If you return `Some(theme)`, iced calls
    `window.set_theme(Some(..))`; on macOS winit then ignores appearance changes
    (`$REG/winit-0.30.13/src/platform_impl/macos/window_delegate.rs:462`), so
    `system::theme_changes()` stops firing while a theme is forced. (Source-verified only.)
11. `canvas::Program::update` must return `Some(Action::request_redraw())` after changing
    `Self::State` without publishing a message (0.14 renders reactively). Clear `canvas::Cache`
    when data **or theme** changes. `canvas::Text` word-wraps at `max_width` and has no ellipsis;
    the tiny-skia fallback renderer does not clip canvas text (wgpu does) → truncate yourself.
12. `scrollable` is `Shrink` in the scroll axis by default → set `.height(Fill)`.
    `on_scroll` only fires when the content overflows (and also on resize/content changes).
13. No text ellipsis anywhere in 0.14 (grep-verified). Truncation = `text(..).wrapping(
    text::Wrapping::None)` inside `container(..).width(Fill).clip(true)` 🏃.
14. `.window(window::Settings {..})` **overwrites** all window settings set before it
    (`window_size`, `centered`, …); `.settings(Settings {..})` overwrites fonts/default_font/
    antialiasing. Call them first.
15. `Message` must be `Send + 'static` (application bound) and `Clone` (button, mouse_area,
    text_input, pick_list `From` impls require `Message: Clone`).
16. rfd on macOS: build the dialog future in `update` (main thread) and hand it to
    `Task::perform`; first panel creation blocks the main thread ~0.6 s; don't open it before the
    window exists (rfd then falls back to a *blocking* modal).
17. trash on macOS: default `DeleteMethod::Finder` runs AppleScript → Automation permission
    prompt + Finder sound. Use `DeleteMethod::NsFileManager` (no prompt).

---------------------------------------------------------------------------------------------

## a) Application entry ✅🏃

Source: `$REG/iced-0.14.0/src/application.rs`

```rust
// application.rs:79
pub fn application<State, Message, Theme, Renderer>(
    boot: impl BootFn<State, Message>,
    update: impl UpdateFn<State, Message>,
    view: impl for<'a> ViewFn<'a, State, Message, Theme, Renderer>,
) -> Application<impl Program<State = State, Message = Message, Theme = Theme>>
where State: 'static, Message: Send + 'static, Theme: theme::Base, Renderer: program::Renderer
```

* `BootFn`: any `Fn() -> C` where `C: IntoBoot` = `State` **or** `(State, Task<Message>)`.
  It is `Fn` (not `FnOnce`): clone captured data inside the closure. Called once per run.
* `UpdateFn`: `Fn(&mut State, Message) -> C` with `C: Into<Task<Message>>`; returning `()` works
  (`impl<T> From<()> for Task<T>`).
* `ViewFn`: `Fn(&'a State) -> W` with `W: Into<Element<'a, Message, Theme, Renderer>>`.
* Methods returning `Self`: `settings(Settings)` (replaces ALL of Settings), `antialiasing(bool)` (default **true**),
  `default_font(Font)`, `font(impl Into<Cow<'static, [u8]>>)`, `window(window::Settings)`,
  `centered()`, `exit_on_close_request(bool)`, `window_size(impl Into<Size>)` (Size: From
  `(f32, f32)`, `[f32; 2]`, `(u32, u32)`), `transparent`, `resizable`, `decorations`,
  `position(window::Position)`, `level(window::Level)`, `presets(..)`.
* Methods returning a new `Application<impl Program>` (chain in any order):
  `title(impl TitleFn)` (`&'static str` or `Fn(&State) -> String`),
  `subscription(impl Fn(&State) -> Subscription<Message>)`,
  `theme(impl ThemeFn)` (`Theme` or `Fn(&State) -> impl Into<Option<Theme>>`),
  `style(impl Fn(&State, &Theme) -> theme::Style)`, `scale_factor(impl Fn(&State) -> f32)`,
  `executor::<E>()`.
* `run(self) -> iced::Result` (`Result<(), iced::Error>`).
* `window::Settings` fields (`$REG/iced_core-0.14.0/src/window/settings.rs`): `size`
  (default 1024×768), `maximized`, `fullscreen`, `position`, `min_size: Option<Size>`,
  `max_size`, `visible`, `resizable`, `closeable`, `minimizable`, `decorations`, `transparent`,
  `blur`, `level`, `icon: Option<Icon>` (`window::icon::from_rgba(Vec<u8>, w, h)`),
  `platform_specific` (fields differ per OS → always `..Default::default()`),
  `exit_on_close_request` (default true).
* `iced::Settings` fields: `id: Option<String>`, `fonts`, `default_font`,
  `default_text_size: Pixels` (default 16), `antialiasing` (true), `vsync` (true).

```rust
// ✅🏃 src/main.rs
pub fn main() -> iced::Result {
    let cli_root: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);

    iced::application(move || App::boot(cli_root.clone()), App::update, App::view)
        .title(App::title)                       // fn title(&self) -> String
        .theme(App::theme)                       // fn theme(&self) -> Option<Theme>
        .subscription(App::subscription)         // fn subscription(&self) -> Subscription<Message>
        .window(window::Settings {               // FIRST: overwrites other window settings
            size: Size::new(1180.0, 760.0),
            min_size: Some(Size::new(720.0, 480.0)),
            exit_on_close_request: false,        // so we can confirm quitting
            ..window::Settings::default()
        })
        .centered()
        .antialiasing(true)
        .default_font(Font::DEFAULT)
        .font(FIRA_SANS)                         // &'static [u8] (include_bytes!)
        .run()
}

fn boot(cli_root: Option<PathBuf>) -> (App, Task<Message>) {
    let app = App { /* … */ };
    let mut tasks = vec![system::theme().map(Message::SystemTheme)];
    if let Some(root) = cli_root {
        tasks.push(Task::done(Message::StartScan(root)));
    }
    (app, Task::batch(tasks))
}
```

---------------------------------------------------------------------------------------------

## b) Task ✅🏃

Source: `$REG/iced_runtime-0.14.0/src/task.rs` (re-exported as `iced::Task`, `iced::task::Handle`)

```rust
pub fn none() -> Self                                                     // :31
pub fn done(value: T) -> Self where T: MaybeSend + 'static                // :39
pub fn perform<A>(future: impl Future<Output = A> + MaybeSend + 'static,
                  f: impl FnOnce(A) -> T + MaybeSend + 'static) -> Self   // :48
pub fn run<A>(stream: impl Stream<Item = A> + MaybeSend + 'static,
              f: impl Fn(A) -> T + MaybeSend + 'static) -> Self           // :61
pub fn batch(tasks: impl IntoIterator<Item = Self>) -> Self               // :90  (concurrent)
pub fn map<O>(self, f: impl FnMut(T) -> O + MaybeSend + 'static) -> Task<O>          // :112
pub fn then<O>(self, f: impl FnMut(T) -> Task<O> + MaybeSend + 'static) -> Task<O>   // :128
pub fn chain(self, task: Self) -> Self                                    // :159 (sequential)
pub fn collect(self) -> Task<Vec<T>>                                      // :179
pub fn discard<O>(self) -> Task<O>                                        // :222
pub fn abortable(self) -> (Self, Handle)                                  // :231
pub fn future(future: impl Future<Output = T> + MaybeSend + 'static) -> Self          // :258
pub fn stream(stream: impl Stream<Item = T> + MaybeSend + 'static) -> Self            // :267
// Handle (#[derive(Debug, Clone)]): abort(&self), abort_on_drop(self) -> Self, is_aborted(&self)
// impl Task<Option<T>> { and_then(Fn(T) -> Task<A>) }   impl Task<Result<T,E>> { and_then, map_err }
```
(`MaybeSend` = `Send` on native.) Also: `iced::exit() -> Task<T>` 🏃,
`window::close(id) -> Task<T>` (closing the last window exits an `application`).
`iced_runtime::task::blocking(FnOnce(mpsc::Sender<T>))` exists but is **not re-exported** by
`iced` (you would need `iced_runtime` as a direct dep) — the manual equivalent is below.

```rust
// ✅🏃 src/extras.rs::task_combinators (logs "sum = 21", "and_then: 42")
let a: Task<u32> = Task::done(1);
let b: Task<u32> = Task::perform(async { 2 }, |x| x);
let c: Task<u32> = Task::future(async { 3 });
let d: Task<u32> = Task::run(stream::iter(vec![4u32, 5]), |x| x);
let e: Task<u32> = Task::stream(stream::iter([6u32]));
let sum = Task::batch([a.chain(b), c, d, e])
    .collect()                                          // Task<Vec<u32>>
    .then(|values| Task::done(values.iter().sum::<u32>()));
let side_effect: Task<Message> = Task::done(()).discard();
let and_then = Task::done(Some(41)).and_then(|x| Task::done(x + 1));
```

Cancelling a long-running stream ✅🏃 (`src/scan.rs::start`):
```rust
let (sender, receiver) = mpsc::unbounded::<Event>();          // iced::futures::channel::mpsc
std::thread::spawn(move || worker(&root, &flag, &sender));
let (task, handle) = Task::stream(receiver).abortable();
state.scan = Some(Scan { cancel: flag_arc, _task: handle.abort_on_drop() }); // drop => abort
return task.map(Message::Scan);
```
Dropping/aborting kills the stream → the receiver is dropped → `unbounded_send` returns `Err`
in the worker; an extra `Arc<AtomicBool>` makes the thread stop immediately (🏃 measured).
Gotcha: an aborted task delivers no final message — update UI state yourself when cancelling.
Gotcha: `Task::done(x)` returned from `update` is delivered on a later loop iteration (a
`view()` happens in between), not synchronously.

---------------------------------------------------------------------------------------------

## c) Subscription, streams, events ✅🏃

Source: `$REG/iced_futures-0.14.0/src/subscription.rs`

```rust
pub fn none() -> Self                                                      // :106
pub fn run<S>(builder: fn() -> S) -> Self
    where S: Stream<Item = T> + MaybeSend + 'static, T: 'static            // :182
pub fn run_with<D, S>(data: D, builder: fn(&D) -> S) -> Self
    where D: Hash + 'static, S: Stream<Item = T> + MaybeSend + 'static     // :198
pub fn batch(subscriptions: impl IntoIterator<Item = Subscription<T>>) -> Self   // :212
pub fn with<A>(self, value: A) -> Subscription<(A, T)>
    where A: Hash + Clone + Send + Sync + 'static                          // :226
pub fn map<F, A>(self, f: F) -> Subscription<A>
    where F: Fn(T) -> A + MaybeSend + Clone + 'static                      // :282 non-capturing!
pub fn filter_map<F, A>(self, f: F) -> Subscription<A>
    where F: Fn(T) -> Option<A> + MaybeSend + Clone + 'static              // :341 non-capturing!
```
Identity = hash of (TypeId of data, data). While `subscription()` keeps returning the same
identity, the stream keeps running; when it's no longer returned, the stream is dropped (🏃
`PROBE_SCAN_VIA_SUB=1`: worker logs "receiver dropped → stopping"). Change `data` (e.g. a scan
id) to restart.

`iced::stream::channel` (`$REG/iced_futures-0.14.0/src/stream.rs:11`):
```rust
pub fn channel<T>(size: usize, f: impl AsyncFnOnce(mpsc::Sender<T>)) -> impl Stream<Item = T>
```
`size` = buffer of the internal `futures::channel::mpsc::channel`; `output.send(x).await` needs
`use iced::futures::SinkExt;` and returns `Result` (Err when the stream was dropped). Also
`stream::try_channel` (closure returns `Result<(), E>`).
```rust
// ✅🏃 src/extras.rs
fn heartbeat() -> impl Stream<Item = Message> {
    iced::stream::channel(1, async |mut output| {
        let mut beats = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            beats += 1;
            if output.send(Message::Heartbeat(beats)).await.is_err() { break; }
        }
    })
}
fn countdown(from: &u32) -> impl Stream<Item = u32> + use<> {     // `use<>` needed (edition 2024)
    stream::iter((0..=*from).rev().collect::<Vec<_>>())
}
Subscription::batch([
    Subscription::run(heartbeat),
    Subscription::run_with(3u32, countdown)
        .with("countdown")                                        // context instead of capture
        .map(|(label, n)| Message::Log(format!("{label}: {n}"))),
])
```

**Delivering messages from a `std::thread` worker — recommended pattern** ✅🏃 (`src/scan.rs`):
`futures::channel::mpsc::unbounded()`; the thread calls `sender.unbounded_send(ev)` (never
blocks, `Err` = UI side gone → stop); the UI side is `Task::stream(receiver).abortable()`
(variant A, imperative, easy cancel) or `Subscription::run_with(ScanRequest { id, root },
|req| { spawn thread; receiver })` (variant B, declarative). Variant C (`stream::channel` +
forwarding loop) also works but is more code. **Batch in the worker** (e.g. flush every
100 ms) so the UI gets ~10–20 messages/s; iced's event-loop proxy has backpressure (max 100
in-flight actions, `$REG/iced_winit-0.14.1/src/proxy.rs`). No need for `spawn_blocking`/crossbeam.

Time (tokio backend, `$REG/iced_futures-0.14.0/src/backend/native/tokio.rs:38,61`):
```rust
pub fn every(duration: Duration) -> Subscription<Instant>     // first tick after `duration`, MissedTickBehavior::Skip
pub fn repeat<F, T>(f: fn() -> F, interval: Duration) -> Subscription<T>   // tokio only
// usage ✅🏃: time::every(Duration::from_millis(100)).map(Message::Tick)
// helpers: iced::time::{Duration, Instant, milliseconds(u64), seconds(u64)}, time::now() -> Task<Instant>
```

Keyboard (`$REG/iced_futures-0.14.0/src/keyboard.rs:7`): `pub fn listen() -> Subscription<keyboard::Event>`
— only events **not captured** by a widget (`Status::Ignored`). There is no `on_key_press`.
`keyboard::Event::KeyPressed { key, modified_key, physical_key, location, modifiers, text, repeat }`
(note `repeat`; always match with `..`).
```rust
// ✅ src/main.rs
keyboard::listen().filter_map(map_key)          // fn item or non-capturing closure
fn map_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else { return None };
    let action = match key.as_ref() {                          // Key<&str>
        keyboard::Key::Named(key::Named::Escape) => KeyAction::Escape,
        keyboard::Key::Named(key::Named::ArrowDown) => KeyAction::Down,
        keyboard::Key::Character("o") if modifiers.command() => KeyAction::Open, // Cmd / Ctrl
        _ => return None,
    };
    Some(Message::Key(action))
}
```
Generic events (`$REG/iced_futures-0.14.0/src/event.rs`): `event::listen() -> Subscription<Event>`
(ignored events), `event::listen_with(f: fn(Event, event::Status, window::Id) -> Option<M>)`,
`event::listen_raw(..)` (includes redraws; beware loops).
```rust
// ✅ drag & drop a folder onto the window
event::listen_with(|event, _status, _window| match event {
    iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
    _ => None,
})
```
Window (`$REG/iced_runtime-0.14.0/src/window.rs`): `close_requests() -> Subscription<Id>` (only
useful with `exit_on_close_request(false)`; then call `window::close(id)` yourself),
`open_events() -> Subscription<Id>` 🏃 (fires once at startup: easiest way to learn the Id),
`close_events()`, `resize_events() -> Subscription<(Id, Size)>`, `events() -> Subscription<(Id,
window::Event)>`, `frames() -> Subscription<Instant>`; tasks `latest()/oldest() ->
Task<Option<Id>>`, `screenshot(Id) -> Task<Screenshot>` 🏃, `close`, `resize`, `maximize`, …
`window::Event` variants: `Opened{..}`, `Closed`, `Moved`, `Resized(Size)`, `Rescaled`,
`RedrawRequested(Instant)`, `CloseRequested`, `Focused`, `Unfocused`, `FileHovered(PathBuf)`,
`FileDropped(PathBuf)`, `FilesHoveredLeft`.

System theme (`$REG/iced_runtime-0.14.0/src/system.rs:58,63`, re-exported `iced::system`):
```rust
pub fn theme() -> Task<theme::Mode>                  // 🏃 returned Mode::Dark
pub fn theme_changes() -> Subscription<theme::Mode>  // 🏃 observed firing once at startup as well
```
`theme::Mode = None | Light | Dark`. Simplest "follow the OS": `fn theme(&self) -> Option<Theme>`
returning `None` (iced uses `<Theme as Base>::default(mode)` = `Theme::Light`/`Theme::Dark` and
updates live; env var `ICED_THEME="Tokyo Night"` overrides). See gotcha 10 for forced themes.

---------------------------------------------------------------------------------------------

## d) Widgets ✅ (🏃 = visible in screenshots)

Imports used by the probe: `use iced::widget::{button, canvas, center, checkbox, column,
container, hover, keyed_column, lazy, mouse_area, opaque, operation, pick_list, pin,
progress_bar, responsive, row, rule, scrollable, space, stack, text, text_input, toggler,
tooltip, Column, Id};` and `use iced::{Center, Element, Fill, FillPortion, Font, Length,
Shrink, Size, Subscription, Task, Theme, font, mouse};` (helpers: `$REG/iced_widget-0.14.2/src/helpers.rs`).

**Sizing**: `Length::{Fill, FillPortion(u16), Shrink, Fixed(f32)}`; `impl From<f32|u32|Pixels>
for Length` → `.width(300)`, `.width(Fill)`, `.width(FillPortion(2))`. Re-exports: `iced::{Fill,
FillPortion, Shrink, Center, Left, Right, Top, Bottom}`.

**column!/row!** (macros expand to `Column::with_children([Element::from(x), ..])`):
`column![a, b].spacing(8).padding(10).width(Fill).align_x(Center)`;
`row![a, b].spacing(8).align_y(Center).height(28)`; also `column(iter_of_elements)`,
`Column::with_capacity(n).push(x).extend(iter)`, `.max_width`, `.clip(bool)`,
`Row::wrap()`/`Column::wrap()` (flow layout) 🏃. `push(Option<T>)` for optional children.

**text** (`$REG/iced_core-0.14.0/src/widget/text.rs`): `text(impl IntoFragment)` (accepts
`&str`, `String`, `&String`, numbers, `char`, `bool`), `text!("{x} files")` macro, `value(x)`.
Methods: `.size(impl Into<Pixels>)`, `.line_height(..)`, `.font(impl Into<Font>)`,
`.font_maybe`, `.width/.height`, `.center()`, `.align_x(impl Into<text::Alignment>)`
(`text::Alignment::{Default, Left, Center, Right, Justified}`; `Center`/`Horizontal` convert),
`.align_y(alignment::Vertical)`, `.shaping(text::Shaping)`, `.wrapping(text::Wrapping::{None,
Word (default), Glyph, WordOrGlyph})`, `.color(impl Into<Color>)`, `.color_maybe`,
`.style(Fn(&Theme) -> text::Style)` (`text::Style { color: Option<Color> }`). Built-in styles:
`text::{default, base, primary, secondary, success, warning, danger}` 🏃.
Bold: `Font { weight: font::Weight::Bold, ..Font::DEFAULT }` (see gotcha 1!); monospace:
`Font::MONOSPACE` 🏃 (renders Menlo on macOS). `Font::with_name("Family")` is `const fn`.
Shaping default is `Shaping::Auto` = basic for pure ASCII, advanced (with font fallback) otherwise.

**button** (`$REG/iced_widget-0.14.2/src/button.rs`): `button(content)` + `.on_press(Message)`,
`.on_press_with(Fn() -> Message)`, `.on_press_maybe(Option<Message>)` (None = disabled; no
on_press = disabled), `.padding(..)` (default 5 vertical / 10 horizontal), `.width/.height`, `.clip(bool)`,
`.style(impl Fn(&Theme, button::Status) -> button::Style + 'a)`.
`button::Status::{Active, Hovered, Pressed, Disabled}`;
`button::Style { background: Option<Background>, text_color: Color, border: Border, shadow:
Shadow, snap: bool }` + `.with_background(..)`. Built-ins (all `fn(&Theme, Status) -> Style`):
`button::{primary (default), secondary, success, warning, danger, text, background, subtle}` 🏃.
Press fires on **release**. Closure factory pattern ✅ (`src/style.rs::tree_row`):
```rust
pub fn tree_row(selected: bool, odd: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let pair = if selected { palette.primary.weak } else { palette.background.base };
        let background = match status {
            button::Status::Hovered if !selected => palette.background.weak.color,
            button::Status::Pressed => palette.background.strong.color,
            _ => pair.color,
        };
        button::Style { background: Some(background.into()), text_color: pair.text,
                        border: border::rounded(4), ..button::Style::default() }
    }
}
```

**mouse_area** (`$REG/iced_widget-0.14.2/src/mouse_area.rs`): `mouse_area(content)` +
`.on_press(M)`, `.on_release(M)`, `.on_double_click(M)`, `.on_right_press(M)`,
`.on_right_release(M)`, `.on_middle_press/.on_middle_release(M)`,
`.on_scroll(Fn(mouse::ScrollDelta) -> M)` (captures wheel events → blocks a parent scrollable!),
`.on_enter(M)`, `.on_move(Fn(Point) -> M)` (relative position), `.on_exit(M)`,
`.interaction(mouse::Interaction::Pointer)`. Needs `Message: Clone`. See gotcha 9.
`hover(base, top)` shows `top` only while hovered — zero-message hover highlight 🏃.

**container** (`$REG/iced_widget-0.14.2/src/container.rs`): `container(content)`, `.padding`,
`.width/.height`, `.max_width/.max_height`, `.center_x(len)`, `.center_y(len)`,
`.center(len)`, `.align_left/right/top/bottom(len)`, `.align_x/.align_y`, `.clip(bool)`,
`.id(..)`, `.style(Fn(&Theme) -> container::Style)`; helpers `center(x)` (= container centred,
Fill), `center_x`, `center_y`, `right`, `bottom`, `bottom_right`, … 
`container::Style { text_color: Option<Color>, background: Option<Background>, border, shadow,
snap }` + builders `.color()`, `.background()`, `.border()`, `.shadow()`; `From<Color>`.
Built-ins: `container::{transparent, rounded_box, bordered_box, dark, primary, secondary,
success, warning, danger}` 🏃 and `container::background(impl Into<Background>) -> Style`.
`Border { color, width, radius }`, helpers `border::rounded(r)`, `border::color(c)`,
`border::width(w)`, chainable `.rounded()/.color()/.width()`; `Radius` from f32/u8/u32/i32
and `border::top_left(..)` etc. `Shadow { color, offset: Vector, blur_radius }`.

**scrollable** (`$REG/iced_widget-0.14.2/src/scrollable.rs`): `scrollable(content)`,
`.direction(scrollable::Direction::Vertical(Scrollbar::new().width(6).scroller_width(6)
.margin(2).anchor(..)))` / `Horizontal(..)` / `Both{vertical, horizontal}`, `.horizontal()`,
`.id(impl Into<Id>)`, `.width/.height` (**set Fill**), `.on_scroll(Fn(Viewport) -> M)`,
`.anchor_top/bottom/left/right()`, `.spacing(px)` (embedded scrollbar), `.auto_scroll(bool)`
(middle-click), `.style(Fn(&Theme, scrollable::Status) -> scrollable::Style)`.
`Viewport::{absolute_offset() -> AbsoluteOffset {x, y}, absolute_offset_reversed(),
relative_offset() -> RelativeOffset, bounds() -> Rectangle, content_bounds() -> Rectangle}`.
Style: `scrollable::Style { container, vertical_rail: Rail, horizontal_rail, gap:
Option<Background>, auto_scroll: AutoScroll }`, `Rail { background: Option<Background>,
border, scroller: Scroller { background: Background, border } }`, `scrollable::default(theme,
status)` as base (✅ `src/style.rs::thin_scrollbar` 🏃). Status variants carry
`is_vertical_scrollbar_hovered/dragged/disabled` flags.
Programmatic scrolling 🏃: `operation::scroll_to(id, scrollable::AbsoluteOffset { x: None, y:
Some(px) })`, `operation::snap_to(id, scrollable::RelativeOffset::START)` (`END`),
`operation::snap_to_end(id)`, `operation::scroll_by(id, AbsoluteOffset { x, y })`.

**progress_bar** (`$REG/iced_widget-0.14.2/src/progress_bar.rs`): `progress_bar(range:
RangeInclusive<f32>, value: f32)`, `.length(impl Into<Length>)` (default Fill), `.girth(..)`
(default 30 px), `.vertical()`, `.style(Fn(&Theme) -> progress_bar::Style)`;
`Style { background: Background, bar: Background, border: Border }`; built-ins
`progress_bar::{primary, secondary, success, warning, danger}` 🏃. No indeterminate mode
(probe animates the value from a tick counter 🏃).

**tooltip**: `tooltip(content, tooltip_content, tooltip::Position::{Top (default), Bottom, Left,
Right, FollowCursor})` + `.gap(px)`, `.padding(px)`, `.delay(Duration)` (new),
`.snap_within_viewport(bool)`, `.style(Fn(&Theme) -> container::Style)` (default transparent →
style the tooltip body, e.g. `container(..).style(container::rounded_box)`).

**pick_list**: `pick_list(options: L, selected: Option<V>, on_selected: impl Fn(T) -> M)` with
`T: ToString + PartialEq + Clone`, `L: Borrow<[T]>` (e.g. `&Sort::ALL[..]`, `Vec<T>`),
`V: Borrow<T>`; `.placeholder`, `.width`, `.text_size`, `.padding`, `.font`, `.menu_height`,
`.on_open/.on_close(M)`, `.style`, `.menu_style` 🏃. (`combo_box` = searchable variant.)

**text_input**: `text_input(placeholder: &str, value: &str)` + `.id(impl Into<Id>)`,
`.on_input(Fn(String) -> M)` (without it the input is disabled), `.on_input_maybe`,
`.on_submit(M)`, `.on_paste(Fn(String) -> M)`, `.secure(bool)`, `.font(Font)`, `.icon(..)`,
`.padding`, `.size`, `.width`, `.align_x`, `.style(Fn(&Theme, text_input::Status) -> Style)`
(`Status::{Active, Hovered, Focused { is_hovered }, Disabled}`; base `text_input::default`).
Focus 🏃: `operation::focus(FILTER_INPUT)` where `const FILTER_INPUT: &str = "filter-input";`
(`operation::is_focused(id) -> Task<bool>` returned `true`).

**checkbox / toggler**: `checkbox(is_checked).label("Show hidden").on_toggle(Message::X)`
(`on_toggle(F: Fn(bool) -> M)`, `.on_toggle_maybe`, `.size`, `.spacing`, `.text_size`, `.font`,
`.icon`, `.style`); `toggler(is_on).label(..).on_toggle(..)` same shape 🏃.

**rule / space**: `rule::horizontal(1)` (arg = thickness), `rule::vertical(1)`,
`.style(Fn(&Theme) -> rule::Style)`; built-ins `rule::{default, weak}`.
`space()` (zero size), `space::horizontal()` (= Fill width), `space::vertical()`,
`space().width(22)`, `space().height(px)`, `Space::new().width(..).height(..)`.

**stack / overlays**: `stack![a, b]` (later = on top), `Stack::with_children`, `.push_under`;
`opaque(x)` captures mouse presses over `x` and makes it report an interaction, which makes
the cursor "levitate" for lower stack layers (so scrolling below is blocked too);
`pin(content).x(12).y(16)` absolute positioning inside a stack 🏃 (useful for context menus at
the cursor). Canonical modal ✅🏃 (`shots/09-modal.png`):
```rust
fn with_modal<'a>(base: impl Into<Element<'a, Message>>, dialog: impl Into<Element<'a, Message>>,
                  on_blur: Message) -> Element<'a, Message> {
    stack![
        base.into(),
        opaque(mouse_area(center(opaque(dialog)).style(style::backdrop)).on_press(on_blur)),
    ]
    .into()
}
// backdrop: container::Style { background: Some(Color { a: 0.55, ..Color::BLACK }.into()), ..Default::default() }
```

**responsive**: `responsive(|size: Size| -> Element<'a, ..>)`; default `Fill × Fill`
(`.width/.height` to change); closure may borrow state 🏃.
**lazy** (feature `lazy`): `lazy(dependency: impl Hash, |dep| -> impl Into<Element<'static,
..>>)` — the view must not borrow state ❌ (`negative/neg_lazy_borrows_state.rs`).
**keyed_column**: `keyed_column(iter of (Key, Element))` with `Key: Copy + PartialEq` 🏃.
**Element**: `.map(Fn(M) -> B)`, `.explain(Color)` (layout debugging).
New in 0.14 (not needed): `table`, `grid`, `sensor` (on_show/on_hide), `float`, `pin`.

---------------------------------------------------------------------------------------------

## e) Canvas ✅🏃 (`src/treemap.rs`, `shots/05..10`)

`$REG/iced_widget-0.14.2/src/canvas/program.rs`:
```rust
pub trait Program<Message, Theme = crate::Theme, Renderer = crate::Renderer>
where Renderer: geometry::Renderer {
    type State: Default + 'static;
    fn update(&self, _state: &mut Self::State, _event: &Event, _bounds: Rectangle,
              _cursor: mouse::Cursor) -> Option<Action<Message>> { None }
    fn draw(&self, state: &Self::State, renderer: &Renderer, theme: &Theme,
            bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry<Renderer>>;
    fn mouse_interaction(&self, _state: &Self::State, _bounds: Rectangle,
                         _cursor: mouse::Cursor) -> mouse::Interaction { Default::default() }
}
```
* `canvas::Event` = `iced::Event` (`Event::Mouse(mouse::Event::CursorMoved { .. })`,
  `ButtonPressed(mouse::Button::Left)`, `WheelScrolled { delta }`, …).
* `canvas::Action` (`$REG/iced_widget-0.14.2/src/action.rs`): `Action::publish(msg)`,
  `Action::capture()`, `Action::request_redraw()`, `Action::request_redraw_at(Instant)`,
  `.and_capture()`.
* `canvas(program).width(Fill).height(Fill)` — default size is 100×100 px!
* `canvas::Cache` (`$REG/iced_graphics-0.14.0/src/geometry/cache.rs`): `Cache::new()`,
  `clear(&self)` (interior mutability; `!Sync`), `draw(&self, renderer, size, |frame| ..) ->
  Geometry`. Keep it in app state, pass `&Cache` into the program.
* `Frame` (`$REG/iced_graphics-0.14.0/src/geometry/frame.rs`): `Frame::new(renderer, size)`,
  `fill(&Path, impl Into<Fill>)` (Color: Into<Fill>), `fill_rectangle(top_left: Point, size:
  Size, impl Into<Fill>)`, `stroke(&Path, impl Into<Stroke>)`, `stroke_rectangle(..)`,
  `fill_text(impl Into<canvas::Text>)` (**drawn above all geometry of the canvas**),
  `with_clip(Rectangle, |frame| ..)` (coordinates inside stay in the parent frame's space;
  wgpu clips text, tiny-skia does not), `with_save`, `translate`, `scale`, `rotate`,
  `size/width/height/center`, `into_geometry()`.
* `Path::rectangle(top_left, size)`, `Path::rounded_rectangle(top_left, size,
  border::Radius)` (`4.0.into()`), `Path::circle(center, r)`, `Path::line(a, b)`,
  `Path::new(|builder| { builder.move_to(..); builder.line_to(..); builder.close(); })`.
* `Stroke::default().with_color(c).with_width(1.0)` (fields `style, width, line_cap,
  line_join, line_dash`).
* `canvas::Text` fields (`$REG/iced_graphics-0.14.0/src/geometry/text.rs`): `content: String,
  position: Point, max_width: f32 (default INFINITY; text WRAPS at it), color: Color (default
  BLACK!), size: Pixels (default 16), line_height: LineHeight, font: Font, align_x:
  text::Alignment, align_y: alignment::Vertical, shaping: Shaping` + `..Text::default()`.
* Cursor: `cursor.position_in(bounds) -> Option<Point>` (relative to canvas),
  `cursor.position_over(bounds)`, `cursor.is_over(bounds)`; `cursor.position()` is `None`
  when the cursor "levitates" over a higher stack layer.

```rust
// ✅🏃 condensed from src/treemap.rs
impl<Message> canvas::Program<Message> for Treemap<'_, Message> {
    type State = State;                                   // #[derive(Default)] struct State { hovered: Option<usize> }
    fn update(&self, state: &mut State, event: &Event, bounds: Rectangle,
              cursor: mouse::Cursor) -> Option<Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let hovered = cursor.position_in(bounds).and_then(|p| self.hit(bounds.size(), p));
                if hovered != state.hovered {
                    state.hovered = hovered;
                    return Some(Action::request_redraw());   // REQUIRED
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let index = self.hit(bounds.size(), cursor.position_in(bounds)?)?;
                Some(Action::publish((self.on_click)(index)).and_capture())
            }
            _ => None,
        }
    }
    fn draw(&self, state: &State, renderer: &Renderer, theme: &Theme, bounds: Rectangle,
            _cursor: mouse::Cursor) -> Vec<Geometry> {
        let tiles = self.cache.draw(renderer, bounds.size(), |frame| {
            let path = Path::rounded_rectangle(rect.position(), rect.size(), 4.0.into());
            frame.fill(&path, color);
            frame.stroke(&path, Stroke::default().with_color(bg).with_width(1.0));
            frame.with_clip(rect, |frame| frame.fill_text(Text {
                content: truncate_to_width(&label, rect.width - 12.0, 13.0),
                position: Point::new(rect.x + 6.0, rect.y + 4.0),
                color: text_color, size: Pixels(13.0), ..Text::default()
            }));
        });
        let mut overlay = Frame::new(renderer, bounds.size());   // uncached hover layer
        /* stroke hovered rect */
        vec![tiles, overlay.into_geometry()]
    }
    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.hovered.is_some() && cursor.is_over(bounds) { mouse::Interaction::Pointer }
        else { mouse::Interaction::default() }
    }
}
// in view(): canvas(Treemap { tiles: &self.tiles, cache: &self.treemap_cache,
//                              on_click: Message::TileClicked }).width(Fill).height(Fill)
```
A squarified layout (`squarify(&[u64], Rectangle) -> Vec<Rectangle>`) is in `src/treemap.rs`.

---------------------------------------------------------------------------------------------

## f) Theming ✅🏃

Source: `$REG/iced_core-0.14.0/src/theme.rs`, `.../theme/palette.rs`.
* `Theme` variants: `Light, Dark, Dracula, Nord, SolarizedLight, SolarizedDark, GruvboxLight,
  GruvboxDark, CatppuccinLatte, CatppuccinFrappe, CatppuccinMacchiato, CatppuccinMocha,
  TokyoNight, TokyoNightStorm, TokyoNightLight, KanagawaWave, KanagawaDragon, KanagawaLotus,
  Moonfly, Nightfly, Oxocarbon, Ferra, Custom(Arc<Custom>)`; `Theme::ALL`; `Display` = name.
* `theme.palette() -> Palette` (Copy): `Palette { background, text, primary, success,
  warning, danger }` (**no `secondary`** in Palette).
* `theme.extended_palette() -> &palette::Extended`: `background: Background { base, weakest,
  weaker, weak, neutral, strong, stronger, strongest }`, `primary / secondary / success /
  warning / danger: { base, weak, strong }`, `is_dark: bool`. Every entry is a
  `Pair { color: Color, text: Color }` (`text` is guaranteed readable on `color`).
* `Theme::custom(name: impl Into<Cow<'static, str>>, palette: Palette) -> Theme`
  (extended palette auto-generated), `Theme::custom_with_fn(name, palette, |p| Extended)`.
* Colour helpers: `iced::color!(0x1e1f22)`, `color!(r, g, b)`, `Color::from_rgb8`,
  `Color::from_rgba`, `.scale_alpha(f)`, `.inverse()`; `iced::theme::palette::{mix(a, b,
  factor), deviate, darken, lighten, readable(bg, text), is_dark}`.
* Built-in Theme::Dark background is `#2B2D31`; Light is white. ✅🏃 custom themes in
  `src/style.rs::{dark, light}` (`shots/10-light-selected.png`).
* Consistent styling = small `fn(&Theme) -> X::Style` / closure factories reading
  `theme.extended_palette()` (see `src/style.rs`: `card`, `header`, `backdrop`,
  `tree_row_background`, `row_hover_overlay`, `muted`, `share_bar`, `thin_scrollbar`,
  `search_input`, `tile_color`). Start from the built-in style
  (`scrollable::default(theme, status)`, `text_input::default(..)`) and tweak fields.

---------------------------------------------------------------------------------------------

## g) rfd / opener / trash ✅ (rfd also 🏃)

**rfd 0.17.2** — `rfd::AsyncFileDialog::new().set_title(..).set_directory(impl AsRef<Path>)
.set_can_create_directories(bool).pick_folder() -> impl Future<Output = Option<FileHandle>>`
(`$REG/rfd-0.17.2/src/file_dialog.rs:281`; concrete type `Pin<Box<dyn Future + Send>>`);
`FileHandle::path() -> &Path`. Also `pick_folders`, `pick_file`, sync `rfd::FileDialog`.
```rust
// ✅🏃 src/platform.rs — call from update() (main thread)
let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose a folder to analyze");
if let Some(start) = start { dialog = dialog.set_directory(start); }
let future = dialog.pick_folder();            // panel is created HERE (eagerly)
Task::perform(future, move |handle| on_picked(handle.map(|h| h.path().to_path_buf())))
```
macOS caveats (`$REG/rfd-0.17.2/src/lib.rs:79-86`, `backend/macos/modal_future.rs`): the
dialog is a sheet on the app's main window when `NSApp.isRunning() && window exists`; otherwise
it prints "running async dialog in unsupported environment" and runs a **blocking** modal (must
be on the main thread or it panics). Creating it in `update` = main thread (recommended). The
`async { AsyncFileDialog::new()….pick_folder().await }` form also compiles (✅
`pick_folder_async_block`); rfd then hops to the main thread via GCD `exec_sync`. 🏃 Opening at
boot (after the window existed) worked, no warning, app exited cleanly; panel creation cost
~0.6 s of main-thread time. Linux: xdg-desktop-portal via runtime-`dlopen("libdbus-1.so.3")`,
fallback `zenity`; no build-time system deps.

**opener 0.9.0** — `pub fn open<P: AsRef<OsStr>>(path: P) -> Result<(), OpenError>`;
`#[cfg(feature = "reveal")] pub fn reveal<P: AsRef<Path>>(path: P) -> Result<(), OpenError>`
(`$REG/opener-0.9.0/src/lib.rs:103,177`). macOS runs `open -R -- <path>` (reveal) / `open
<path>` with `Command::output()` (waits) → run via `spawn_blocking`. Linux reveal uses D-Bus
FileManager1 / portal (zbus, pure Rust), falls back to opening the parent.
`OpenError` is not `Clone` → map to `String` for messages.

**trash 5.2.9** — `pub fn delete<T: AsRef<Path>>(path: T) -> Result<(), trash::Error>`,
`delete_all(paths)`, `TrashContext::default().delete(path)` (`$REG/trash-5.2.9/src/lib.rs`).
macOS `trash::macos::DeleteMethod::{Finder (default), NsFileManager}` + trait
`trash::macos::TrashContextExtMacos { set_delete_method, delete_method }`:
Finder = `osascript "tell application Finder to delete"` → **Automation permission prompt**,
Finder sound, supports "Put Back"; NsFileManager = `trashItemAtURL`, no prompt, faster, "Put
Back" may be missing. Not run by the probe (would modify ~/.Trash).
```rust
// ✅ src/platform.rs
fn trash_path(path: &Path) -> Result<(), trash::Error> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context.delete(path)
    }
    #[cfg(not(target_os = "macos"))]
    { trash::delete(path) }
}
// wrapped: Task::perform(async move { tokio::task::spawn_blocking(move || ..).await
//                                       .map_err(|e| e.to_string())? }, on_done)
```

---------------------------------------------------------------------------------------------

## h) Performance ✅🏃

* **No virtualized list widget exists in 0.14** (`table`/`keyed_column`/`Column` build every
  child; `lazy` only caches by hash). Manual virtualization works great 🏃
  (`src/rows.rs`, `shots/07`, `08`): fixed row height, `scrollable(..).on_scroll(
  Message::Scrolled)` → store `viewport.absolute_offset().y` and `viewport.bounds().height`;
  render `space().height(first * ROW_H)`, rows `first..last` (each exactly `ROW_H` tall,
  `.height(ROW_H)`), `space().height((n - last) * ROW_H)` + overscan. Measured with **160,711
  rows**: `view()` ≈ 35 µs average; `operation::scroll_to(y = 400 000 px)` rendered the right
  slice. Before the first `on_scroll` (content not yet overflowing) assume a tall viewport.
  Messages published during a redraw (like the first `on_scroll`) are processed immediately
  before drawing (no flicker).
* **When is `view()` called?** (`$REG/iced_winit-0.14.1/src/lib.rs` ≈1123-1270) After each
  event-loop iteration that processed ≥1 message: all queued messages go through `update`
  first, then `subscription()` and `view()` run **once**. Plain hover/redraw events do not call
  `view()`. Measured while scanning: ~35 `update()`/s vs ~22–28 `view()`/s.
* Avoid redraw storms: batch in the worker (flush every ~100 ms); keep heavy derived data in
  state and rebuild it on a throttled `time::every(100ms)` tick only when dirty (probe:
  `rows_dirty` + `Message::Tick`); return `Task::none()` for no-op updates. The
  `time::every` subscription is only returned while scanning (no idle wakeups).
* Measured cost to avoid on the UI thread: re-flattening + sorting the whole tree (160 k rows,
  PathBuf clones) took up to **75 ms per rebuild** — fine for expanded-only views, too slow to
  do every 100 ms for huge trees → incremental updates or move aggregation into the worker.
* Scanner (single-thread `read_dir` + `symlink_metadata`, `st_blocks*512`, hard-link dedupe,
  no cross-device): `/System/Library` 278 k files / 160 k dirs / 15.5 GB in 36 s (debug,
  opt-level 1). Note: on the sealed macOS system volume many files report `st_blocks == 0`
  (compressed/inline) → allocated size ≠ apparent size.

---------------------------------------------------------------------------------------------

## i) Fonts & glyphs 🏃 (`shots/02-playground.png`, `03-fonts.png`, `04-fonts-fira.png`)

* Default font: `Font::DEFAULT = Font { family: SansSerif, weight: Normal, stretch: Normal,
  style: Normal }` (`$REG/iced_core-0.14.0/src/font.rs`); iced bundles only `Iced-Icons.ttf`
  (+ FiraSans-Regular with feature `fira-sans`) and loads all system fonts
  (`$REG/iced_graphics-0.14.0/src/text.rs: font_system()`). cosmic-text 0.15 maps SansSerif →
  "Open Sans", Monospace → "Noto Sans Mono", Serif → "DejaVu Serif"; missing → fallback lists
  (`$REG/cosmic-text-0.15.0/src/font/fallback/{macos,unix,windows}.rs`; macOS: ".SF NS", "Menlo",
  "Apple Color Emoji", "Geneva", "Arial Unicode MS"). Result on macOS 26: Normal = SF ✓,
  Bold = Menlo Bold ✗, Medium/Semibold = Apple SD Gothic Neo ✗ (also gives a centred "⋯" for
  "…"), Monospace = Menlo ✓. See gotcha 1 for fixes.
* Glyphs with the default `Shaping::Auto`: ▸ ▾ ● ✓ ⚠ ↻ ✕ … ◐ ↓ and colour emoji 📁 🗑 all
  render on macOS (system fallback) 🏃. With `Shaping::Basic`, ✕/📁/🗑 render as tofu — never
  force Basic on symbol text.
* Custom font 🏃: `.font(include_bytes!("../assets/FiraSans-Regular.ttf").as_slice())` at
  startup, or at runtime `iced::font::load(bytes) -> Task<Result<(), font::Error>>` (returned
  `Ok(())`); use `Font::with_name("Fira Sans")` (family name inside the file) and optionally
  `.default_font(Font::with_name(..))`. You need one static file per weight (the probe only has
  Regular, so "Fira Sans" Bold fell back to Menlo Bold).

---------------------------------------------------------------------------------------------

## j) Build ✅

* iced 0.14.0: `edition = "2024"`, `rust-version = "1.88"`; builds warning-free on rustc
  1.93.1 (the probe uses 2024 let-chains → needs ≥ 1.88 anyway).
* Clean builds on this M-series 16-core Mac (382 crates): **dev 37.5 s** (with
  `[profile.dev] opt-level = 1` and `[profile.dev.package."*"] opt-level = 2`, recommended for a
  usable debug GUI), **release 30.8 s**, release binary 12.3 MB (not stripped); incremental
  rebuild of the app crate ~2–7 s.
* macOS: no system dependencies. Linux (not verified here — only `aarch64-apple-darwin` std is
  installed): X11/Wayland/Vulkan/libdbus are all `dlopen`ed at runtime; keep code that touches
  `std::os::unix`/`trash::macos` behind `#[cfg(unix)]` / `#[cfg(target_os = "macos")]` as in
  `src/scan.rs` and `src/platform.rs`.
* `debug` feature is handy during development (`iced = { features = [..., "debug"] }`) but then
  `Message: Debug` is required.
