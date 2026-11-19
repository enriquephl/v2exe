//! The interactive interface: how it looks and how every screen is framed.

pub mod app;
pub mod frame;
pub mod list;
pub mod nodes;
pub mod prompt;
pub mod reader;
pub mod theme;
pub mod topics;

use v2ex_rs::Listing;

/// The screen the interface opens on, on top of the node list.
#[derive(Debug, Clone, PartialEq)]
pub enum Start {
    Home,
    Listing(Listing),
    Topic(u64),
}
