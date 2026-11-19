use v2ex_rs::{Client, Error, Listing, Topic};
use wiremock::matchers::{any, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOPICS: &str = include_str!("fixtures/api1/topics.json");
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
async fn restricted_ids_are_fetched_once_across_tab_listings() {
    let server = MockServer::start().await;
    serve("/feed/tab/all.xml", &[], TAB_FEED)
        .mount(&server)
        .await;
    serve("/feed/tab/tech.xml", &[], TAB_FEED)
        .mount(&server)
        .await;
    mount_restricted_sources(&server, 1).await;
    let client = client(&server);

    client.topics(&Listing::Tab("all".into()), 1).await.unwrap();
    client
        .topics(&Listing::Tab("tech".into()), 1)
        .await
        .unwrap();
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
                .insert_header("x-rate-limit-reset", "1795060800"),
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
