//! Which topics the user has opened and how many replies they had then, saved as `read.json`.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::paths::write_atomic;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    Unread,
    /// Opened before; carries how many replies were added since.
    Updated(u32),
    Read,
}

/// How many topics `save` keeps.
pub const KEEP: usize = 10_000;

#[derive(Serialize, Deserialize)]
struct Seen {
    replies: u32,
    at: DateTime<Utc>,
}

pub struct ReadState {
    path: Option<PathBuf>,
    seen: HashMap<u64, Seen>,
}

impl ReadState {
    /// A missing or unreadable file gives an empty state. Without a `path` the state lives in
    /// memory only.
    pub fn load(path: Option<PathBuf>) -> ReadState {
        let seen = path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        ReadState { path, seen }
    }

    pub fn mark(&self, topic_id: u64, replies: u32) -> Mark {
        match self.seen.get(&topic_id) {
            None => Mark::Unread,
            Some(seen) if replies > seen.replies => Mark::Updated(replies - seen.replies),
            Some(_) => Mark::Read,
        }
    }

    /// Notes that the topic was opened when it had `replies` replies.
    pub fn record(&mut self, topic_id: u64, replies: u32, now: DateTime<Utc>) {
        self.seen.insert(topic_id, Seen { replies, at: now });
    }

    pub fn forget(&mut self, topic_id: u64) {
        self.seen.remove(&topic_id);
    }

    /// Writes the [`KEEP`] most recently opened topics. Without a path there is nothing to
    /// write and this is `Ok`.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut newest: Vec<_> = self.seen.iter().collect();
        newest.sort_by_key(|(id, seen)| std::cmp::Reverse((seen.at, **id)));
        newest.truncate(KEEP);
        let kept: BTreeMap<_, _> = newest.into_iter().collect();
        write_atomic(path, &serde_json::to_vec(&kept)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    #[test]
    fn marks_follow_the_recorded_reply_count() {
        let mut r = ReadState::load(None);
        assert_eq!(r.mark(1, 5), Mark::Unread);
        r.record(1, 5, t0());
        assert_eq!(r.mark(1, 5), Mark::Read);
        assert_eq!(r.mark(1, 8), Mark::Updated(3));
        assert_eq!(r.mark(1, 4), Mark::Read);
        r.forget(1);
        assert_eq!(r.mark(1, 5), Mark::Unread);
    }

    #[test]
    fn recording_again_replaces_the_reply_count() {
        let mut r = ReadState::load(None);
        r.record(1, 5, t0());
        r.record(1, 8, t0() + Duration::minutes(1));
        assert_eq!(r.mark(1, 8), Mark::Read);
        assert_eq!(r.mark(1, 9), Mark::Updated(1));
    }

    #[test]
    fn save_keeps_the_newest_entries_only() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("read.json");
        let mut r = ReadState::load(Some(path.clone()));
        for i in 0..(KEEP + 5) {
            r.record(i as u64, 2, t0() + Duration::seconds(i as i64));
        }
        r.save().unwrap();

        let reloaded = ReadState::load(Some(path));
        for i in 0..5 {
            assert_eq!(reloaded.mark(i, 2), Mark::Unread);
        }
        for i in 5..(KEEP + 5) as u64 {
            assert_eq!(reloaded.mark(i, 2), Mark::Read);
        }
        assert_eq!(reloaded.mark(8_000, 3), Mark::Updated(1));
    }

    #[test]
    fn saving_twice_writes_the_same_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("read.json");
        let mut r = ReadState::load(Some(path.clone()));
        for id in [30, 4, 5000, 12, 777] {
            r.record(id, id as u32, t0());
        }
        r.save().unwrap();
        let first = std::fs::read(&path).unwrap();
        ReadState::load(Some(path.clone())).save().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), first);
    }

    #[test]
    fn the_file_maps_topic_numbers_to_replies_and_time() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("read.json");
        let mut r = ReadState::load(Some(path.clone()));
        r.record(42, 7, t0());
        r.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["42"]["replies"], 7);
        assert_eq!(value["42"]["at"], "2026-01-01T00:00:00Z");
    }

    #[test]
    fn a_corrupt_file_reads_as_empty_and_save_without_a_path_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("read.json");
        std::fs::write(&path, "[1,2").unwrap();
        let r = ReadState::load(Some(path));
        assert_eq!(r.mark(1, 0), Mark::Unread);

        ReadState::load(None).save().unwrap();
    }
}
