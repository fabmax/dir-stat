//! dir-stat's engine: scanner, tree, cleanup catalog, safety guards and trash.
//! It has no iced dependency, so everything here is testable without a GUI.

pub mod breakdown;
pub mod catalog;
pub mod git;
pub mod platform;
pub mod safety;
pub mod scan;
pub mod smallfiles;
pub mod trash;
pub mod tree;
pub mod volumes;
