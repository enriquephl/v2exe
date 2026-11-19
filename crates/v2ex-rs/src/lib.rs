// Only tests call the parsers.
#[cfg_attr(not(test), expect(dead_code))]
mod api1;
mod error;
mod types;

pub use error::{Error, Result};
pub use types::{Listing, MemberRef, Node, NodeRef, Page, RateLimit, Reply, Topic};
