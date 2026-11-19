//! The interactive interface: its screens, how they are drawn, and the runtime that drives them.

pub mod app;
pub mod frame;
pub mod help;
pub mod list;
pub mod nodes;
pub mod prompt;
pub mod reader;
mod runtime;
pub mod theme;
pub mod topics;

use chrono::FixedOffset;
use v2ex_rs::Listing;

use crate::store::config::SegmentEdge;
use crate::store::paths::Dirs;

pub use runtime::run;

/// The screen the interface opens on, on top of the node list.
#[derive(Debug, Clone, PartialEq)]
pub enum Start {
    Home,
    Listing(Listing),
    Topic(u64),
}

/// What the interface is started with.
pub struct Options {
    pub start: Start,
    pub edge: SegmentEdge,
    pub color: bool,
    /// The offset times are shown in.
    pub tz: FixedOffset,
    /// Where the favorites, the read state and the node directory are kept.
    pub dirs: Dirs,
}
