//! The public entry point: which source serves each call, and the restricted-node rules.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, PercentEncode, utf8_percent_encode};

use crate::feed::{self, FeedKind};
use crate::http::{Endpoint, Settings, Transport, lock};
use crate::restricted::{self, IdSource, RestrictedNode};
use crate::{Error, Listing, Node, Page, RateLimit, Reply, Result, Topic, api1};

const DEFAULT_BASE_URL: &str = "https://www.v2ex.com";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

const TOPICS_TTL: Duration = Duration::from_secs(120);
const REPLIES_TTL: Duration = Duration::from_secs(60);
const NODE_TTL: Duration = Duration::from_secs(3600);
const NODES_TTL: Duration = Duration::from_secs(7200);

const NAME_ESCAPES: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Cheap to clone; clones share one connection pool, cache, quota record and set of restricted
/// ids, and take turns at tab listings.
#[derive(Clone)]
pub struct Client {
    transport: Transport,
    /// The recent topic ids of each restricted node, by name, from the last fetch of its source
    /// that succeeded. Kept apart from the response cache: [`Client::clear_cache`] leaves them,
    /// and they do not expire.
    restricted_ids: Arc<Mutex<HashMap<&'static str, HashSet<u64>>>>,
    /// Held for the whole of a tab listing (feed, id refetch, filtering), so tab listings run
    /// one at a time and each filters with the ids that go with its feed.
    tab_turn: Arc<tokio::sync::Mutex<()>>,
}

pub struct ClientBuilder {
    settings: Settings,
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder {
            settings: Settings {
                base_url: DEFAULT_BASE_URL.to_string(),
                proxy: None,
                timeout: DEFAULT_TIMEOUT,
                user_agent: concat!("v2ex-rs/", env!("CARGO_PKG_VERSION")).to_string(),
            },
        }
    }

    /// Lists topics. Only page 1 exists: later pages are empty and cost no request.
    ///
    /// Restricted nodes are refused with [`Error::Restricted`] and dropped from the listings that
    /// can contain them.
    pub async fn topics(&self, listing: &Listing, page: u32) -> Result<Page<Topic>> {
        let items = if page == 1 {
            self.first_page(listing).await?
        } else {
            Vec::new()
        };
        Ok(Page {
            items,
            page,
            has_more: false,
        })
    }

    /// Fetches one topic. A topic in a restricted node is refused with [`Error::Restricted`].
    pub async fn topic(&self, id: u64) -> Result<Topic> {
        let topic = self
            .api_topics(&format!("/api/topics/show.json?id={id}"), TOPICS_TTL)
            .await?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or(Error::NotFound)?;
        match topic.node.as_ref().and_then(restricted::of) {
            Some(node) => Err(restricted_error(node.name)),
            None => Ok(topic),
        }
    }

    /// Fetches all replies of a topic. Only page 1 exists: later pages are empty and cost no
    /// request. The response carries no node information, so the caller must have fetched the
    /// topic with [`Client::topic`] first to get the restricted-node check.
    pub async fn replies(&self, topic_id: u64, page: u32) -> Result<Page<Reply>> {
        let items = if page == 1 {
            let path = format!("/api/replies/show.json?topic_id={topic_id}");
            self.fetch(&path, Endpoint::Api, REPLIES_TTL, api1::parse_replies)
                .await?
        } else {
            Vec::new()
        };
        Ok(Page {
            items,
            page,
            has_more: false,
        })
    }

    /// Fetches one node. A restricted node is refused with [`Error::Restricted`]: by its name
    /// without a request, or, when the server resolves some other spelling of the name to a
    /// restricted node, by the name in the response.
    pub async fn node(&self, name: &str) -> Result<Node> {
        if restricted::by_name(name).is_some() {
            return Err(restricted_error(name));
        }
        let path = format!("/api/nodes/show.json?name={}", encode(name));
        let node = self
            .fetch(&path, Endpoint::Api, NODE_TTL, api1::parse_node)
            .await?;
        if restricted::by_name(&node.name).is_some() {
            return Err(restricted_error(name));
        }
        Ok(node)
    }

    /// Fetches the node directory, without restricted nodes.
    pub async fn nodes(&self) -> Result<Vec<Node>> {
        let mut nodes = self
            .fetch(
                "/api/nodes/all.json",
                Endpoint::Api,
                NODES_TTL,
                api1::parse_nodes,
            )
            .await?;
        nodes.retain(|n| restricted::by_name(&n.name).is_none());
        Ok(nodes)
    }

    /// The quota reported by the most recent API response whose reset time had not passed. A
    /// copy served from the site's CDN carries the headers of the moment it was cached.
    pub fn rate_limit(&self) -> Option<RateLimit> {
        self.transport.rate_limit()
    }

    /// Drops every cached response, so the next calls fetch afresh. The ids kept for the
    /// restricted nodes stay.
    pub fn clear_cache(&self) {
        self.transport.clear_cache();
    }

    async fn first_page(&self, listing: &Listing) -> Result<Vec<Topic>> {
        let mut topics = match listing {
            Listing::Hot => self.api_topics("/api/topics/hot.json", TOPICS_TTL).await?,
            Listing::Latest => {
                self.api_topics("/api/topics/latest.json", TOPICS_TTL)
                    .await?
            }
            Listing::Tab(tab) => {
                let _turn = self.tab_turn.lock().await;
                // Only the hot tab names each topic's node, in a title prefix.
                let (path, kind) = if tab.eq_ignore_ascii_case("hot") {
                    ("/feed/tab/hot.xml".to_string(), FeedKind::Prefixed)
                } else {
                    (format!("/feed/tab/{}.xml", encode(tab)), FeedKind::Plain)
                };
                // A fresh feed enters the cache only once the ids that go with it are kept, so a
                // load dropped in between leaves nothing behind and the next one starts over. A
                // cached feed reuses the kept ids without a request.
                let mut topics = match self.transport.cached(&path) {
                    Some(body) => parse_feed_body(&body, &kind)?,
                    None => {
                        let body = self.transport.request(&path, Endpoint::Feed).await?;
                        let topics = parse_feed_body(&body, &kind)?;
                        self.refetch_restricted_ids().await;
                        self.transport.store(&path, body, TOPICS_TTL);
                        topics
                    }
                };
                let hidden = lock(&self.restricted_ids);
                topics.retain(|t| !hidden.values().any(|ids| ids.contains(&t.id)));
                topics
            }
            Listing::Node(name) => {
                if restricted::by_name(name).is_some() {
                    return Err(restricted_error(name));
                }
                // The feed's title is the node's canonical title, so a name the server resolves
                // to a restricted node shows up here.
                let topics = self.node_feed(name, TOPICS_TTL).await?;
                if topics.iter().any(is_restricted) {
                    return Err(restricted_error(name));
                }
                topics
            }
        };
        // Hot, latest and the hot tab name each topic's node; the other listings carry none that
        // is restricted. Tab feeds are also filtered by id above, which does not depend on the
        // feed's title format.
        topics.retain(|t| !is_restricted(t));
        Ok(topics)
    }

    /// Fetches the source of every restricted node: a fetch that succeeds replaces its node's
    /// ids, and one that fails leaves them as they were.
    async fn refetch_restricted_ids(&self) {
        for node in restricted::RESTRICTED {
            if let Ok(topics) = self.id_source_topics(node).await {
                let ids = topics.iter().map(|t| t.id).collect();
                lock(&self.restricted_ids).insert(node.name, ids);
            }
        }
    }

    /// The recent topics of a restricted node, fetched without the response cache.
    async fn id_source_topics(&self, node: &RestrictedNode) -> Result<Vec<Topic>> {
        match node.source {
            IdSource::Api => {
                let path = format!("/api/topics/show.json?node_name={}", encode(node.name));
                let body = self.transport.request(&path, Endpoint::Api).await?;
                api1::parse_topics(&body)
            }
            IdSource::Feed => {
                let path = node_feed_path(node.name);
                let body = self.transport.request(&path, Endpoint::Feed).await?;
                parse_feed_body(&body, &FeedKind::Node(node.name.to_string()))
            }
        }
    }

    async fn node_feed(&self, name: &str, ttl: Duration) -> Result<Vec<Topic>> {
        let kind = FeedKind::Node(name.to_string());
        self.feed_topics(&node_feed_path(name), &kind, ttl).await
    }

    async fn api_topics(&self, path: &str, ttl: Duration) -> Result<Vec<Topic>> {
        self.fetch(path, Endpoint::Api, ttl, api1::parse_topics)
            .await
    }

    async fn feed_topics(&self, path: &str, kind: &FeedKind, ttl: Duration) -> Result<Vec<Topic>> {
        self.fetch(path, Endpoint::Feed, ttl, |body| {
            parse_feed_body(body, kind)
        })
        .await
    }

    /// Fetches `path` and parses the body. A body that does not parse is dropped from the cache,
    /// so the next call asks the server again.
    async fn fetch<T>(
        &self,
        path: &str,
        endpoint: Endpoint,
        ttl: Duration,
        parse: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        let body = self.transport.get(path, endpoint, ttl).await?;
        parse(&body).inspect_err(|_| self.transport.forget(path))
    }
}

impl ClientBuilder {
    /// The site's root URL. Defaults to `https://www.v2ex.com`.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.settings.base_url = url.into();
        self
    }

    /// Routes requests through this http, https or socks5 proxy. Without it the proxy
    /// environment variables apply.
    pub fn proxy(mut self, url: impl Into<String>) -> Self {
        self.settings.proxy = Some(url.into());
        self
    }

    /// Per-request timeout. Defaults to 10 seconds.
    pub fn timeout(mut self, d: Duration) -> Self {
        self.settings.timeout = d;
        self
    }

    /// Defaults to `v2ex-rs/<version>`.
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.settings.user_agent = ua.into();
        self
    }

    /// Fails with [`Error::Config`] on an invalid base URL or proxy.
    pub fn build(self) -> Result<Client> {
        Ok(Client {
            transport: Transport::new(&self.settings)?,
            restricted_ids: Arc::default(),
            tab_turn: Arc::default(),
        })
    }
}

fn node_feed_path(name: &str) -> String {
    format!("/feed/{}.xml", encode(name))
}

/// Parses a feed body. The site answers a tab it does not have with 200 and an empty body.
fn parse_feed_body(body: &[u8], kind: &FeedKind) -> Result<Vec<Topic>> {
    if body.is_empty() {
        return Err(Error::NotFound);
    }
    feed::parse_feed(body, kind)
}

/// Escapes everything but the RFC 3986 unreserved characters, so a name stays inside the path
/// segment or query value it is placed in.
fn encode(name: &str) -> PercentEncode<'_> {
    utf8_percent_encode(name, NAME_ESCAPES)
}

fn is_restricted(topic: &Topic) -> bool {
    topic
        .node
        .as_ref()
        .is_some_and(|n| restricted::of(n).is_some())
}

fn restricted_error(node: &str) -> Error {
    Error::Restricted {
        node: node.to_string(),
    }
}
