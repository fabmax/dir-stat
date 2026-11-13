//! Cross-platform compile check of the reference engine (no harness, no alternatives):
//! `RUSTC_BOOTSTRAP=1 cargo check -Zbuild-std --target x86_64-unknown-linux-gnu` (and windows-msvc).
#![allow(dead_code)]
#[path = "../../scan-bench/src/listing.rs"]
pub mod listing;
#[path = "../../scan-bench/src/model.rs"]
pub mod model;
#[path = "../../scan-bench/src/mounts.rs"]
pub mod mounts;
#[path = "../../scan-bench/src/queue.rs"]
pub mod queue;
#[path = "../../scan-bench/src/scanner.rs"]
pub mod scanner;
