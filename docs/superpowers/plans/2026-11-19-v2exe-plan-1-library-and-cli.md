# v2exe 实施计划一：数据层与非互动式

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建好 workspace、`v2ex-rs` 库、正文渲染和非互动式子命令，让 `v2exe hot / latest / tab / node / topic / nodes` 从站点取数并输出文本或 JSON。

**Architecture:** `crates/v2ex-rs` 是不含界面代码的库：一个 `Transport` 负责请求、缓存、限流和重试，`api1` 与 `feed` 两个模块把响应解析成公共类型，`Client` 按 spec §6.2 选来源并执行 §6.5 的受限节点规则。`crates/v2exe` 同时有 lib 与 bin：`render` 把正文 HTML 转成带语义角色的行，`cli` 解析参数、合并配置、调用库并输出。互动式界面、我的最爱、已读状态在实施计划二，本计划落地后再写。

**Tech Stack:** Rust 2024 edition。库：tokio、reqwest（rustls，不用 OpenSSL）、serde、serde_json、chrono、thiserror、feed-rs、bytes。程序：clap（derive）、toml、scraper、unicode-width。测试：wiremock、assert_cmd、tempfile。版本取实现时 crates.io 上的最新稳定版。

**Spec:** `docs/superpowers/specs/2026-11-19-v2exe-design.md`

## Global Constraints

- 依赖方向只有 `v2exe` → `v2ex-rs`。`v2ex-rs` 不含界面、命令行、终端颜色、文件读写代码。
- 只支持 Linux 与 macOS，从源码安装（`cargo install --path crates/v2exe`）。不做预编译二进制、crates.io 发布、Windows 支持。
- `base_url` 默认 `https://www.v2ex.com`；超时默认 10 秒；代理默认取环境变量；库的默认 User-Agent 为 `v2ex-rs/<库版本>`，程序设为 `v2exe/<程序版本>`。
- 缓存有效期：列表与帖子 120 秒，回复 60 秒，单个节点 1 小时，节点目录 2 小时，受限帖子编号 10 分钟。
- 重试：连接失败、超时、5xx 最多重试 2 次，带退避；其余错误不重试；被限流不重试。
- 受限节点：`flamewar`（水深火热，来源 API 1.0 `topics/show.json?node_name=`）、`nsfw`（NSFW，来源节点 feed）、`sandbox`（沙盒，来源节点 feed）。
- 颜色只用终端 16 色。`--no-color`、环境变量 `NO_COLOR`、stdout 不是终端，三者任一成立即关闭颜色。
- 退出码：0 成功；1 其他错误；2 用法错误；3 找不到；4 被限流；5 需要登录；6 网络错误。错误信息写 stderr，格式 `v2exe: <信息>`。
- 配置文件 `$XDG_CONFIG_HOME/v2exe/config.toml`，未设该变量时 `~/.config/v2exe/config.toml`，macOS 同样如此。文件缺失时取默认值，程序不创建它。
- 面向用户的文字用中文。
- 测试夹具保留真实响应的字段集合与格式（API 时间是 epoch 秒；Atom 正文是 CDATA；链接形如 `https://www.v2ex.com/t/<id>#reply<N>`），用户名与正文用示例内容，因为本仓公开。
- 贴近移植参考项目的代码时，在同一个 commit 里创建或更新 `THIRD-PARTY-NOTICES`，写入对方的版权与 MIT 许可声明。
- 每个任务结束时 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全部通过。Commit message 用英文 Conventional Commits。

## Review Focus

1. **下游提前关闭管道**（`v2exe hot | head -1`）：安静结束、退出码 0，stderr 没有 panic 信息。测试在 Task 8。
2. **没有断点、比可用宽度还长的串**（长网址、长代码行）：硬折断，每一行都不超过宽度，拼回去与原文相同。测试在 Task 6。
3. **只有标题、零回复的帖子**：文本输出不留空段落和回复区，`--json` 的 `replies` 为 `[]`。测试在 Task 8。
4. **配置文件写错**（TOML 语法错、字段类型错）：报出文件路径与原因，退出码 1，不 panic；文件不存在时用默认值。测试在 Task 7 与 Task 8。
5. **帖子引用的各种写法**：`123`、`https://www.v2ex.com/t/123#reply5`、`v2ex.com/t/123?p=2`、`/t/123` 都解析为 123；`abc`、`/t/`、`0` 是用法错误。测试在 Task 7 与 Task 8。

---

### Task 1: Workspace 与库的公共类型

**Files:**
- Create: `Cargo.toml`（workspace，`members = ["crates/*"]`，`resolver = "3"`；`[workspace.package]` 写 `edition = "2024"`、`license = "MIT"`、`repository = "https://github.com/enriquephl/v2exe"`）
- Create: `.gitignore`（`/target`）
- Create: `LICENSE`（MIT 全文，版权行 `Copyright (c) 2026 enriquephl`）
- Create: `.github/workflows/ci.yml`
- Create: `crates/v2ex-rs/Cargo.toml`、`crates/v2ex-rs/src/lib.rs`、`crates/v2ex-rs/src/types.rs`、`crates/v2ex-rs/src/error.rs`
- Modify: `AGENTS.md` 的「目录地图」表，加一行 `crates/v2ex-rs/` → 数据层库：类型、来源、限流、缓存
- Test: `crates/v2ex-rs/tests/types.rs`

**Interfaces:**
- Produces（`v2ex_rs::` 下公开）：spec §6.3 的 `Topic`、`Reply`、`Node`、`MemberRef`、`NodeRef`、`RateLimit`，字段与类型照抄 spec；§6.1 的 `Listing`、`Page<T>`。全部 derive `Debug, Clone, PartialEq, Serialize, Deserialize`；时间类型 `chrono::DateTime<Utc>`，serde 输出 RFC 3339。
- Produces：`pub enum Error`，即 spec §6.4 的六个变体再加 `Config(String)`（无效的 base_url 或代理）；`Network(String)`、`Decode(String)` 携带说明文字；`RateLimited { reset: Option<DateTime<Utc>> }`；`Restricted { node: String }`。`pub type Result<T> = std::result::Result<T, Error>`。用 thiserror derive，Display 用英文。

- [ ] **Step 1: 写失败的测试**

```rust
// crates/v2ex-rs/tests/types.rs
#[test]
fn topic_serializes_with_rfc3339_times_and_null_options() {
    let t = Topic {
        id: 1001, title: "示例标题一".into(), url: "https://www.v2ex.com/t/1001".into(),
        author: MemberRef { username: "alice".into(), id: Some(7) },
        node: None,
        created: Utc.timestamp_opt(1795050000, 0).unwrap(),
        last_touched: None, replies: 3, last_reply_by: None, content: None, content_html: None,
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
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2ex-rs --test types`
Expected: 编译失败，`Topic` 等未定义。

- [ ] **Step 3: 实现 `types.rs`、`error.rs`，在 `lib.rs` 里 `pub use` 它们**

`crates/v2ex-rs/Cargo.toml`：`name = "v2ex-rs"`，继承 workspace 的 edition 与 license；依赖 serde（derive）、chrono（serde）、thiserror；dev 依赖 serde_json。

- [ ] **Step 4: 写 CI**

`.github/workflows/ci.yml`：push 与 pull_request 触发；矩阵 `ubuntu-latest`、`macos-latest`；步骤为 checkout、安装 stable 工具链（含 rustfmt、clippy）、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。

- [ ] **Step 5: 运行，确认通过**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: 两个测试 PASS，无警告。

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml .gitignore LICENSE .github/workflows/ci.yml crates/v2ex-rs AGENTS.md
git commit -m "feat(v2ex-rs): add workspace and public data types"
```

---

### Task 2: API 1.0 响应解析

**Files:**
- Create: `crates/v2ex-rs/src/api1.rs`（`mod api1;` 加进 `lib.rs`，不公开）
- Create: `crates/v2ex-rs/tests/fixtures/api1/topics.json`、`topic.json`、`topic_restricted.json`、`topic_empty.json`、`replies.json`、`node.json`、`nodes.json`
- Test: `api1.rs` 内的 `#[cfg(test)] mod tests`，用 `include_bytes!("../tests/fixtures/api1/...")` 读夹具

**Interfaces:**
- Consumes: Task 1 的类型与 `Error`。
- Produces（`pub(crate)`）：
  - `fn parse_topics(body: &[u8]) -> Result<Vec<Topic>>`：hot、latest、`show.json?id=`、`show.json?node_name=` 都返回数组，共用它。
  - `fn parse_replies(body: &[u8]) -> Result<Vec<Reply>>`：`floor` 为数组下标加 1。
  - `fn parse_node(body: &[u8]) -> Result<Node>`
  - `fn parse_nodes(body: &[u8]) -> Result<Vec<Node>>`

映射规则：epoch 秒转 `DateTime<Utc>`；`member` → `MemberRef { username, id: Some(id) }`；`node` → `NodeRef { name: Some(name), title }`；`content_rendered` → `content_html`；`content`、`content_rendered`、`last_reply_by`、`header`、`title_alternative` 为空串或 null 时为 `None`；未知字段忽略；JSON 不合法或缺必需字段 → `Error::Decode`。

**夹具**（字段集合照真实响应；只列与测试相关的值，其余字段照真实格式填示例值）：
- `topics.json`：三条。
  - 1001：标题「示例标题一」，节点 `python` / `Python`，作者 `alice`（id 7），`created` 1795050000，`last_touched` 1795057200，`last_reply_by` `"bob"`，`replies` 3，`content` `"第一段"`，`content_rendered` `"<p>第一段</p>"`。
  - 1002：节点 `flamewar` / `水深火热`，作者 `carol`，`replies` 0，`last_reply_by` `""`，`content` 与 `content_rendered` 都是 `""`。
  - 1003：标题「示例标题三」，节点 `qna` / `问与答`，作者 `dave`，`created` 1795057500，`replies` 120。
  - 每条的 `url` 都是 `https://www.v2ex.com/t/<id>`。
- `topic.json`：只含 1001 的数组。
- `topic_restricted.json`：只含 1002 的数组。
- `topic_empty.json`：只含一条的数组：1004，标题「只有标题」，节点 `qna` / `问与答`，作者 `carol`，`created` 1795050000，`replies` 0，`content` 与 `content_rendered` 为 `""`。
- `replies.json`：两条。5001：作者 `bob`（id 8），`content` `"@alice 同意"`，`content_rendered` `"@<a href=\"/member/alice\">alice</a> 同意"`，`created` 1795055400。5002：作者 `erin`（id 9），`content` 与 `content_rendered` 为 `"第二则"`，`created` 1795057200。
- `node.json`：`python`，标题 `Python`，`title_alternative` `"Python"`，`topics` 16302，`stars` 9000，`header` `"这里讨论 Python"`，`parent_node_name` `"programming"`。
- `nodes.json`：`python`（同上）、`flamewar`（水深火热）、`qna`（问与答，`header` 与 `parent_node_name` 为 null）。

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn parse_topics_maps_fields() {
    let t = &parse_topics(TOPICS).unwrap()[0];
    assert_eq!((t.id, t.title.as_str(), t.replies), (1001, "示例标题一", 3));
    assert_eq!(t.author, MemberRef { username: "alice".into(), id: Some(7) });
    assert_eq!(t.node, Some(NodeRef { name: Some("python".into()), title: "Python".into() }));
    assert_eq!(t.created, Utc.timestamp_opt(1795050000, 0).unwrap());
    assert_eq!(t.last_touched, Some(Utc.timestamp_opt(1795057200, 0).unwrap()));
    assert_eq!(t.last_reply_by.as_deref(), Some("bob"));
    assert_eq!(t.content.as_deref(), Some("第一段"));
    assert_eq!(t.content_html.as_deref(), Some("<p>第一段</p>"));
}
#[test] fn parse_topics_turns_empty_strings_into_none() {
    let t = &parse_topics(TOPICS).unwrap()[1];
    assert_eq!((t.last_reply_by.clone(), t.content.clone(), t.content_html.clone()), (None, None, None));
}
#[test] fn parse_topics_accepts_empty_array() { assert!(parse_topics(b"[]").unwrap().is_empty()); }
#[test] fn parse_topics_rejects_html() { assert!(matches!(parse_topics(b"<html>"), Err(Error::Decode(_)))); }
#[test] fn parse_replies_numbers_floors() {
    let r = parse_replies(REPLIES).unwrap();
    assert_eq!(r.iter().map(|r| r.floor).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(r[0].author, MemberRef { username: "bob".into(), id: Some(8) });
    assert!(r[0].content_html.contains("href=\"/member/alice\""));
}
#[test] fn parse_node_and_nodes() {
    let n = parse_node(NODE).unwrap();
    assert_eq!((n.name.as_str(), n.topics, n.header.as_deref()), ("python", 16302, Some("这里讨论 Python")));
    let all = parse_nodes(NODES).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!((all[2].header.clone(), all[2].parent_node_name.clone()), (None, None));
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2ex-rs api1`
Expected: 编译失败，函数未定义。

- [ ] **Step 3: 实现 `api1.rs`**

用 serde 的私有原始结构体反序列化，再转换成公共类型。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p v2ex-rs api1`
Expected: 6 个测试 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/v2ex-rs
git commit -m "feat(v2ex-rs): parse API 1.0 topics, replies and nodes"
```

---

### Task 3: Atom feed 解析

**Files:**
- Create: `crates/v2ex-rs/src/feed.rs`（不公开）
- Create: `crates/v2ex-rs/tests/fixtures/feed/node.xml`、`tab_hot.xml`、`tab.xml`
- Test: `feed.rs` 内的单元测试

**Interfaces:**
- Consumes: Task 1 的类型。
- Produces（`pub(crate)`）：
  - `enum FeedKind { Node(String), Prefixed, Plain }`：`Node(name)` 用于节点 feed，`Prefixed` 用于 `/feed/tab/hot.xml`，`Plain` 用于其余 tab feed。
  - `fn parse_feed(body: &[u8], kind: &FeedKind) -> Result<Vec<Topic>>`

映射规则：
- `id`：取 entry 链接里 `/t/<数字>`；`url`：链接去掉 `#` 之后的部分；`replies`：链接的 `#reply<N>`，没有片段为 0。
- `author`：`MemberRef { username: <author name>, id: None }`；`created` = `<published>`；`last_touched` = `Some(<updated>)`；`content_html` = `Some(<content>)`；`content` 与 `last_reply_by` 为 `None`。
- `Node(name)`：`node = Some(NodeRef { name: Some(name), title: <feed 的 title> })`，标题原样。
- `Prefixed`：标题以 `[` 开头且第一个 `]` 后面紧跟一个空格时，括号内是节点标题，拆成 `node = Some(NodeRef { name: None, title })`，`title` 为空格之后的部分；否则 `node = None`、标题原样。
- `Plain`：`node = None`，标题原样（方括号是作者自己写的）。
- 链接里找不到 `/t/<数字>` 的 entry 跳过；不是 feed → `Error::Decode`。

**夹具**：entry 照下面的真实格式写（链接、`<published>`、`<updated>`、author、CDATA 正文）：

```xml
<entry>
  <title>示例标题</title>
  <link rel="alternate" type="text/html" href="https://www.v2ex.com/t/2001#reply16" />
  <id>tag:www.v2ex.com,2026-11-19:/t/2001</id>
  <published>2026-11-19T01:00:00Z</published>
  <updated>2026-11-19T03:00:00Z</updated>
  <author><name>alice</name><uri>https://www.v2ex.com/member/alice</uri></author>
  <content type="html" xml:base="https://www.v2ex.com/" xml:lang="en"><![CDATA[<p>正文</p>]]></content>
</entry>
```

- `node.xml`：feed `<title>Python</title>`；entry 2001（`#reply16`）、2002（`#reply0`）、2003（链接不带片段）。
- `tab_hot.xml`：feed `<title>V2EX - 最热 - 2026-11-18</title>`；entry 3001「[Python] 标题甲」、3002「[🤖Vibe Coding] [求助] 标题乙」、3003「[水深火热] 标题丙」。
- `tab.xml`：feed `<title>V2EX - 全部</title>`；entry 4001「[拼团] 作者自己的方括号」、4002「普通标题」、1002「被挡住的帖子」。

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn node_feed_maps_entries() {
    let t = parse_feed(NODE_XML, &FeedKind::Node("python".into())).unwrap();
    assert_eq!(t.iter().map(|t| t.id).collect::<Vec<_>>(), vec![2001, 2002, 2003]);
    assert_eq!(t.iter().map(|t| t.replies).collect::<Vec<_>>(), vec![16, 0, 0]);
    assert_eq!(t[0].url, "https://www.v2ex.com/t/2001");
    assert_eq!(t[0].node, Some(NodeRef { name: Some("python".into()), title: "Python".into() }));
    assert_eq!(t[0].created.to_rfc3339(), "2026-11-19T01:00:00+00:00");
    assert_eq!(t[0].last_touched.unwrap().to_rfc3339(), "2026-11-19T03:00:00+00:00");
    assert_eq!(t[0].author, MemberRef { username: "alice".into(), id: None });
    assert_eq!((t[0].content.clone(), t[0].content_html.as_deref()), (None, Some("<p>正文</p>")));
}
#[test] fn prefixed_feed_splits_node_title() {
    let t = parse_feed(TAB_HOT_XML, &FeedKind::Prefixed).unwrap();
    assert_eq!(t.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["标题甲", "[求助] 标题乙", "标题丙"]);
    assert_eq!(t.iter().map(|t| t.node.clone().unwrap().title).collect::<Vec<_>>(), vec!["Python", "🤖Vibe Coding", "水深火热"]);
    assert!(t.iter().all(|t| t.node.as_ref().unwrap().name.is_none()));
}
#[test] fn plain_feed_keeps_author_brackets() {
    let t = parse_feed(TAB_XML, &FeedKind::Plain).unwrap();
    assert_eq!(t[0].title, "[拼团] 作者自己的方括号");
    assert!(t.iter().all(|t| t.node.is_none()));
}
#[test] fn parse_feed_rejects_json() { assert!(matches!(parse_feed(b"[]", &FeedKind::Plain), Err(Error::Decode(_)))); }
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2ex-rs feed`
Expected: 编译失败。

- [ ] **Step 3: 实现 `feed.rs`**

用 feed-rs 解析 Atom；链接与标题前缀用字符串处理，不引入 regex。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p v2ex-rs feed`
Expected: 4 个测试 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/v2ex-rs
git commit -m "feat(v2ex-rs): parse node and tab Atom feeds"
```

---

### Task 4: 传输层：请求、缓存、限流、重试

**Files:**
- Create: `crates/v2ex-rs/src/http.rs`（不公开）
- Test: `http.rs` 内的 `#[tokio::test]`，用 wiremock

**Interfaces:**
- Consumes: Task 1 的 `Error`、`RateLimit`。
- Produces（`pub(crate)`）：

```rust
pub(crate) struct Settings { pub base_url: String, pub proxy: Option<String>, pub timeout: Duration, pub user_agent: String }
pub(crate) enum Endpoint { Api, Feed }
pub(crate) struct Transport { /* reqwest::Client、base_url、缓存、限流状态；可 clone，内部共享 */ }
impl Transport {
    pub(crate) fn new(s: &Settings) -> Result<Transport>;          // 无效 base_url 或代理 → Error::Config
    pub(crate) async fn get(&self, path: &str, kind: Endpoint, ttl: Duration) -> Result<bytes::Bytes>;
    pub(crate) fn rate_limit(&self) -> Option<RateLimit>;
    pub(crate) fn clear_cache(&self);
}
```

行为：
- `path` 含查询串，拼在 `base_url` 后面；缓存键就是 `path`。只缓存成功的响应体，有效期到了才重新请求。
- `Endpoint::Api` 的响应带 `x-rate-limit-limit`、`x-rate-limit-remaining`、`x-rate-limit-reset`（epoch 秒）时更新限流状态。发 `Api` 请求前，若记录的余量为 0 且当前时间早于恢复时间，不发请求，返回 `RateLimited { reset: Some(..) }`。`Endpoint::Feed` 不读也不查限流状态。
- 状态码映射：404 → `NotFound`；429，或其他非 2xx 且响应头余量为 `0` → `RateLimited`（恢复时间取响应头，没有则 `None`），不重试；5xx → 退避后重试，最多重试 2 次，仍失败 → `Status(code)`；其余非 2xx → `Status(code)`。
- 连接失败与超时 → 同样最多重试 2 次，仍失败 → `Network(说明)`。退避间隔为 250 毫秒乘以第几次重试。
- 每个请求带 `Settings.user_agent`；没有显式代理时用 reqwest 的环境变量代理。

- [ ] **Step 1: 写失败的测试**

```rust
// 每个测试起一个 MockServer，Settings { base_url: server.uri(), proxy: None, timeout: 2s, user_agent: "v2ex-rs-test/1" }
#[tokio::test] async fn caches_within_ttl() { /* mock /a 200 .expect(1)；get 两次 ttl 60s；两次 body 相同 */ }
#[tokio::test] async fn clear_cache_refetches() { /* mock /a .expect(2)；get、clear_cache、get */ }
#[tokio::test] async fn errors_are_not_cached() { /* /a 先 404（up_to_n_times(1)）后 200；第一次 Err(NotFound)，第二次 Ok */ }
#[tokio::test] async fn records_rate_limit_headers() {
    // /api/x 200 带头 600 / 571 / 1795060800
    // assert_eq!(t.rate_limit(), Some(RateLimit { limit: 600, remaining: 571, reset: Utc.timestamp_opt(1795060800, 0).unwrap() }));
}
#[tokio::test] async fn feed_does_not_touch_rate_limit() { /* Feed 请求带同样的头；rate_limit() == None */ }
#[tokio::test] async fn exhausted_quota_short_circuits() {
    // /api/a 200 带 remaining 0、reset = 现在 + 3600；/api/b .expect(0)
    // get(/api/b, Api) → Err(RateLimited { reset: Some(_) })
}
#[tokio::test] async fn status_429_is_rate_limited_without_retry() { /* .expect(1) → RateLimited */ }
#[tokio::test] async fn rejection_with_zero_remaining_is_rate_limited() { /* 403 + remaining 0 → RateLimited */ }
#[tokio::test] async fn status_404_is_not_found() { /* .expect(1) → NotFound */ }
#[tokio::test] async fn server_errors_retry_then_succeed() { /* 503 up_to_n_times(2)，再 200 → Ok */ }
#[tokio::test] async fn server_errors_give_up_after_two_retries() { /* 503 .expect(3) → Status(503) */ }
#[tokio::test] async fn timeout_is_network_error() { /* 200 set_delay(1s)，timeout 200ms → Network(_) */ }
#[tokio::test] async fn closed_port_is_network_error() { /* base_url "http://127.0.0.1:9" → Network(_) */ }
#[tokio::test] async fn sends_user_agent() { /* mock 匹配 header("user-agent", "v2ex-rs-test/1") */ }
#[tokio::test] async fn invalid_proxy_is_config_error() { /* proxy Some("not a url") → Transport::new 返回 Err(Config(_)) */ }
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2ex-rs http`
Expected: 编译失败。

- [ ] **Step 3: 实现 `http.rs`**

缓存用 `Mutex<HashMap<String, (Instant, Bytes)>>`；限流状态用 `Mutex<Option<RateLimit>>`；二者放进 `Arc` 让 `Transport` 可 clone。依赖加 tokio（rt、macros、time、sync）、reqwest（rustls）、bytes；dev 依赖加 wiremock。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p v2ex-rs http`
Expected: 15 个测试 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/v2ex-rs
git commit -m "feat(v2ex-rs): add transport with cache, rate limit and retries"
```

---

### Task 5: `Client` 公开接口与受限节点

**Files:**
- Create: `crates/v2ex-rs/src/client.rs`、`crates/v2ex-rs/src/restricted.rs`
- Modify: `crates/v2ex-rs/src/lib.rs`（`pub use client::{Client, ClientBuilder}`）
- Test: `crates/v2ex-rs/tests/client.rs`（只用公开接口；夹具来自 Task 2、3）

**Interfaces:**
- Consumes: Task 2 的 `parse_*`，Task 3 的 `parse_feed`、`FeedKind`，Task 4 的 `Transport`、`Settings`、`Endpoint`。
- Produces（公开，签名照 spec §6.1）：

```rust
impl Client {
    pub fn builder() -> ClientBuilder;
    pub async fn topics(&self, listing: &Listing, page: u32) -> Result<Page<Topic>>;
    pub async fn topic(&self, id: u64) -> Result<Topic>;
    pub async fn replies(&self, topic_id: u64, page: u32) -> Result<Page<Reply>>;
    pub async fn node(&self, name: &str) -> Result<Node>;
    pub async fn nodes(&self) -> Result<Vec<Node>>;
    pub fn rate_limit(&self) -> Option<RateLimit>;
    pub fn clear_cache(&self);
}
impl ClientBuilder {
    pub fn base_url(self, url: impl Into<String>) -> Self;
    pub fn proxy(self, url: impl Into<String>) -> Self;
    pub fn timeout(self, d: Duration) -> Self;
    pub fn user_agent(self, ua: impl Into<String>) -> Self;
    pub fn build(self) -> Result<Client>;
}
```

- `restricted.rs`：`pub(crate) struct RestrictedNode { name: &'static str, title: &'static str, source: IdSource }`，`pub(crate) enum IdSource { Api, Feed }`，常量表照 Global Constraints 的三项。

各调用的路径与有效期：

| 调用 | 路径 | 来源 | 有效期 |
|---|---|---|---|
| `Hot` | `/api/topics/hot.json` | Api | 120s |
| `Latest` | `/api/topics/latest.json` | Api | 120s |
| `Tab("hot")` | `/feed/tab/hot.xml`，`FeedKind::Prefixed` | Feed | 120s |
| `Tab(t)` | `/feed/tab/{t}.xml`，`FeedKind::Plain` | Feed | 120s |
| `Node(n)` | `/feed/{n}.xml`，`FeedKind::Node(n)` | Feed | 120s |
| `topic(id)` | `/api/topics/show.json?id={id}` | Api | 120s |
| `replies(id, 1)` | `/api/replies/show.json?topic_id={id}` | Api | 60s |
| `node(name)` | `/api/nodes/show.json?name={name}` | Api | 3600s |
| `nodes()` | `/api/nodes/all.json` | Api | 7200s |
| 受限编号（Api） | `/api/topics/show.json?node_name={name}` | Api | 600s |
| 受限编号（Feed） | `/feed/{name}.xml` | Feed | 600s |

规则（spec §6.1、§6.5）：
- `page != 1` 时不发请求，返回 `Page { items: vec![], page, has_more: false }`；`page == 1` 时 `has_more: false`。
- `Hot`、`Latest` 按 `node.name` 滤掉受限节点；`Tab("hot")` 按 `node.title` 滤。
- 其余 `Tab`：取完列表后，依次取三个受限节点的编号来源，合成编号集合，滤掉 `id` 在集合里的条目。某个来源出错就跳过它。
- `Node(受限)`、`node(受限)` 直接返回 `Restricted`，不发请求。
- `topic(id)`：空数组 → `NotFound`；节点是受限节点 → `Restricted`。
- `nodes()` 去掉受限节点。
- `ClientBuilder` 默认值：`https://www.v2ex.com`、10 秒、`v2ex-rs/<CARGO_PKG_VERSION>`。

- [ ] **Step 1: 写失败的测试**

```rust
// 每个测试：MockServer 按路径与查询串挂夹具；client = Client::builder().base_url(server.uri()).build()
#[tokio::test] async fn hot_drops_restricted_nodes() {
    // /api/topics/hot.json → topics.json
    let p = client.topics(&Listing::Hot, 1).await.unwrap();
    assert_eq!(ids(&p.items), vec![1001, 1003]);
    assert_eq!((p.page, p.has_more), (1, false));
}
#[tokio::test] async fn latest_uses_latest_endpoint() { /* /api/topics/latest.json → topics.json；ids [1001, 1003] */ }
#[tokio::test] async fn node_listing_reads_node_feed() { /* /feed/python.xml → node.xml；ids [2001, 2002, 2003] */ }
#[tokio::test] async fn restricted_node_listing_is_refused_without_request() {
    // 挂一个匹配任意路径、.expect(0) 的 mock
    assert!(matches!(client.topics(&Listing::Node("flamewar".into()), 1).await, Err(Error::Restricted { node }) if node == "flamewar"));
}
#[tokio::test] async fn hot_tab_drops_restricted_titles() {
    // /feed/tab/hot.xml → tab_hot.xml
    assert_eq!(titles(&p.items), vec!["标题甲", "[求助] 标题乙"]);
}
#[tokio::test] async fn plain_tab_drops_recent_restricted_ids_and_skips_failed_sources() {
    // /feed/tab/all.xml → tab.xml；/api/topics/show.json?node_name=flamewar → topics.json；
    // /feed/nsfw.xml → 404；/feed/sandbox.xml → node.xml
    assert_eq!(ids(&client.topics(&Listing::Tab("all".into()), 1).await.unwrap().items), vec![4001, 4002]);
}
#[tokio::test] async fn page_two_is_empty_without_request() { /* hot mock .expect(0)；topics(&Hot, 2) → items 空、page 2、has_more false */ }
#[tokio::test] async fn topic_by_id() { /* ?id=1001 → topic.json；id 1001 */ }
#[tokio::test] async fn unknown_topic_is_not_found() { /* ?id=999 → "[]"；Err(NotFound) */ }
#[tokio::test] async fn restricted_topic_is_refused() { /* ?id=1002 → topic_restricted.json；Err(Restricted { node: "flamewar" }) */ }
#[tokio::test] async fn replies_first_page_and_empty_second_page() { /* floors [1, 2]；page 2 → 空且不请求 */ }
#[tokio::test] async fn node_lookup_and_not_found() {
    // ?name=python → node.json；?name=nope → 404 + {"status": "error", "message": "Object Not Found"}
    // node("python") Ok；node("nope") Err(NotFound)；node("nsfw") Err(Restricted) 且不请求
}
#[tokio::test] async fn nodes_excludes_restricted() { /* all.json → nodes.json；names ["python", "qna"] */ }
#[tokio::test] async fn rate_limit_is_exposed() { /* hot 响应带三个限流头；调用后 client.rate_limit().unwrap().remaining == 571 */ }
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2ex-rs --test client`
Expected: 编译失败。

- [ ] **Step 3: 实现 `restricted.rs` 与 `client.rs`**

`Client` 内含一个 `Transport`，可廉价 clone。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p v2ex-rs`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/v2ex-rs
git commit -m "feat(v2ex-rs): add client with source selection and restricted-node rules"
```

---

### Task 6: 正文渲染

**Files:**
- Create: `crates/v2exe/Cargo.toml`（`name = "v2exe"`；依赖 `v2ex-rs = { path = "../v2ex-rs" }`、scraper、unicode-width）
- Create: `crates/v2exe/src/lib.rs`（`pub mod render;`）、`crates/v2exe/src/render.rs`
- Modify: `AGENTS.md` 的「目录地图」，加一行 `crates/v2exe/` → 程序：非互动式子命令、互动式界面、正文渲染
- Test: `crates/v2exe/tests/render.rs`

**Interfaces:**
- Produces（`v2exe::render`）：

```rust
pub enum Role { Plain, Bold, Link, Mention, Code, Image }
pub enum LineKind { Text, Code, Quote }
pub struct Seg { pub text: String, pub role: Role }
pub struct Line { pub kind: LineKind, pub segs: Vec<Seg> }
impl Line { pub fn text(&self) -> String }                // 各段文字拼接
pub fn render_html(html: &str) -> Vec<Line>;              // 逻辑行，不折行
pub fn wrap(lines: &[Line], width: usize) -> Vec<Line>;   // 按显示宽度折行
pub fn display_width(s: &str) -> usize;                   // 中日韩与 emoji 算 2
```
（`Role`、`LineKind`、`Seg`、`Line` derive `Debug, Clone, PartialEq`。）

`render_html` 规则：
- `<p>`、`<div>` 是块，块之间一个空行（`segs` 为空的 `Text` 行）；`<br>` 换行。连续空行合并为一行，首尾不留空行。
- 普通文字里的连续空白合并为一个空格，每行去掉首尾空白。
- `<pre>` 的每一行源文字成为一个 `Code` 行，单段 `Role::Code`，保留空格；行内 `<code>` 是 `Role::Code` 段。
- `<blockquote>` 里的行 `kind = Quote`。
- `<li>`：`<ul>` 里前缀 `• `，`<ol>` 里前缀 `1. `、`2. `……，各自一行。
- `<h1>`–`<h6>`、`<strong>`、`<b>` 是 `Role::Bold`。
- `href` 以 `/member/` 开头的链接是提及：它前面紧挨的 `@` 并入，成为一段 `Role::Mention` 的 `@名字`。
- 其他链接：文字与 `href` 相同时一段 `Role::Link`；不同时一段 `Role::Link`，文字为 `文字 (href)`。
- `<img>` 单独一行：一段 `Role::Image`，文字为 `[图片] <src>`。
- HTML 实体解码。

`wrap` 规则：每行显示宽度不超过 `width.max(2)`；优先在最后一个空格处断开（断开处的空格丢弃）；中日韩字符之间可断；没有断点时硬断；不拆字符；折出的行保留原行的 `kind` 与各段 `role`。

- [ ] **Step 1: 写失败的测试**

```rust
fn texts(l: &[Line]) -> Vec<String> { l.iter().map(Line::text).collect() }

#[test] fn paragraphs_and_breaks() {
    assert_eq!(texts(&render_html("<p>第一段</p><p>第二段<br>第二行</p>")), vec!["第一段", "", "第二段", "第二行"]);
}
#[test] fn pre_keeps_indentation() {
    let l = render_html("<pre><code>fn main() {\n    x();\n}</code></pre>");
    assert_eq!(texts(&l), vec!["fn main() {", "    x();", "}"]);
    assert!(l.iter().all(|l| l.kind == LineKind::Code));
}
#[test] fn mention_absorbs_at_sign() {
    let l = render_html(r#"@<a href="/member/alice">alice</a> 同意"#);
    assert_eq!(l[0].segs, vec![Seg { text: "@alice".into(), role: Role::Mention }, Seg { text: " 同意".into(), role: Role::Plain }]);
}
#[test] fn links() {
    assert_eq!(render_html(r#"<a href="https://a.example/x">https://a.example/x</a>"#)[0].segs,
               vec![Seg { text: "https://a.example/x".into(), role: Role::Link }]);
    assert_eq!(render_html(r#"<a href="https://a.example/x">文档</a>"#)[0].segs,
               vec![Seg { text: "文档 (https://a.example/x)".into(), role: Role::Link }]);
}
#[test] fn image_becomes_placeholder_line() {
    assert_eq!(render_html(r#"<img src="https://i.example/a.png">"#)[0].segs,
               vec![Seg { text: "[图片] https://i.example/a.png".into(), role: Role::Image }]);
}
#[test] fn entities_lists_and_quotes() {
    assert_eq!(texts(&render_html("<p>a &amp; b</p>")), vec!["a & b"]);
    assert_eq!(texts(&render_html("<ul><li>一</li><li>二</li></ul>")), vec!["• 一", "• 二"]);
    assert_eq!(render_html("<blockquote>引用</blockquote>")[0].kind, LineKind::Quote);
}
#[test] fn display_width_counts_wide_chars() { assert_eq!(display_width("中a🤖"), 5); }
#[test] fn wrap_cjk() {
    assert_eq!(texts(&wrap(&render_html("一二三四五"), 6)), vec!["一二三", "四五"]);
}
#[test] fn wrap_prefers_spaces() {
    assert_eq!(texts(&wrap(&render_html("hello world foo"), 11)), vec!["hello world", "foo"]);
}
#[test] fn wrap_hard_breaks_long_tokens() {   // Review Focus 2
    let url = format!("https://example.com/{}", "a".repeat(100));
    let out = wrap(&render_html(&url), 30);
    assert!(out.iter().all(|l| display_width(&l.text()) <= 30));
    assert_eq!(out.iter().map(Line::text).collect::<String>(), url);
}
#[test] fn wrap_keeps_roles_and_kind() {
    let out = wrap(&render_html("<pre>abcdefghij</pre>"), 4);
    assert!(out.len() == 3 && out.iter().all(|l| l.kind == LineKind::Code && l.segs[0].role == Role::Code));
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2exe --test render`
Expected: 编译失败。

- [ ] **Step 3: 实现 `render.rs`**

用 scraper 解析成 DOM 后递归遍历。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p v2exe --test render`
Expected: 11 个测试 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/v2exe AGENTS.md
git commit -m "feat(v2exe): render topic HTML into styled, wrapped lines"
```

---

### Task 7: 配置、命令行参数、帖子引用、错误信息

**Files:**
- Create: `crates/v2exe/src/config.rs`、`crates/v2exe/src/cli/mod.rs`、`cli/args.rs`、`cli/topic_ref.rs`、`cli/exit.rs`
- Modify: `crates/v2exe/src/lib.rs`（`pub mod config; pub mod cli;`），`Cargo.toml` 加 clap（derive）、toml、serde、chrono
- Test: 各文件内的单元测试

**Interfaces:**
- Consumes: `v2ex_rs::Error`。
- Produces：

```rust
// config.rs
#[derive(Debug, Default, PartialEq, Deserialize)]
pub struct Config { pub base_url: Option<String>, pub proxy: Option<String>, pub timeout: Option<u64> }   // timeout 单位秒；未知键忽略
pub fn config_path_from(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf>;
pub fn config_path() -> Option<PathBuf>;                                  // 用进程环境调用上面那个
pub fn load(path: &Path) -> Result<Config, ConfigError>;                 // 文件不存在 → Ok(Config::default())
pub struct ConfigError { pub path: PathBuf, pub reason: String }          // Display: "配置文件 {path} 有误：{reason}"

// cli/args.rs，clap derive；顶层设 arg_required_else_help = true
pub struct Cli {
    #[arg(long, global = true)] pub json: bool,
    #[arg(long, global = true)] pub no_color: bool,
    #[arg(long, global = true)] pub base_url: Option<String>,
    #[arg(long, global = true)] pub proxy: Option<String>,
    #[arg(long, global = true)] pub timeout: Option<u64>,
    #[command(subcommand)] pub command: Option<Command>,
}
pub enum Command {
    Hot { #[arg(long)] limit: Option<usize> },
    Latest { #[arg(long)] limit: Option<usize> },
    Tab { name: String, #[arg(long)] limit: Option<usize> },
    Node { name: String, #[arg(long)] limit: Option<usize> },
    Topic { reference: String, #[arg(long)] no_replies: bool },
    Nodes { keyword: Option<String>, #[arg(long)] limit: Option<usize> },
}

// cli/topic_ref.rs
pub fn parse_topic_ref(s: &str) -> Option<u64>;

// cli/exit.rs
pub fn exit_code(e: &v2ex_rs::Error) -> u8;
pub fn message(e: &v2ex_rs::Error, tz: &FixedOffset) -> String;
```

`config_path_from`：`xdg_config_home` 非空 → `<它>/v2exe/config.toml`；否则 `<home>/.config/v2exe/config.toml`；两者都没有 → `None`。

`parse_topic_ref`：整串是正整数 → 该数；否则找 `/t/` 后面紧跟的数字串（之后可以是 `#`、`?`、`/` 或结尾）。0 与找不到都返回 `None`。

`exit_code`：`NotFound` 3，`RateLimited` 4，`Restricted` 5，`Network` 6，`Config` 2，`Status`、`Decode` 1。

`message` 的文字：

| 错误 | 文字 |
|---|---|
| `NotFound` | `找不到这个帖子或节点` |
| `RateLimited { reset: Some(t) }` | `接口额度已用完，{t 换到 tz 后的 HH:MM} 恢复` |
| `RateLimited { reset: None }` | `接口额度已用完，稍后再试` |
| `Restricted { node }` | `节点 {node} 需要登录才能查看` |
| `Network(s)` | `网络错误：{s}` |
| `Status(c)` | `站点返回 HTTP {c}` |
| `Decode(s)` | `无法解析站点的响应：{s}` |
| `Config(s)` | `设置有误：{s}` |

- [ ] **Step 1: 写失败的测试**

```rust
// config.rs
#[test] fn missing_file_gives_defaults() { assert_eq!(load(&tmp.path().join("nope.toml")).unwrap(), Config::default()); }
#[test] fn reads_all_fields() {
    // 写入 "base_url = \"https://global.v2ex.co\"\nproxy = \"socks5://127.0.0.1:1080\"\ntimeout = 20\n"
    assert_eq!(load(&p).unwrap(), Config { base_url: Some("https://global.v2ex.co".into()), proxy: Some("socks5://127.0.0.1:1080".into()), timeout: Some(20) });
}
#[test] fn syntax_error_names_the_file() {   // Review Focus 4
    // 写入 "base_url = "
    let e = load(&p).unwrap_err().to_string();
    assert!(e.contains(&p.display().to_string()) && e.contains("有误"));
}
#[test] fn wrong_type_is_an_error() { /* 写入 "timeout = \"ten\"" → Err */ }
#[test] fn config_path_rules() {
    assert_eq!(config_path_from(Some("/x".as_ref()), Some("/h".as_ref())), Some("/x/v2exe/config.toml".into()));
    assert_eq!(config_path_from(Some("".as_ref()), Some("/h".as_ref())), Some("/h/.config/v2exe/config.toml".into()));
    assert_eq!(config_path_from(None, None), None);
}
// topic_ref.rs，Review Focus 5
#[test] fn topic_refs() {
    for (s, want) in [("123", Some(123)), ("https://www.v2ex.com/t/123#reply5", Some(123)),
                      ("v2ex.com/t/123?p=2", Some(123)), ("/t/123", Some(123)),
                      ("abc", None), ("/t/", None), ("0", None)] {
        assert_eq!(parse_topic_ref(s), want, "{s}");
    }
}
// exit.rs
#[test] fn exit_codes() {
    assert_eq!([exit_code(&Error::NotFound), exit_code(&Error::RateLimited { reset: None }),
                exit_code(&Error::Restricted { node: "x".into() }), exit_code(&Error::Network("x".into())),
                exit_code(&Error::Status(500)), exit_code(&Error::Config("x".into()))], [3, 4, 5, 6, 1, 2]);
}
#[test] fn rate_limited_message_uses_local_time() {
    let tz = FixedOffset::east_opt(8 * 3600).unwrap();
    let e = Error::RateLimited { reset: Some(Utc.timestamp_opt(1795060800, 0).unwrap()) };
    assert_eq!(message(&e, &tz), "接口额度已用完，12:00 恢复");
}
// args.rs
#[test] fn parses_topic_flags() {
    let c = Cli::try_parse_from(["v2exe", "topic", "123", "--no-replies", "--json"]).unwrap();
    assert!(c.json && matches!(c.command, Some(Command::Topic { ref reference, no_replies: true }) if reference == "123"));
}
#[test] fn bare_invocation_shows_help() {
    assert_eq!(Cli::try_parse_from(["v2exe"]).unwrap_err().kind(), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
}
#[test] fn clap_definition_is_valid() { Cli::command().debug_assert(); }
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2exe --lib`
Expected: 编译失败。

- [ ] **Step 3: 实现上述五个文件**

dev 依赖加 tempfile。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p v2exe --lib`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/v2exe
git commit -m "feat(v2exe): add config loading, CLI arguments and error mapping"
```

---

### Task 8: 输出格式与程序入口

**Files:**
- Create: `crates/v2exe/src/cli/output.rs`、`crates/v2exe/src/cli/run.rs`、`crates/v2exe/src/main.rs`
- Modify: `crates/v2exe/Cargo.toml`（tokio rt、serde_json、crossterm；dev 依赖 assert_cmd、wiremock、tempfile、tokio macros）
- Modify: `README.md`，把开头的提示改为「非互动式子命令已可用，互动式界面还在开发。下面写的是第一版要做到的样子。」
- Test: `output.rs` 内单元测试（颜色）；`crates/v2exe/tests/cli.rs`（端到端，夹具用 `include_str!("../../v2ex-rs/tests/fixtures/...")`）

**Interfaces:**
- Consumes: Task 5 的 `Client`，Task 6 的 `render_html`、`wrap`，Task 7 的全部。
- Produces：

```rust
// cli/output.rs（纯函数，结果以 "\n" 结尾）
pub struct Painter { pub color: bool }
pub fn topic_rows(topics: &[Topic], tz: &FixedOffset, p: &Painter) -> String;
pub fn topic_detail(topic: &Topic, replies: Option<&[Reply]>, width: Option<usize>, tz: &FixedOffset, p: &Painter) -> String;
pub fn node_rows(nodes: &[Node], p: &Painter) -> String;
pub fn json_topics(topics: &[Topic]) -> String;
pub fn json_topic(topic: &Topic, replies: Option<&[Reply]>) -> String;
pub fn json_nodes(nodes: &[Node]) -> String;

// cli/run.rs
pub struct RunEnv { pub config_path: Option<PathBuf>, pub color: bool, pub width: Option<usize>, pub tz: FixedOffset }
impl RunEnv { pub fn from_process() -> RunEnv; }   // color：未设 NO_COLOR 且 stdout 是终端；width：stdout 是终端时取终端宽度；tz：本地时区当前偏移
pub struct Failure { pub code: u8, pub message: String }
pub async fn run(cli: Cli, out: &mut dyn Write, env: &RunEnv) -> Result<(), Failure>;
```

`run` 的流程：读配置（失败 → code 1，信息为 `ConfigError` 的 Display）；参数覆盖配置；建 `Client`，User-Agent 为 `v2exe/<CARGO_PKG_VERSION>`；按子命令调用；`nodes` 的关键词对 `name`、`title`、`title_alternative` 做不分大小写的子串匹配；`--limit` 截断列表；`topic` 先 `topic(id)`，没有 `--no-replies` 时再 `replies(id, 1)`；引用解析失败 → code 2，信息 `无法识别的帖子：{reference}`；库错误 → `exit_code` 与 `message`；写 stdout 遇到 `BrokenPipe` → 当作成功返回 `Ok(())`，其余写错误 → code 1。颜色开关为 `env.color && !cli.no_color`。

`main.rs`：`Cli::parse()`（用法错误由 clap 以退出码 2 结束）；单线程 tokio runtime 执行 `run`，传入锁定的 stdout 与 `RunEnv::from_process()`；`Err(f)` 时 `eprintln!("v2exe: {}", f.message)` 并以 `f.code` 退出。

**文本格式**（颜色关闭时逐字如下；时间都按 `tz` 换算）：
- 列表每行：`format!("{id:>7}  {replies:>4}  {MM-DD HH:MM}  {author:<15}  {title}")`；时间取 `created`；有 `node` 时标题前加 `[{node.title}] `。
- `topic_detail`：

```
示例标题一
alice · Python · 2026-11-19 09:00 · 3 回复
https://www.v2ex.com/t/1001

第一段

#1 bob · 11-19 10:30
    @alice 同意

#2 erin · 11-19 11:00
    第二则
```

  - 第二行在没有节点时省略节点那一段。回复作者与帖子作者相同时，名字后加 ` 楼主`。
  - 正文没有 `content_html` 时省略正文段及其前面的空行。`replies` 为 `None` 或空时，不输出回复区。
  - 正文的 `Code` 行前加 4 个空格，`Quote` 行前加 `> `。回复内容整体再缩进 4 格。
  - `width` 为 `Some(w)` 时，正文按 `w` 折行，回复按 `w - 4` 折行；为 `None` 时不折行。
- 节点每行：`format!("{name:<20}  {topics:>6}  {title}")`。
- 颜色开启时：编号与时间暗色；回复数 100 以上红色粗体、10 到 99 黄色粗体、1 到 9 绿色；作者青色；节点标题与链接、提及蓝色；楼主标记洋红。去掉 ANSI 转义序列后与关闭颜色时逐字相同。
- JSON：`serde_json::to_string_pretty` 加换行。`json_topic` 有回复时为 `{"topic": …, "replies": […]}`；`replies` 为 `None` 时只有 `"topic"` 键。

- [ ] **Step 1: 写失败的测试**

```rust
// output.rs 单元测试
#[test] fn color_only_adds_escape_codes() {
    let plain = topic_rows(&topics, &tz8, &Painter { color: false });
    let colored = topic_rows(&topics, &tz8, &Painter { color: true });
    assert!(colored.contains("\x1b["));
    assert_eq!(strip_ansi(&colored), plain);   // strip_ansi 是测试里的辅助函数
}

// tests/cli.rs：每个测试起 MockServer；命令带 --base-url <uri>；
// 环境设 TZ=Asia/Taipei、XDG_CONFIG_HOME=<临时目录>，并移除 NO_COLOR
#[tokio::test] async fn hot_prints_rows() {
    // /api/topics/hot.json → topics.json，mock 要求 user-agent 为 concat!("v2exe/", env!("CARGO_PKG_VERSION"))
    let row = |id, n, t, a, title| format!("{id:>7}  {n:>4}  {t}  {a:<15}  {title}\n");
    let want = row(1001, 3, "11-19 09:00", "alice", "[Python] 示例标题一")
             + &row(1003, 120, "11-19 11:05", "dave", "[问与答] 示例标题三");
    // stdout == want，退出码 0
}
#[tokio::test] async fn hot_json() { /* --json hot：数组长 2，[0]["id"] == 1001，[0]["created"] == "2026-11-19T01:00:00Z" */ }
#[tokio::test] async fn limit_truncates() { /* hot --limit 1：stdout 只有一行 */ }
#[tokio::test] async fn topic_text() { /* topic 1001：stdout 与上面的示例块逐字相同 */ }
#[tokio::test] async fn topic_json_without_replies() {
    // --json topic https://www.v2ex.com/t/1001#reply2 --no-replies；replies 的 mock .expect(0)
    // JSON 有 "topic" 键、没有 "replies" 键
}
#[tokio::test] async fn title_only_topic_with_no_replies() {   // Review Focus 3
    // ?id=1004 → topic_empty.json；?topic_id=1004 → "[]"
    // 文本 stdout == "只有标题\ncarol · 问与答 · 2026-11-19 09:00 · 0 回复\nhttps://www.v2ex.com/t/1004\n"
    // --json 时 replies == []
}
#[tokio::test] async fn nodes_keyword_filters() { /* nodes py → 只有 python 一行 */ }
#[tokio::test] async fn exit_codes_end_to_end() {
    // topic 999（返回 "[]"）→ 3，stderr 含 "找不到"
    // node flamewar → 5；hot 返回 429 → 4；--base-url http://127.0.0.1:9 hot → 6
    // topic abc → 2；--proxy "not a url" hot → 2
}
#[tokio::test] async fn config_file_supplies_base_url_and_flags_override() {
    // config.toml 写 base_url = <mock uri>，不带 --base-url 也能取到数据
    // config 写一个关闭的端口、--base-url 给 mock uri，仍然成功
}
#[tokio::test] async fn malformed_config_exits_1() {   // Review Focus 4
    // config.toml 写 "base_url = "；退出码 1，stderr 含配置文件路径与 "有误"，不含 "panicked"
}
#[tokio::test] async fn closed_pipe_exits_quietly() {   // Review Focus 1
    // std::process::Command 运行 nodes，stdout piped；spawn 后立刻 drop child.stdout
    // 退出码 0，stderr 不含 "panicked"
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p v2exe`
Expected: 编译失败。

- [ ] **Step 3: 实现 `output.rs`、`run.rs`、`main.rs`**

ANSI 码直接写（`\x1b[2m` 暗色、`\x1b[1;31m` 等），不引入颜色库。终端宽度用 crossterm 的 `terminal::size()`。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: 全部 PASS。

- [ ] **Step 5: 对真实站点各跑一次**

Run: `cargo run -q -p v2exe -- hot && cargo run -q -p v2exe -- --json topic "$(cargo run -q -p v2exe -- hot --limit 1 | awk '{print $1}')" | head -20 && cargo run -q -p v2exe -- tab tech --limit 5 && cargo run -q -p v2exe -- nodes python`
Expected: 四条命令都输出内容、退出码 0；hot 里没有 flamewar、nsfw、sandbox 的帖子。总共约 5 次 API 请求。

- [ ] **Step 6: Commit**

```bash
git add crates/v2exe README.md
git commit -m "feat(v2exe): add non-interactive subcommands with text and JSON output"
```
