//! The topic list of one listing.

use v2ex_rs::Listing;

pub struct TopicsScreen {
    pub listing: Listing,
    /// The listing's display name.
    pub title: String,
    /// The ticket of the request whose result the screen is waiting for.
    pub pending: Option<u64>,
}

impl TopicsScreen {
    pub fn new(listing: Listing, title: String, ticket: u64) -> TopicsScreen {
        TopicsScreen {
            listing,
            title,
            pending: Some(ticket),
        }
    }
}
