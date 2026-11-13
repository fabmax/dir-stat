//! dir-stat's engine: scanner, tree, cleanup catalog, safety guards and trash.
//! It has no iced dependency, so everything here is testable without a GUI.

pub mod catalog;
pub mod platform;
pub mod scan;
pub mod trash;
pub mod tree;
