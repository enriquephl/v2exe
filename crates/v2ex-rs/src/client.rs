//! The public entry point: which source serves each call, and the restricted-node rules.

use std::collections::HashSet;
use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, PercentEncode, utf8_percent_encode};

use crate::feed::{self, FeedKind};
use crate::http::{Endpoint, Settings, Transport};
use crate::restricted::{self, IdSource};
use crate::{Error, Listing, Node, Page, RateLimit, Reply, Result, Topic, api1};

const DEFAULT_BASE_URL: &str = "https://www.v2ex.com";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

const TOPICS_TTL: Duration = Duration::from_secs(120);
const REPLIES_TTL: Duration = Duration::from_secs(60);
const NODE_TTL: Duration = Duration::from_secs(3600);
const NODES_TTL: Duration = Duration::from_secs(7200);
const RESTRICTED_IDS_TTL: Duration = Duration::from_secs(600);

const NAME_ESCAPES: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Cheap to clone; clones share one connection pool, cache and quota record.
#[derive(Clone)]
pub struct Client {
    transport: Transport,
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

    /// The quota reported by the most recent API response.
    pub fn rate_limit(&self) -> Option<RateLimit> {
        self.transport.rate_limit()
    }

    /// Drops every cached response, so the next calls fetch afresh.
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
                // Only the hot tab names each topic's node, in a title prefix.
                let (path, kind) = if tab.eq_ignore_ascii_case("hot") {
                    ("/feed/tab/hot.xml".to_string(), FeedKind::Prefixed)
                } else {
                    (format!("/feed/tab/{}.xml", encode(tab)), FeedKind::Plain)
                };
                let mut topics = self.feed_topics(&path, &kind, TOPICS_TTL).await?;
                let hidden = self.restricted_ids().await;
                topics.retain(|t| !hidden.contains(&t.id));
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

    /// The ids of the recent topics of every restricted node. A source that fails is skipped.
    async fn restricted_ids(&self) -> HashSet<u64> {
        let mut ids = HashSet::new();
        for node in restricted::RESTRICTED {
            let topics = match node.source {
                IdSource::Api => {
                    let path = format!("/api/topics/show.json?node_name={}", encode(node.name));
                    self.api_topics(&path, RESTRICTED_IDS_TTL).await
                }
                IdSource::Feed => self.node_feed(node.name, RESTRICTED_IDS_TTL).await,
            };
            if let Ok(topics) = topics {
                ids.extend(topics.iter().map(|t| t.id));
            }
        }
        ids
    }

    async fn node_feed(&self, name: &str, ttl: Duration) -> Result<Vec<Topic>> {
        let path = format!("/feed/{}.xml", encode(name));
        self.feed_topics(&path, &FeedKind::Node(name.to_string()), ttl)
            .await
    }

    async fn api_topics(&self, path: &str, ttl: Duration) -> Result<Vec<Topic>> {
        self.fetch(path, Endpoint::Api, ttl, api1::parse_topics)
            .await
    }

    async fn feed_topics(&self, path: &str, kind: &FeedKind, ttl: Duration) -> Result<Vec<Topic>> {
        self.fetch(path, Endpoint::Feed, ttl, |body| {
            feed::parse_feed(body, kind)
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
        })
    }
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
