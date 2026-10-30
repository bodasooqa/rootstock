//! rootstock: create a project from a versioned git template and update it
//! with a three-way merge.

pub mod answers;
pub mod apply;
pub mod cli;
pub mod commands;
pub mod config;
pub mod hooks;
pub mod jinja;
pub mod merge;
pub mod plan;
pub mod record;
pub mod render;
pub mod source;

/// Path relative to a project or template root: `/` separators, never starting with `/`, never containing `..`.
pub type RelPath = String;
