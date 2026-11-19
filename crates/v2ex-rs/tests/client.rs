use std::time::Duration;

use v2ex_rs::{Client, Error, Listing, Topic};
use wiremock::matchers::{any, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOPICS: &str = include_str!("fixtures/api1/topics.json");
const TOPICS_GZ: &[u8] = include_bytes!("fixtures/api1/topics.json.gz");
const TOPIC: &str = include_str!("fixtures/api1/topic.json");
const TOPIC_RESTRICTED: &str = include_str!("fixtures/api1/topic_restricted.json");
const REPLIES: &str = include_str!("fixtures/api1/replies.json");
const NODE: &str = include_str!("fixtures/api1/node.json");
const NODES: &str = include_str!("fixtures/api1/nodes.json");
const NODE_FEED: &str = include_str!("fixtures/feed/node.xml");
const TAB_HOT_FEED: &str = include_str!("fixtures/feed/tab_hot.xml");
const TAB_FEED: &str = include_str!("fixtures/feed/tab.xml");

fn client(server: &MockServer) -> Client {
    Client::builder().base_url(server.uri()).build().unwrap()
}

fn ids(topics: &[Topic]) -> Vec<u64> {
    topics.iter().map(|t| t.id).collect()
}

fn titles(topics: &[Topic]) -> Vec<&str> {
    topics.iter().map(|t| t.title.as_str()).collect()
}

/// Serves `body` for `GET path` carrying the given query parameters.
fn serve(path_: &str, query: &[(&str, &str)], body: &str) -> Mock {
    let mut matcher = Mock::given(method("GET")).and(path(path_));
    for &(key, value) in query {
        matcher = matcher.and(query_param(key, value));
    }
    matcher.respond_with(ResponseTemplate::new(200).set_body_string(body))
}

/// Mounts the three restricted-node sources, each expected to be requested `times` times.
async fn mount_restricted_sources(server: &MockServer, times: u64) {
    serve(
        "/api/topics/show.json",
        &[("node_name", "flamewar")],
        TOPICS,
    )
    .expect(times)
    .mount(server)
    .await;
    serve("/feed/nsfw.xml", &[], NODE_FEED)
        .expect(times)
        .mount(server)
        .await;
    serve("/feed/sandbox.xml", &[], NODE_FEED)
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn hot_drops_restricted_nodes() {
    let server = MockServer::start().await;
    serve("/api/topics/hot.json", &[], TOPICS)
        .mount(&server)
        .await;

    let p = client(&server).topics(&Listing::Hot, 1).await.unwrap();

    assert_eq!(ids(&p.items), vec![1001, 1003]);
    assert_eq!((p.page, p.has_more), (1, false));
}

#[tokio::test]
async fn latest_uses_latest_endpoint() {
    let server = MockServer::start().await;
    serve("/api/topics/latest.json", &[], TOPICS)
        .mount(&server)
        .await;

    let p = client(&server).topics(&Listing::Latest, 1).await.unwrap();

    assert_eq!(ids(&p.items), vec![1001, 1003]);
}

#[tokio::test]
async fn node_listing_reads_node_feed() {
    let server = MockServer::start().await;
    serve("/feed/python.xml", &[], NODE_FEED)
        .mount(&server)
        .await;

    let p = client(&server)
        .topics(&Listing::Node("python".into()), 1)
        .await
        .unwrap();

    assert_eq!(ids(&p.items), vec![2001, 2002, 2003]);
    let node = p.items[0].node.as_ref().unwrap();
    assert_eq!(node.name.as_deref(), Some("python"));
    assert_eq!((p.page, p.has_more), (1, false));
}

#[tokio::test]
async fn restricted_node_listing_is_refused_without_request() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client(&server);

    for name in ["flamewar", "nsfw", "sandbox"] {
        let result = client.topics(&Listing::Node(name.into()), 1).await;
        assert!(matches!(result, Err(Error::Restricted { node }) if node == name));
    }
}

#[tokio::test]
async fn node_listing_whose_feed_resolves_to_a_restricted_node_is_refused() {
    let server = MockServer::start().await;
    let feed = NODE_FEED.replace("<title>Python</title>", "<title>NSFW</title>");
    serve("/feed/alias-x.xml", &[], &feed).mount(&server).await;

    let result = client(&server)
        .topics(&Listing::Node("alias-x".into()), 1)
        .await;

    assert!(matches!(result, Err(Error::Restricted { node }) if node == "alias-x"));
}

#[tokio::test]
async fn restricted_names_match_ignoring_case_without_request() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client(&server);

    let listing = client.topics(&Listing::Node("NSFW".into()), 1).await;
    assert!(matches!(listing, Err(Error::Restricted { node }) if node == "NSFW"));
    let node = client.node("FLAMEWAR").await;
    assert!(matches!(node, Err(Error::Restricted { node }) if node == "FLAMEWAR"));
    let node = client.node("Sandbox").await;
    assert!(matches!(node, Err(Error::Restricted { node }) if node == "Sandbox"));
}

#[tokio::test]
async fn names_are_percent_encoded_as_query_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/nodes/show.json"))
        .and(query_param("name", "a&b#c"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NODE))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).node("a&b#c").await.unwrap();
}

#[tokio::test]
async fn names_are_percent_encoded_as_path_segments() {
    let server = MockServer::start().await;
    let client = client(&server);
    let name = "a/b?c#d&e";

    let _ = client.topics(&Listing::Node(name.into()), 1).await;
    let _ = client.topics(&Listing::Tab(name.into()), 1).await;

    let requests = server.received_requests().await.unwrap();
    let targets: Vec<_> = requests
        .iter()
        .map(|r| (r.url.path(), r.url.query()))
        .collect();
    assert_eq!(
        targets,
        vec![
            ("/feed/a%2Fb%3Fc%23d%26e.xml", None),
            ("/feed/tab/a%2Fb%3Fc%23d%26e.xml", None),
        ]
    );
}

#[tokio::test]
async fn hot_tab_drops_restricted_titles() {
    let server = MockServer::start().await;
    serve("/feed/tab/hot.xml", &[], TAB_HOT_FEED)
        .mount(&server)
        .await;

    let p = client(&server)
        .topics(&Listing::Tab("hot".into()), 1)
        .await
        .unwrap();

    assert_eq!(titles(&p.items), vec!["标题甲", "[求助] 标题乙"]);
}

#[tokio::test]
async fn hot_tab_also_drops_recent_restricted_ids() {
    let server = MockServer::start().await;
    serve("/feed/tab/hot.xml", &[], TAB_HOT_FEED)
        .mount(&server)
        .await;
    let nsfw = NODE_FEED.replace("/t/2001#", "/t/3001#");
    serve("/feed/nsfw.xml", &[], &nsfw).mount(&server).await;

    let p = client(&server)
        .topics(&Listing::Tab("hot".into()), 1)
        .await
        .unwrap();

    assert_eq!(titles(&p.items), vec!["[求助] 标题乙"]);
}

#[tokio::test]
async fn plain_tab_drops_recent_restricted_ids_and_skips_failed_sources() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .mount(&server)
        .await;
    serve(
        "/api/topics/show.json",
        &[("node_name", "flamewar")],
        TOPICS,
    )
    .mount(&server)
    .await;
    Mock::given(method("GET"))
        .and(path("/feed/nsfw.xml"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    serve("/feed/sandbox.xml", &[], NODE_FEED)
        .mount(&server)
        .await;

    let p = client(&server)
        .topics(&Listing::Tab("all".into()), 1)
        .await
        .unwrap();

    assert_eq!(ids(&p.items), vec![4001, 4002]);
    assert_eq!(p.items[0].title, "[拼团] 作者自己的方括号");
}

#[tokio::test]
async fn restricted_ids_are_refetched_with_each_fresh_tab_feed() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .expect(1)
        .mount(&server)
        .await;
    serve("/feed/tab/tech.xml", &[], TAB_FEED)
        .expect(1)
        .mount(&server)
        .await;
    mount_restricted_sources(&server, 2).await;
    let client = client(&server);

    // The first two fetch their feed afresh; the third reads its feed from the cache and so
    // reuses the kept ids.
    client.topics(&Listing::Tab("all".into()), 1).await.unwrap();
    client
        .topics(&Listing::Tab("tech".into()), 1)
        .await
        .unwrap();
    client.topics(&Listing::Tab("all".into()), 1).await.unwrap();
}

#[tokio::test]
async fn concurrent_tab_loads_both_wait_for_the_restricted_ids() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .expect(1)
        .mount(&server)
        .await;
    serve(
        "/api/topics/show.json",
        &[("node_name", "flamewar")],
        TOPICS,
    )
    .mount(&server)
    .await;
    // The nsfw feed lists topic 4001, which `TAB_FEED` also lists, and answers late.
    Mock::given(method("GET"))
        .and(path("/feed/nsfw.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(NODE_FEED.replace("/t/2001#", "/t/4001#"))
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;
    serve("/feed/sandbox.xml", &[], NODE_FEED)
        .mount(&server)
        .await;
    let client = client(&server);
    let listing = Listing::Tab("all".into());

    // The second load starts while the first waits for the nsfw feed.
    let (first, second) = tokio::join!(client.topics(&listing, 1), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        client.topics(&listing, 1).await
    });

    assert_eq!(ids(&first.unwrap().items), vec![4002]);
    assert_eq!(ids(&second.unwrap().items), vec![4002]);
}

#[tokio::test]
async fn a_tab_load_dropped_before_its_restricted_ids_arrive_caches_nothing() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .expect(2)
        .mount(&server)
        .await;
    serve(
        "/api/topics/show.json",
        &[("node_name", "flamewar")],
        TOPICS,
    )
    .mount(&server)
    .await;
    // The nsfw feed lists topic 4001, which `TAB_FEED` also lists, and answers late.
    Mock::given(method("GET"))
        .and(path("/feed/nsfw.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(NODE_FEED.replace("/t/2001#", "/t/4001#"))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;
    serve("/feed/sandbox.xml", &[], NODE_FEED)
        .mount(&server)
        .await;
    let client = client(&server);
    let listing = Listing::Tab("all".into());

    // Given up on while it waits for the nsfw feed.
    let dropped =
        tokio::time::timeout(Duration::from_millis(100), client.topics(&listing, 1)).await;
    assert!(dropped.is_err());
    let p = client.topics(&listing, 1).await.unwrap();

    assert_eq!(ids(&p.items), vec![4002]);
}

/// Mounts the three restricted-node sources. The nsfw feed lists topic 4001, which `TAB_FEED`
/// also lists; it answers once with that feed and then with `later`.
async fn mount_restricted_sources_with_nsfw_then(server: &MockServer, later: ResponseTemplate) {
    serve(
        "/api/topics/show.json",
        &[("node_name", "flamewar")],
        TOPICS,
    )
    .mount(server)
    .await;
    serve(
        "/feed/nsfw.xml",
        &[],
        &NODE_FEED.replace("/t/2001#", "/t/4001#"),
    )
    .up_to_n_times(1)
    .expect(1)
    .mount(server)
    .await;
    Mock::given(method("GET"))
        .and(path("/feed/nsfw.xml"))
        .respond_with(later)
        .expect(1..)
        .mount(server)
        .await;
    serve("/feed/sandbox.xml", &[], NODE_FEED)
        .mount(server)
        .await;
}

#[tokio::test]
async fn ids_kept_before_clear_cache_filter_while_the_nsfw_source_fails() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .expect(2)
        .mount(&server)
        .await;
    mount_restricted_sources_with_nsfw_then(&server, ResponseTemplate::new(500)).await;
    let client = client(&server);

    let first = client.topics(&Listing::Tab("all".into()), 1).await.unwrap();
    client.clear_cache();
    let second = client.topics(&Listing::Tab("all".into()), 1).await.unwrap();

    assert_eq!(ids(&first.items), vec![4002]);
    assert_eq!(ids(&second.items), vec![4002]);
}

#[tokio::test]
async fn ids_kept_before_clear_cache_filter_while_the_flamewar_source_is_rate_limited() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .expect(2)
        .mount(&server)
        .await;
    // `TOPICS` lists topic 1002, which `TAB_FEED` also lists.
    serve(
        "/api/topics/show.json",
        &[("node_name", "flamewar")],
        TOPICS,
    )
    .up_to_n_times(1)
    .expect(1)
    .mount(&server)
    .await;
    Mock::given(method("GET"))
        .and(path("/api/topics/show.json"))
        .and(query_param("node_name", "flamewar"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;
    serve("/feed/nsfw.xml", &[], NODE_FEED).mount(&server).await;
    serve("/feed/sandbox.xml", &[], NODE_FEED)
        .mount(&server)
        .await;
    let client = client(&server);

    let first = client.topics(&Listing::Tab("all".into()), 1).await.unwrap();
    client.clear_cache();
    let second = client.topics(&Listing::Tab("all".into()), 1).await.unwrap();

    assert_eq!(ids(&first.items), vec![4001, 4002]);
    assert_eq!(ids(&second.items), vec![4001, 4002]);
}

#[tokio::test]
async fn failed_refetch_of_a_restricted_source_keeps_its_ids() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .mount(&server)
        .await;
    serve("/feed/tab/tech.xml", &[], TAB_FEED)
        .mount(&server)
        .await;
    mount_restricted_sources_with_nsfw_then(&server, ResponseTemplate::new(500)).await;
    let client = client(&server);

    let first = client.topics(&Listing::Tab("all".into()), 1).await.unwrap();
    let second = client
        .topics(&Listing::Tab("tech".into()), 1)
        .await
        .unwrap();

    assert_eq!(ids(&first.items), vec![4002]);
    assert_eq!(ids(&second.items), vec![4002]);
}

#[tokio::test]
async fn unparseable_refetch_of_a_restricted_source_keeps_its_ids() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .mount(&server)
        .await;
    serve("/feed/tab/tech.xml", &[], TAB_FEED)
        .mount(&server)
        .await;
    mount_restricted_sources_with_nsfw_then(
        &server,
        ResponseTemplate::new(200).set_body_string("<html>busy</html>"),
    )
    .await;
    let client = client(&server);

    client.topics(&Listing::Tab("all".into()), 1).await.unwrap();
    let fresh = client
        .topics(&Listing::Tab("tech".into()), 1)
        .await
        .unwrap();
    // This feed comes from the cache, so the ids are not fetched again: the ids that the bad
    // body did not replace do the filtering. The nsfw feed is requested twice in total, once
    // for `all` and once for the refresh behind `tech`.
    let cached = client.topics(&Listing::Tab("all".into()), 1).await.unwrap();

    assert_eq!(ids(&fresh.items), vec![4002]);
    assert_eq!(ids(&cached.items), vec![4002]);
    let nsfw_requests = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/feed/nsfw.xml")
        .count();
    assert_eq!(nsfw_requests, 2);
}

#[tokio::test]
async fn unknown_tab_is_not_found_and_not_cached() {
    let server = MockServer::start().await;
    serve("/feed/tab/zzzz.xml", &[], "")
        .expect(2)
        .mount(&server)
        .await;
    let client = client(&server);

    for _ in 0..2 {
        let result = client.topics(&Listing::Tab("zzzz".into()), 1).await;
        assert!(matches!(result, Err(Error::NotFound)), "{result:?}");
    }
}

#[tokio::test]
async fn page_two_is_empty_without_request() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client(&server);

    for listing in [
        Listing::Hot,
        Listing::Latest,
        Listing::Tab("all".into()),
        Listing::Node("python".into()),
    ] {
        let p = client.topics(&listing, 2).await.unwrap();
        assert!(p.items.is_empty());
        assert_eq!((p.page, p.has_more), (2, false));
    }
}

#[tokio::test]
async fn topic_by_id() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "1001")], TOPIC)
        .mount(&server)
        .await;

    let topic = client(&server).topic(1001).await.unwrap();

    assert_eq!(topic.id, 1001);
}

#[tokio::test]
async fn unknown_topic_is_not_found() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "999")], "[]")
        .mount(&server)
        .await;

    let result = client(&server).topic(999).await;

    assert!(matches!(result, Err(Error::NotFound)));
}

#[tokio::test]
async fn a_response_for_another_topic_is_not_found() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "1002")], TOPIC)
        .mount(&server)
        .await;

    let result = client(&server).topic(1002).await;

    assert!(matches!(result, Err(Error::NotFound)));
}

#[tokio::test]
async fn an_undecodable_response_is_not_cached() {
    let server = MockServer::start().await;
    serve("/api/topics/hot.json", &[], "<html>busy</html>")
        .up_to_n_times(1)
        .mount(&server)
        .await;
    serve("/api/topics/hot.json", &[], TOPICS)
        .mount(&server)
        .await;
    let client = client(&server);

    let first = client.topics(&Listing::Hot, 1).await;
    let second = client.topics(&Listing::Hot, 1).await;

    assert!(matches!(first, Err(Error::Decode(_))));
    assert!(!second.unwrap().items.is_empty());
}

#[tokio::test]
async fn restricted_topic_is_refused() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "1002")], TOPIC_RESTRICTED)
        .mount(&server)
        .await;

    let result = client(&server).topic(1002).await;

    assert!(matches!(result, Err(Error::Restricted { node }) if node == "flamewar"));
}

#[tokio::test]
async fn replies_first_page_and_empty_second_page() {
    let server = MockServer::start().await;
    serve("/api/replies/show.json", &[("topic_id", "1001")], REPLIES)
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);

    let first = client.replies(1001, 1).await.unwrap();
    let second = client.replies(1001, 2).await.unwrap();

    let floors: Vec<u32> = first.items.iter().map(|r| r.floor).collect();
    assert_eq!(floors, vec![1, 2]);
    assert_eq!((first.page, first.has_more), (1, false));
    assert!(second.items.is_empty());
    assert_eq!((second.page, second.has_more), (2, false));
}

#[tokio::test]
async fn node_lookup_and_not_found() {
    let server = MockServer::start().await;
    serve("/api/nodes/show.json", &[("name", "python")], NODE)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/nodes/show.json"))
        .and(query_param("name", "nope"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"status": "error", "message": "Object Not Found"}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/nodes/show.json"))
        .and(query_param("name", "nsfw"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client(&server);

    assert_eq!(client.node("python").await.unwrap().name, "python");
    assert!(matches!(client.node("nope").await, Err(Error::NotFound)));
    assert!(matches!(
        client.node("nsfw").await,
        Err(Error::Restricted { node }) if node == "nsfw"
    ));
}

#[tokio::test]
async fn node_resolved_by_the_server_to_a_restricted_node_is_refused() {
    let server = MockServer::start().await;
    // The server trims whitespace and folds accents, so each of these names comes back as `nsfw`.
    let canonical = NODE.replace(r#""name": "python""#, r#""name": "nsfw""#);
    serve("/api/nodes/show.json", &[], &canonical)
        .mount(&server)
        .await;
    let client = client(&server);

    for name in ["nsfw ", " nsfw", "nśfw"] {
        let result = client.node(name).await;
        assert!(matches!(result, Err(Error::Restricted { node }) if node == name));
    }
}

#[tokio::test]
async fn nodes_excludes_restricted() {
    let server = MockServer::start().await;
    serve("/api/nodes/all.json", &[], NODES)
        .mount(&server)
        .await;

    let nodes = client(&server).nodes().await.unwrap();

    let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["python", "qna"]);
}

#[tokio::test]
async fn rate_limit_is_exposed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/topics/hot.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(TOPICS)
                .insert_header("x-rate-limit-limit", "600")
                .insert_header("x-rate-limit-remaining", "571")
                // 2100-01-01: a reset time that is still ahead.
                .insert_header("x-rate-limit-reset", "4102444800"),
        )
        .mount(&server)
        .await;
    let client = client(&server);

    assert_eq!(client.rate_limit(), None);
    client.topics(&Listing::Hot, 1).await.unwrap();

    assert_eq!(client.rate_limit().unwrap().remaining, 571);
}

#[tokio::test]
async fn clear_cache_forces_a_refetch() {
    let server = MockServer::start().await;
    serve("/api/topics/hot.json", &[], TOPICS)
        .expect(2)
        .mount(&server)
        .await;
    let client = client(&server);

    client.topics(&Listing::Hot, 1).await.unwrap();
    client.topics(&Listing::Hot, 1).await.unwrap();
    client.clear_cache();
    client.topics(&Listing::Hot, 1).await.unwrap();
}

/// Serves the hot listing only to requests carrying `user_agent`.
async fn mount_hot_for_user_agent(server: &MockServer, user_agent: &str) {
    Mock::given(method("GET"))
        .and(path("/api/topics/hot.json"))
        .and(header("user-agent", user_agent))
        .respond_with(ResponseTemplate::new(200).set_body_string(TOPICS))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn default_user_agent_names_the_library() {
    let server = MockServer::start().await;
    mount_hot_for_user_agent(&server, concat!("v2ex-rs/", env!("CARGO_PKG_VERSION"))).await;

    client(&server).topics(&Listing::Hot, 1).await.unwrap();
}

#[tokio::test]
async fn builder_overrides_user_agent() {
    let server = MockServer::start().await;
    mount_hot_for_user_agent(&server, "v2exe/9.9").await;
    let client = Client::builder()
        .base_url(server.uri())
        .user_agent("v2exe/9.9")
        .build()
        .unwrap();

    client.topics(&Listing::Hot, 1).await.unwrap();
}

#[tokio::test]
async fn socks5_proxy_is_supported() {
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let client = Client::builder()
        .base_url("http://127.0.0.1:9")
        .proxy(format!("socks5://127.0.0.1:{closed_port}"))
        .build()
        .unwrap();

    let result = client.topics(&Listing::Hot, 1).await;

    assert!(
        matches!(&result, Err(Error::Network(msg)) if !msg.contains("unsupported scheme")),
        "{result:?}"
    );
}

#[tokio::test]
async fn gzip_responses_are_decoded() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/topics/hot.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "gzip")
                .set_body_bytes(TOPICS_GZ),
        )
        .mount(&server)
        .await;

    let p = client(&server).topics(&Listing::Hot, 1).await.unwrap();

    assert_eq!(ids(&p.items), vec![1001, 1003]);
}
