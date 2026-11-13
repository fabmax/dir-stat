//! App-level tests without rendering (design §10.3): scans are simulated by feeding
//! `Message::Scan` updates by hand; returned tasks are ignored.

use std::io::ErrorKind;
use std::sync::{Arc, Mutex, OnceLock};

use dirstat_core::catalog::{Engine, Resolver};
use dirstat_core::scan::{Batch, Progress, StartError, Summary, Update};
use dirstat_core::tree::NodeId;
use dirstat_core::tree::testing::{TreeBuilder, dir};

use crate::Cli;
use crate::app::{App, Env, Message, Screen};
use crate::prefs::{MemoryPrefs, Prefs};
use crate::results::{Phase, Results};

fn engine() -> Arc<Engine> {
    static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| Arc::new(Engine::new(&Resolver::from_env())))
        .clone()
}

fn app() -> App {
    let env = Env {
        engine: engine(),
        prefs_store: Box::new(MemoryPrefs(Mutex::new(Prefs::default()))),
        cards: Vec::new(),
        mounts: Vec::new(),
        fda: None,
        system_name: "Test Disk".into(),
    };
    App::new(env, Cli::default()).0
}

fn sample() -> TreeBuilder {
    TreeBuilder::new(
        "/r",
        dir("r")
            .small_files(2, 8000)
            .child(dir("a").child(dir("deep")))
            .child(dir("b")),
    )
}

fn batch(listings: Vec<dirstat_core::scan::DirListing>) -> Update {
    Update::Batch(Batch {
        listings,
        progress: Progress::default(),
    })
}

fn results(app: &App) -> &Results {
    match &app.screen {
        Screen::Results(r) => r,
        Screen::Start(_) => panic!("expected the Results screen"),
    }
}

#[test]
fn stop_is_final_even_if_finished_never_arrives() {
    let mut app = app();
    let builder = sample();
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let mut listings = builder.listings();
    let late = listings.split_off(2);
    let _ = app.update(Message::Scan(id, batch(listings)));
    assert_eq!(results(&app).phase, Phase::Scanning);

    let _ = app.update(Message::Stop);
    let r = results(&app);
    assert_eq!(r.phase, Phase::Stopped);
    assert!(r.tree.is_stopped());
    assert!(r.tree.is_lower_bound(NodeId::ROOT), "partial sizes read ≥");
    assert!(
        !r.is_scanning() && r.can_rescan(),
        "Rescan works without Finished"
    );
    assert!(r.draining.is_some());

    // Late batches of the draining scan still add data; its Finished doesn't change the phase.
    let _ = app.update(Message::Scan(id, batch(late)));
    assert!(results(&app).tree.node(NodeId::ROOT).is_done());
    let _ = app.update(Message::Scan(
        id,
        Update::Finished(Summary {
            cancelled: true,
            ..Summary::default()
        }),
    ));
    let r = results(&app);
    assert_eq!(r.phase, Phase::Stopped);
    assert!(r.draining.is_none() && r.summary.is_some());
}

#[test]
fn stop_while_opening_returns_to_the_start_screen() {
    let mut app = app();
    app.start_simulated("/r");
    let _ = app.update(Message::Stop);
    assert!(matches!(app.screen, Screen::Start(_)));
}

#[test]
fn a_failed_full_scan_shows_the_banner() {
    let mut app = app();
    let id = app.start_simulated("/nope");
    let error = StartError {
        kind: ErrorKind::NotFound,
        message: "No such file or directory (os error 2)".into(),
    };
    let _ = app.update(Message::Scan(id, Update::Failed(error)));
    let Screen::Start(start) = &app.screen else {
        panic!("expected the start screen")
    };
    assert_eq!(
        start.banner.as_deref(),
        Some("Can't scan “/nope”: No such folder.")
    );
}

#[test]
fn updates_of_a_replaced_scan_are_ignored() {
    let mut app = app();
    let builder = sample();
    let old = app.start_simulated("/r");
    let new = app.start_simulated("/r");
    assert_ne!(old, new);
    let _ = app.update(Message::Scan(new, Update::Started(builder.root_info())));
    let _ = app.update(Message::Scan(old, batch(builder.listings())));
    let r = results(&app);
    assert_eq!(r.tree.listed(), 0, "the old scan's batch was dropped");
    let _ = app.update(Message::Scan(old, Update::Finished(Summary::default())));
    assert_eq!(results(&app).phase, Phase::Scanning);
}

#[test]
fn a_complete_scan_and_its_recent_entry() {
    let mut app = app();
    let builder = sample();
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let _ = app.update(Message::Scan(id, batch(builder.listings())));
    let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
    let r = results(&app);
    assert_eq!(r.phase, Phase::Complete);
    assert!(r.can_rescan());
    assert_eq!(
        app.prefs()
            .recents
            .first()
            .map(|(p, _)| p.to_str().unwrap()),
        Some("/r")
    );
}

#[test]
fn escape_stops_a_scan() {
    let mut app = app();
    let builder = sample();
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let _ = app.update(Message::Key(crate::keys::KeyAction::Escape));
    assert_eq!(results(&app).phase, Phase::Stopped);
}
