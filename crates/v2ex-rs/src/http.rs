//! HTTP transport: requests, response cache, quota tracking and retries.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use chrono::{DateTime, TimeZone, Utc};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;

use crate::{Error, RateLimit, Result};

/// Retries after the first attempt.
const MAX_RETRIES: u32 = 2;
/// The wait before retry `n` is `BACKOFF_STEP * n`.
const BACKOFF_STEP: Duration = Duration::from_millis(250);

const HEADER_LIMIT: &str = "x-rate-limit-limit";
const HEADER_REMAINING: &str = "x-rate-limit-remaining";
const HEADER_RESET: &str = "x-rate-limit-reset";

pub(crate) struct Settings {
    pub base_url: String,
    /// `None` uses the proxy environment variables.
    pub proxy: Option<String>,
    pub timeout: Duration,
    pub user_agent: String,
}

/// Which server interface a request goes to. Only `Api` carries and obeys the quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endpoint {
    Api,
    Feed,
}

/// A response body, and whether the call that returned it sent the request.
#[derive(Debug)]
pub(crate) struct Fetched {
    pub body: Bytes,
    /// False when the body came from the cache.
    pub fresh: bool,
}

/// Clones share one connection pool, one cache and one quota record.
#[derive(Clone)]
pub(crate) struct Transport {
    inner: Arc<Inner>,
}

struct Inner {
    client: reqwest::Client,
    /// Without a trailing slash.
    base_url: String,
    /// Path to expiry time and body.
    cache: Mutex<HashMap<String, (Instant, Bytes)>>,
    rate_limit: Mutex<Option<RateLimit>>,
}

/// How a single attempt failed.
enum Failure {
    Retry(Error),
    Fatal(Error),
}

impl Transport {
    pub(crate) fn new(s: &Settings) -> Result<Transport> {
        let base_url = reqwest::Url::parse(&s.base_url)
            .ok()
            .filter(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.has_host()
                    && u.query().is_none()
                    && u.fragment().is_none()
            })
            .map(|_| s.base_url.trim_end_matches('/').to_string())
            .ok_or_else(|| Error::Config(format!("invalid base URL: {}", s.base_url)))?;

        let mut builder = reqwest::Client::builder()
            .timeout(s.timeout)
            .user_agent(&s.user_agent);
        if let Some(proxy) = &s.proxy {
            let proxy = reqwest::Proxy::all(proxy)
                .map_err(|e| Error::Config(format!("invalid proxy: {}", describe(e))))?;
            builder = builder.proxy(proxy);
        }
        let client = builder.build().map_err(|e| Error::Config(describe(e)))?;

        Ok(Transport {
            inner: Arc::new(Inner {
                client,
                base_url,
                cache: Mutex::new(HashMap::new()),
                rate_limit: Mutex::new(None),
            }),
        })
    }

    /// Fetches `path` (with its query string) from the base URL. A successful body is served
    /// from the cache for `ttl`; failures are never cached. Storing a body also drops every
    /// entry that has expired.
    pub(crate) async fn get(&self, path: &str, kind: Endpoint, ttl: Duration) -> Result<Fetched> {
        if let Some(body) = self.cached(path) {
            return Ok(Fetched { body, fresh: false });
        }
        if kind == Endpoint::Api {
            self.check_quota()?;
        }

        let url = format!("{}{path}", self.inner.base_url);
        let mut retries = 0;
        loop {
            match self.attempt(&url, kind).await {
                Ok(body) => {
                    let now = Instant::now();
                    let mut cache = lock(&self.inner.cache);
                    cache.retain(|_, (expires, _)| *expires > now);
                    cache.insert(path.to_string(), (now + ttl, body.clone()));
                    return Ok(Fetched { body, fresh: true });
                }
                Err(Failure::Retry(_)) if retries < MAX_RETRIES => {
                    retries += 1;
                    tokio::time::sleep(BACKOFF_STEP * retries).await;
                }
                Err(Failure::Retry(e) | Failure::Fatal(e)) => return Err(e),
            }
        }
    }

    /// The quota reported by the most recent `Api` response.
    pub(crate) fn rate_limit(&self) -> Option<RateLimit> {
        lock(&self.inner.rate_limit).clone()
    }

    pub(crate) fn clear_cache(&self) {
        lock(&self.inner.cache).clear();
    }

    /// Drops the cached body of `path`, for a body that turned out to be unusable.
    pub(crate) fn forget(&self, path: &str) {
        lock(&self.inner.cache).remove(path);
    }

    fn cached(&self, path: &str) -> Option<Bytes> {
        let mut cache = lock(&self.inner.cache);
        match cache.get(path) {
            Some((expires, body)) if *expires > Instant::now() => Some(body.clone()),
            Some(_) => {
                cache.remove(path);
                None
            }
            None => None,
        }
    }

    /// Fails without a request while the recorded quota is used up and not yet restored.
    fn check_quota(&self) -> Result<()> {
        match lock(&self.inner.rate_limit).as_ref() {
            Some(rl) if rl.remaining == 0 && Utc::now() < rl.reset => Err(Error::RateLimited {
                reset: Some(rl.reset),
            }),
            _ => Ok(()),
        }
    }

    async fn attempt(&self, url: &str, kind: Endpoint) -> std::result::Result<Bytes, Failure> {
        let resp = self
            .inner
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Failure::Retry(Error::Network(describe(e))))?;
        let status = resp.status();
        let headers = resp.headers();

        if kind == Endpoint::Api
            && let Some(rl) = rate_limit_from(headers)
        {
            *lock(&self.inner.rate_limit) = Some(rl);
        }

        if status.is_success() {
            return resp
                .bytes()
                .await
                .map_err(|e| Failure::Retry(Error::Network(describe(e))));
        }
        if status == StatusCode::NOT_FOUND {
            return Err(Failure::Fatal(Error::NotFound));
        }
        let exhausted =
            kind == Endpoint::Api && parse_header::<u32>(headers, HEADER_REMAINING) == Some(0);
        if status == StatusCode::TOO_MANY_REQUESTS || exhausted {
            let reset = if kind == Endpoint::Api {
                reset_from(headers)
            } else {
                None
            };
            return Err(Failure::Fatal(Error::RateLimited { reset }));
        }
        let error = Error::Status(status.as_u16());
        if status.is_server_error() {
            Err(Failure::Retry(error))
        } else {
            Err(Failure::Fatal(error))
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn parse_header<T: FromStr>(headers: &HeaderMap, name: &str) -> Option<T> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn reset_from(headers: &HeaderMap) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(parse_header(headers, HEADER_RESET)?, 0)
        .single()
}

fn rate_limit_from(headers: &HeaderMap) -> Option<RateLimit> {
    Some(RateLimit {
        limit: parse_header(headers, HEADER_LIMIT)?,
        remaining: parse_header(headers, HEADER_REMAINING)?,
        reset: reset_from(headers)?,
    })
}

/// The error and its causes on one line, without the request URL.
fn describe(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeZone, Utc};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{Endpoint, Settings, Transport, lock};
    use crate::{Error, RateLimit};

    const TTL: Duration = Duration::from_secs(60);

    fn settings(base_url: String) -> Settings {
        Settings {
            base_url,
            proxy: None,
            timeout: Duration::from_secs(2),
            user_agent: "v2ex-rs-test/1".to_string(),
        }
    }

    fn transport(server: &MockServer) -> Transport {
        Transport::new(&settings(server.uri())).unwrap()
    }

    fn quota(template: ResponseTemplate, remaining: u32, reset: i64) -> ResponseTemplate {
        template
            .insert_header("x-rate-limit-limit", "600")
            .insert_header("x-rate-limit-remaining", remaining.to_string())
            .insert_header("x-rate-limit-reset", reset.to_string())
    }

    #[tokio::test]
    async fn caches_within_ttl() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        let first = t.get("/a", Endpoint::Feed, TTL).await.unwrap();
        let second = t.get("/a", Endpoint::Feed, TTL).await.unwrap();

        assert_eq!(first.body, second.body);
        assert_eq!(&first.body[..], b"hello");
    }

    #[tokio::test]
    async fn clear_cache_refetches() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
            .expect(2)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/a", Endpoint::Feed, TTL).await.unwrap();
        t.clear_cache();
        t.get("/a", Endpoint::Feed, TTL).await.unwrap();
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(404))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        assert!(matches!(
            t.get("/a", Endpoint::Feed, TTL).await,
            Err(Error::NotFound)
        ));
        assert_eq!(
            &t.get("/a", Endpoint::Feed, TTL).await.unwrap().body[..],
            b"ok"
        );
    }

    #[tokio::test]
    async fn expired_entries_are_refetched() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/a", Endpoint::Feed, Duration::ZERO).await.unwrap();
        t.get("/a", Endpoint::Feed, Duration::ZERO).await.unwrap();
    }

    #[tokio::test]
    async fn storing_an_entry_drops_expired_ones() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/a", Endpoint::Feed, Duration::ZERO).await.unwrap();
        t.get("/b", Endpoint::Feed, TTL).await.unwrap();

        assert_eq!(lock(&t.inner.cache).len(), 1);
    }

    #[tokio::test]
    async fn fresh_is_true_only_when_a_request_was_sent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        let first = t.get("/a", Endpoint::Feed, TTL).await.unwrap();
        let second = t.get("/a", Endpoint::Feed, TTL).await.unwrap();

        assert!(first.fresh);
        assert!(!second.fresh);
        assert_eq!(first.body, second.body);
    }

    #[tokio::test]
    async fn records_rate_limit_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/x"))
            .respond_with(quota(ResponseTemplate::new(200), 571, 1795060800))
            .mount(&server)
            .await;
        let t = transport(&server);

        assert_eq!(t.rate_limit(), None);
        t.get("/api/x", Endpoint::Api, TTL).await.unwrap();

        assert_eq!(
            t.rate_limit(),
            Some(RateLimit {
                limit: 600,
                remaining: 571,
                reset: Utc.timestamp_opt(1795060800, 0).unwrap(),
            })
        );
    }

    #[tokio::test]
    async fn feed_does_not_touch_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/feed/x"))
            .respond_with(quota(ResponseTemplate::new(200), 571, 1795060800))
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/feed/x", Endpoint::Feed, TTL).await.unwrap();

        assert_eq!(t.rate_limit(), None);
    }

    #[tokio::test]
    async fn exhausted_quota_short_circuits() {
        let server = MockServer::start().await;
        let reset = Utc::now().timestamp() + 3600;
        Mock::given(method("GET"))
            .and(path("/api/a"))
            .respond_with(quota(ResponseTemplate::new(200), 0, reset))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/b"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/api/a", Endpoint::Api, TTL).await.unwrap();
        let err = t.get("/api/b", Endpoint::Api, TTL).await.unwrap_err();

        assert!(matches!(
            err,
            Error::RateLimited { reset: Some(r) } if r.timestamp() == reset
        ));
    }

    #[tokio::test]
    async fn exhausted_quota_does_not_block_feeds_or_cached_paths() {
        let server = MockServer::start().await;
        let reset = Utc::now().timestamp() + 3600;
        Mock::given(method("GET"))
            .and(path("/api/a"))
            .respond_with(quota(ResponseTemplate::new(200), 0, reset))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/feed/b"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/api/a", Endpoint::Api, TTL).await.unwrap();

        t.get("/api/a", Endpoint::Api, TTL).await.unwrap();
        t.get("/feed/b", Endpoint::Feed, TTL).await.unwrap();
    }

    #[tokio::test]
    async fn exhausted_quota_past_reset_sends_request() {
        let server = MockServer::start().await;
        let reset = Utc::now().timestamp() - 10;
        Mock::given(method("GET"))
            .and(path("/api/a"))
            .respond_with(quota(ResponseTemplate::new(200), 0, reset))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/b"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/api/a", Endpoint::Api, TTL).await.unwrap();
        t.get("/api/b", Endpoint::Api, TTL).await.unwrap();
    }

    #[tokio::test]
    async fn status_429_is_rate_limited_without_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/a"))
            .respond_with(ResponseTemplate::new(429))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        let err = t.get("/api/a", Endpoint::Api, TTL).await.unwrap_err();

        assert!(matches!(err, Error::RateLimited { reset: None }));
    }

    #[tokio::test]
    async fn rejection_with_zero_remaining_is_rate_limited() {
        let server = MockServer::start().await;
        let reset = Utc::now().timestamp() + 3600;
        Mock::given(method("GET"))
            .and(path("/api/a"))
            .respond_with(quota(ResponseTemplate::new(403), 0, reset))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        let err = t.get("/api/a", Endpoint::Api, TTL).await.unwrap_err();

        assert!(matches!(
            err,
            Error::RateLimited { reset: Some(r) } if r.timestamp() == reset
        ));
    }

    #[tokio::test]
    async fn rejection_with_remaining_quota_is_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/a"))
            .respond_with(quota(ResponseTemplate::new(403), 5, 1795060800))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        let err = t.get("/api/a", Endpoint::Api, TTL).await.unwrap_err();

        assert!(matches!(err, Error::Status(403)));
    }

    #[tokio::test]
    async fn status_404_is_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/a"))
            .respond_with(quota(ResponseTemplate::new(404), 570, 1795060800))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        let err = t.get("/api/a", Endpoint::Api, TTL).await.unwrap_err();

        assert!(matches!(err, Error::NotFound));
        assert_eq!(t.rate_limit().map(|r| r.remaining), Some(570));
    }

    #[tokio::test]
    async fn server_errors_retry_then_succeed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        assert_eq!(
            &t.get("/a", Endpoint::Feed, TTL).await.unwrap().body[..],
            b"ok"
        );
    }

    #[tokio::test]
    async fn server_errors_give_up_after_two_retries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3)
            .mount(&server)
            .await;
        let t = transport(&server);

        let err = t.get("/a", Endpoint::Feed, TTL).await.unwrap_err();

        assert!(matches!(err, Error::Status(503)));
    }

    #[tokio::test]
    async fn other_client_errors_are_status_without_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        let err = t.get("/a", Endpoint::Feed, TTL).await.unwrap_err();

        assert!(matches!(err, Error::Status(403)));
    }

    #[tokio::test]
    async fn timeout_is_network_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(1)))
            .mount(&server)
            .await;
        let t = Transport::new(&Settings {
            timeout: Duration::from_millis(200),
            ..settings(server.uri())
        })
        .unwrap();

        let err = t.get("/a", Endpoint::Feed, TTL).await.unwrap_err();

        assert!(matches!(err, Error::Network(_)));
    }

    #[tokio::test]
    async fn closed_port_is_network_error() {
        let t = Transport::new(&settings("http://127.0.0.1:9".to_string())).unwrap();

        let err = t.get("/a", Endpoint::Feed, TTL).await.unwrap_err();

        assert!(matches!(err, Error::Network(_)));
    }

    #[tokio::test]
    async fn sends_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .and(header("user-agent", "v2ex-rs-test/1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/a", Endpoint::Feed, TTL).await.unwrap();
    }

    #[tokio::test]
    async fn query_string_is_part_of_the_path_and_cache_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        let t = transport(&server);

        t.get("/a?p=1", Endpoint::Feed, TTL).await.unwrap();
        t.get("/a?p=2", Endpoint::Feed, TTL).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].url.query(), Some("p=1"));
        assert_eq!(requests[1].url.query(), Some("p=2"));
    }

    #[tokio::test]
    async fn invalid_proxy_is_config_error() {
        let result = Transport::new(&Settings {
            proxy: Some("not a url".to_string()),
            ..settings("http://127.0.0.1:9".to_string())
        });

        assert!(matches!(result, Err(Error::Config(_))));
    }

    #[tokio::test]
    async fn invalid_base_url_is_config_error() {
        for url in [
            "not a url",
            "https://www.v2ex.com/#x",
            "https://www.v2ex.com/?x=1",
        ] {
            let result = Transport::new(&settings(url.to_string()));

            assert!(matches!(result, Err(Error::Config(_))), "{url}");
        }
    }
}
