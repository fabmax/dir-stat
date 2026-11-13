//! Application state, messages, update, view and subscriptions.

use std::time::{Duration, Instant};

use iced::{Element, Subscription, Task, Theme, time};

use crate::Cli;
use crate::specimen;

pub struct App {
    #[cfg(feature = "screenshot")]
    script: Option<crate::script::Script>,
    /// Where the screenshot in flight goes.
    #[cfg(feature = "screenshot")]
    shot_target: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 50 ms, only while something is live (design §6.5).
    Tick(Instant),
    #[cfg(feature = "screenshot")]
    Screenshot(iced::window::Screenshot),
    #[cfg(feature = "screenshot")]
    ScreenshotSaved(Result<std::path::PathBuf, String>),
}

impl App {
    pub fn new(
        _cli: Cli,
        #[cfg(feature = "screenshot")] script: Option<crate::script::Script>,
    ) -> Self {
        Self {
            #[cfg(feature = "screenshot")]
            script,
            #[cfg(feature = "screenshot")]
            shot_target: None,
        }
    }

    pub fn title(&self) -> String {
        "dir-stat".into()
    }

    /// Always `None`: iced then uses its Light/Dark theme and follows the OS live.
    pub fn theme(&self) -> Option<Theme> {
        None
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick(now) => self.tick(now),
            #[cfg(feature = "screenshot")]
            Message::Screenshot(screenshot) => {
                let Some(target) = self.shot_target.take() else {
                    return Task::none();
                };
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::script::save_png(&screenshot, &target)
                        })
                        .await
                        .map_err(|error| error.to_string())?
                    },
                    Message::ScreenshotSaved,
                )
            }
            #[cfg(feature = "screenshot")]
            Message::ScreenshotSaved(result) => match result {
                Ok(path) => {
                    eprintln!("dir-stat: screenshot saved to {}", path.display());
                    if let Some(script) = &mut self.script {
                        script.shot_saved(Instant::now());
                    }
                    Task::none()
                }
                Err(error) => {
                    eprintln!("dir-stat: screenshot failed: {error}");
                    iced::exit()
                }
            },
        }
    }

    fn tick(&mut self, #[allow(unused_variables)] now: Instant) -> Task<Message> {
        #[cfg(feature = "screenshot")]
        if let Some(script) = &mut self.script {
            use crate::script::Due;
            match script.poll(now) {
                Due::Nothing => {}
                Due::Shot(target) => {
                    self.shot_target = Some(target);
                    return iced::window::latest()
                        .and_then(iced::window::screenshot)
                        .map(Message::Screenshot);
                }
                Due::Done => return iced::exit(),
            }
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.needs_ticks() {
            time::every(Duration::from_millis(50)).map(Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn needs_ticks(&self) -> bool {
        #[cfg(feature = "screenshot")]
        if self.script.is_some() {
            return true;
        }
        false
    }

    pub fn view(&self) -> Element<'_, Message> {
        specimen::view()
    }
}
