//! rootstock: create a project from a versioned git template and update it
//! with a three-way merge.

pub mod cli;
pub mod config;
pub mod record;

/// Path relative to a project or template root: `/` separators, never starting with `/`, never containing `..`.
pub type RelPath = String;
