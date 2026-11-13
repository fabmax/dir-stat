//! dir-stat cleanup catalog prototype: data model (`model`), static table (`catalog`) and the
//! streaming matcher (`engine`). No dependencies; `cargo test` runs the invariant tests and
//! `cargo run --example catalog_md` prints the catalog as Markdown tables.
pub mod catalog;
pub mod engine;
pub mod model;
