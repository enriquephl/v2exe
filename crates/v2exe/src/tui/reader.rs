//! The reader of one topic and its replies.

pub struct ReaderScreen {
    /// The topic's id.
    pub id: u64,
    /// The ticket of the request whose result the screen is waiting for.
    pub pending: Option<u64>,
}

impl ReaderScreen {
    pub fn new(id: u64, ticket: u64) -> ReaderScreen {
        ReaderScreen {
            id,
            pending: Some(ticket),
        }
    }
}
