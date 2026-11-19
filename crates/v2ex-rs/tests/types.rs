use chrono::{TimeZone, Utc};
use v2ex_rs::{Error, MemberRef, Topic};

#[test]
fn topic_serializes_with_rfc3339_times_and_null_options() {
    let t = Topic {
        id: 1001,
        title: "示例标题一".into(),
        url: "https://www.v2ex.com/t/1001".into(),
        author: MemberRef {
            username: "alice".into(),
            id: Some(7),
        },
        node: None,
        created: Utc.timestamp_opt(1795050000, 0).unwrap(),
        last_touched: None,
        replies: 3,
        last_reply_by: None,
        content: None,
        content_html: None,
    };
    let v: serde_json::Value = serde_json::to_value(&t).unwrap();
    assert_eq!(v["created"], "2026-11-19T01:00:00Z");
    assert_eq!(v["node"], serde_json::Value::Null);
    assert_eq!(v["author"]["username"], "alice");
    let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys.len(), 11);
    let back: Topic = serde_json::from_value(v).unwrap();
    assert_eq!(back, t);
}

#[test]
fn rate_limited_error_displays() {
    let e = Error::RateLimited { reset: None };
    assert!(!e.to_string().is_empty());
}
