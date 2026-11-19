//! The node directory kept as `nodes.json` in the cache directory, good for a day.

use std::io;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use v2ex_rs::Node;

use super::paths::write_atomic;

/// How long a saved directory is used before it is fetched again.
pub const MAX_AGE: chrono::Duration = chrono::Duration::hours(24);

#[derive(Serialize)]
struct Saved<'a> {
    fetched: DateTime<Utc>,
    nodes: &'a [Node],
}

#[derive(Deserialize)]
struct Loaded {
    fetched: DateTime<Utc>,
    nodes: Vec<Node>,
}

/// `None` when the file is missing, unreadable, older than [`MAX_AGE`] at `now`, or fetched
/// after `now`, which a clock set back would give.
pub fn load_nodes(path: &Path, now: DateTime<Utc>) -> Option<Vec<Node>> {
    let bytes = std::fs::read(path).ok()?;
    let loaded: Loaded = serde_json::from_slice(&bytes).ok()?;
    let age = now - loaded.fetched;
    (age >= chrono::Duration::zero() && age <= MAX_AGE).then_some(loaded.nodes)
}

pub fn save_nodes(path: &Path, nodes: &[Node], now: DateTime<Utc>) -> io::Result<()> {
    let saved = Saved {
        fetched: now,
        nodes,
    };
    write_atomic(path, &serde_json::to_vec(&saved)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn node(id: u64, name: &str, title: &str) -> Node {
        Node {
            id,
            name: name.into(),
            title: title.into(),
            title_alternative: Some(name.to_uppercase()),
            topics: 120,
            stars: 30,
            header: None,
            parent_node_name: Some("programming".into()),
        }
    }

    #[test]
    fn nodes_round_trip_until_they_are_a_day_old() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cache").join("nodes.json");
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let nodes = vec![node(1, "python", "Python"), node(2, "rust", "Rust")];

        save_nodes(&path, &nodes, t0).unwrap();

        assert_eq!(load_nodes(&path, t0), Some(nodes.clone()));
        assert_eq!(
            load_nodes(&path, t0 + Duration::hours(23)),
            Some(nodes.clone())
        );
        assert_eq!(load_nodes(&path, t0 + Duration::hours(25)), None);
        assert_eq!(load_nodes(&tmp.path().join("missing.json"), t0), None);

        std::fs::write(&path, "x").unwrap();
        assert_eq!(load_nodes(&path, t0), None);
    }

    #[test]
    fn a_directory_exactly_max_age_old_is_fresh_and_one_from_the_future_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nodes.json");
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let nodes = vec![node(1, "python", "Python")];

        save_nodes(&path, &nodes, t0).unwrap();

        assert_eq!(load_nodes(&path, t0 + MAX_AGE), Some(nodes));
        assert_eq!(load_nodes(&path, t0 + MAX_AGE + Duration::seconds(1)), None);
        assert_eq!(load_nodes(&path, t0 - Duration::seconds(1)), None);
    }

    #[test]
    fn the_file_holds_the_fetch_time_and_the_nodes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nodes.json");
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        save_nodes(&path, &[node(1, "python", "Python")], t0).unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["fetched"], "2026-01-01T00:00:00Z");
        assert_eq!(value["nodes"][0]["name"], "python");
    }
}
