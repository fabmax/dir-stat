//! Visual-verification harness (feature `screenshot`, design §2.2).
//!
//! - `DIRSTAT_SCREENSHOT=<file.png>` (+ `DIRSTAT_SCREENSHOT_AFTER_MS`, default 1500): take one
//!   screenshot and exit.
//! - `DIRSTAT_SCRIPT="path:/tmp;wait:1500;key:Stop;shot:a.png"`: run the steps, then exit.
//!   `path:<dir>` scans a folder, `key:<KeyAction>` presses a shortcut by its action name,
//!   `expand:<rel>` toggles a folder given relative to the scan root.
//!
//! `ICED_THEME=Light|Dark` is read by iced itself and forces the theme.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::keys::KeyAction;

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Wait(Duration),
    Shot(PathBuf),
    Path(PathBuf),
    Key(KeyAction),
    /// Toggle a folder, by path relative to the scan root.
    Expand(PathBuf),
    /// Press a named target: `errors`, `fda-not-now`.
    Click(String),
}

/// What the app should do on this tick.
#[derive(Debug, PartialEq)]
pub enum Due {
    Nothing,
    Shot(PathBuf),
    Path(PathBuf),
    Key(KeyAction),
    Expand(PathBuf),
    Click(String),
    Done,
}

#[derive(Debug, Clone)]
pub struct Script {
    steps: VecDeque<Step>,
    ready_at: Instant,
    awaiting_shot: bool,
}

impl Script {
    pub fn new(steps: Vec<Step>, now: Instant) -> Self {
        Self {
            steps: steps.into(),
            ready_at: now,
            awaiting_shot: false,
        }
    }

    /// Reads the harness variables. `Ok(None)` when neither is set.
    pub fn from_env(now: Instant) -> Result<Option<Self>, String> {
        let mut steps = match std::env::var("DIRSTAT_SCRIPT") {
            Ok(spec) => parse(&spec)?,
            Err(_) => Vec::new(),
        };
        if let Some(target) = std::env::var_os("DIRSTAT_SCREENSHOT") {
            let after = match std::env::var("DIRSTAT_SCREENSHOT_AFTER_MS") {
                Ok(ms) => parse_ms(&ms)?,
                Err(_) => Duration::from_millis(1500),
            };
            steps.extend([Step::Wait(after), Step::Shot(target.into())]);
        }
        Ok((!steps.is_empty()).then(|| Self::new(steps, now)))
    }

    pub fn poll(&mut self, now: Instant) -> Due {
        if self.awaiting_shot {
            return Due::Nothing;
        }
        while now >= self.ready_at {
            match self.steps.pop_front() {
                None => return Due::Done,
                Some(Step::Wait(duration)) => self.ready_at = now + duration,
                Some(Step::Shot(target)) => {
                    self.awaiting_shot = true;
                    return Due::Shot(target);
                }
                Some(Step::Path(path)) => return Due::Path(path),
                Some(Step::Key(action)) => return Due::Key(action),
                Some(Step::Expand(rel)) => return Due::Expand(rel),
                Some(Step::Click(name)) => return Due::Click(name),
            }
        }
        Due::Nothing
    }

    /// The pending screenshot was written; the next step starts counting from `now`.
    pub fn shot_saved(&mut self, now: Instant) {
        self.awaiting_shot = false;
        self.ready_at = now;
    }
}

pub fn parse(spec: &str) -> Result<Vec<Step>, String> {
    spec.split(';')
        .map(str::trim)
        .filter(|step| !step.is_empty())
        .map(|step| match step.split_once(':') {
            Some(("wait", ms)) => parse_ms(ms).map(Step::Wait),
            Some(("shot", path)) if !path.is_empty() => Ok(Step::Shot(path.into())),
            Some(("path", path)) if !path.is_empty() => Ok(Step::Path(path.into())),
            Some(("expand", rel)) if !rel.is_empty() => Ok(Step::Expand(rel.into())),
            Some(("click", name)) if !name.is_empty() => Ok(Step::Click(name.into())),
            Some(("key", name)) => KeyAction::from_name(name)
                .map(Step::Key)
                .ok_or_else(|| format!("unknown key action {name:?}")),
            _ => Err(format!(
                "unsupported script step {step:?} (supported: wait:<ms>, shot:<file.png>, path:<dir>, key:<action>, expand:<rel>, click:<target>)"
            )),
        })
        .collect()
}

fn parse_ms(ms: &str) -> Result<Duration, String> {
    ms.trim()
        .parse()
        .map(Duration::from_millis)
        .map_err(|_| format!("not a number of milliseconds: {ms:?}"))
}

/// Writes an RGBA window screenshot as PNG. Runs on a blocking thread.
pub fn save_png(
    screenshot: &iced::window::Screenshot,
    target: &PathBuf,
) -> Result<PathBuf, String> {
    let file = std::fs::File::create(target).map_err(|e| format!("{}: {e}", target.display()))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        screenshot.size.width,
        screenshot.size.height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&screenshot.rgba)
        .map_err(|e| e.to_string())?;
    Ok(target.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn parses_steps() {
        assert_eq!(
            parse("wait:1500; shot:a.png;;wait:0").unwrap(),
            vec![
                Step::Wait(1500 * MS),
                Step::Shot("a.png".into()),
                Step::Wait(Duration::ZERO)
            ]
        );
        assert_eq!(
            parse("path:/tmp;key:Stop").unwrap(),
            vec![Step::Path("/tmp".into()), Step::Key(KeyAction::Stop)]
        );
        assert!(parse("key:Sideways").is_err());
        assert_eq!(
            parse("click:errors").unwrap(),
            vec![Step::Click("errors".into())]
        );
        assert!(parse("wait:soon").is_err());
        assert!(parse("shot:").is_err());
    }

    #[test]
    fn runs_steps_in_order() {
        let start = Instant::now();
        let steps = vec![
            Step::Wait(100 * MS),
            Step::Shot("a.png".into()),
            Step::Shot("b.png".into()),
        ];
        let mut script = Script::new(steps, start);

        assert_eq!(script.poll(start), Due::Nothing);
        assert_eq!(script.poll(start + 99 * MS), Due::Nothing);
        assert_eq!(script.poll(start + 100 * MS), Due::Shot("a.png".into()));
        // Nothing happens until the screenshot is saved.
        assert_eq!(script.poll(start + 500 * MS), Due::Nothing);
        script.shot_saved(start + 600 * MS);
        assert_eq!(script.poll(start + 600 * MS), Due::Shot("b.png".into()));
        script.shot_saved(start + 700 * MS);
        assert_eq!(script.poll(start + 700 * MS), Due::Done);
    }
}
