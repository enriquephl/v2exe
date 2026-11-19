# v2exe 实施计划二：互动式界面

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `v2exe` 不带子命令时进入互动式界面：节点列表（我的最爱 / 全部节点）、帖子列表、阅读页、说明页；本地保存我的最爱、已读状态和节点目录；`-i` 让子命令改为打开界面并停在该位置。

**Architecture:** 界面是一个不碰终端和网络的状态机：`App::update(Msg, now) -> Vec<Cmd>` 处理按键和请求结果，`view(&App, &mut Frame)` 只读状态画出一帧。`tui::runtime` 是唯一碰外部的地方：读终端事件、执行 `Cmd`（调用库、写状态文件、开浏览器）、把结果以 `Msg` 送回。每个请求带一个票号，画面只收自己当前票号的结果，离开画面后到达的结果自然被丢弃。`store` 管三类本地文件，`tui` 与 `cli` 都只通过 `store` 读写它们。

**Tech Stack:** ratatui 0.30（crossterm 0.29 后端）、tokio（`rt`、`sync`）。其余沿用计划一的依赖。测试用 ratatui 的 `TestBackend`、tempfile、wiremock。

**Spec:** `docs/superpowers/specs/2026-11-19-v2exe-design.md`

## Global Constraints

- 依赖方向：`v2exe` → `v2ex-rs`；程序内 `cli` 与 `tui` 互不依赖，都依赖 `render`、`store` 和库。`v2ex-rs` 不含界面、终端、文件读写代码。
- `App::update` 与各画面的状态代码不做 I/O：不读写文件、不发请求、不读时钟（时间由 `now` 参数传入）。I/O 只在 `tui::runtime` 与 `store`。
- 颜色只用终端 16 色（`ratatui::style::Color` 的具名色，不用 `Indexed`、`Rgb`）。关闭颜色后只用反白、粗体、暗色。配色含义照 spec §8.3。
- 段落边缘默认 `plain`；`powerline` 才使用 U+E0B0–U+E0B3。默认配置下输出不含这些字符。
- 版面照 spec §8.2 的两幅示意：圆角边框、上边框左右两组段落、下边框左按键提示右位置；帖子每条两行；回复每则一块，内容缩进 4 格。颜色、间距好不好看不写断言。
- 按键照 spec §8.5 全表；`h j k l` 是方向，说明用 `?`，`Ctrl-C` 在任何画面退出。
- 最小终端尺寸 40 列 × 8 行，小于它只显示一行提示。阅读页正文最大行宽 100 列。
- 写死的 tab（名字 → 显示名）：`tech` 技术、`creative` 创意、`play` 好玩、`apple` Apple、`jobs` 酷工作、`deals` 交易、`city` 城市、`qna` 问与答、`all` 全部。`hot` 与 `latest` 指 `Listing::Hot`（最热）与 `Listing::Latest`（最新）。
- 首次启动预置的我的最爱，按此顺序：最热、最新、技术、创意、好玩、问与答、全部。
- 本地文件（spec §9）：`<state>/favorites.json`、`<state>/read.json`、`<cache>/nodes.json`，其中 `<state>` 是 `$XDG_STATE_HOME/v2exe`（默认 `~/.local/state/v2exe`），`<cache>` 是 `$XDG_CACHE_HOME/v2exe`（默认 `~/.cache/v2exe`）。XDG 变量只在是绝对路径时采用。状态文件先写同目录的临时文件再改名。已读状态保留最近 10000 条；节点目录 24 小时内不重新下载。
- 打开帖子总是调用 `topic(id)` 再调用 `replies(id, 1)`，不复用列表里 feed 带的正文。
- 面向用户的文字用中文。测试夹具用示例内容。
- 贴近移植参考项目的代码时，在同一个 commit 里创建或更新 `THIRD-PARTY-NOTICES`，写入对方的版权与 MIT 许可声明。
- 每个任务结束时 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` 全部通过。Commit message 用英文 Conventional Commits。

## Review Focus

1. **终端在使用中被缩小或放大**，包括缩到最小尺寸以下再放大：不 panic；游标与滚动位置回到合法范围；每一行的最右一格仍是边框。测试在 Task 4、Task 5、Task 7。
2. **请求的结果回来时人已经离开那个画面**，或同一画面先后发了两次请求、旧的后到：旧结果被丢弃，画面内容不被覆盖。请求失败时保留已有内容。测试在 Task 5、Task 6、Task 7。
3. **空的东西**：列表为空、过滤后一条不剩、编号输入超出范围、零回复的帖子、没有正文的帖子：按键不 panic，游标操作是空操作，位置显示 `0/0`。测试在 Task 5、Task 6、Task 7。
4. **状态文件不存在、内容损坏、目录不可写、`HOME` 未设**：程序照常启动并使用默认值；写失败时界面继续可用，右上段落显示原因。测试在 Task 3、Task 8。
5. **标题或名字里有全角字符、emoji、控制字符**，且正好落在行尾：行不超宽，右边框完整，控制字符不进终端。测试在 Task 4、Task 6。

---

### Task 1: 库的补强

**Files:**
- Modify: `crates/v2ex-rs/src/http.rs`、`crates/v2ex-rs/src/client.rs`
- Modify: `docs/superpowers/specs/2026-11-19-v2exe-design.md` §6.5（受限编号集合的那一句，改为下面「受限编号的新鲜度」的规则）
- Test: `crates/v2ex-rs/src/http.rs` 的单元测试、`crates/v2ex-rs/tests/client.rs`、`crates/v2exe/tests/cli.rs`

**Interfaces:**
- Produces：`Transport::get(&self, path, kind, ttl) -> Result<Fetched>`，`pub(crate) struct Fetched { pub body: Bytes, pub fresh: bool }`，`fresh` 表示这次真的发了请求。`Client` 的公开接口不变。

三处行为：

- **受限编号的新鲜度。** `Listing::Tab` 的 feed 是这次新取的（`fresh`）时，先丢掉三个受限来源的缓存再取编号集合；feed 来自缓存时沿用已缓存的编号集合。理由：界面是长驻进程，tab feed 120 秒过期而编号集合 600 秒过期，中间新发的受限帖子会漏出来。代价是每次新取 tab feed 多一次 API 请求（`flamewar`）与两次 feed 请求。
- **不存在的 tab。** 站点对不存在的 tab 名返回 200 与空正文。feed 端点的空正文映射为 `Error::NotFound`，并且不留在缓存里。
- **缓存清扫。** 存入一条缓存时，顺手删掉所有已过期的条目。

- [ ] **Step 1: 写失败的测试**

```rust
// crates/v2ex-rs/tests/client.rs，替换 restricted_ids_are_fetched_once_across_tab_listings
#[tokio::test]
async fn restricted_ids_are_refetched_with_each_fresh_tab_feed() {
    // 挂 /feed/tab/all.xml 与 /feed/tab/tech.xml；mount_restricted_sources(&server, 2)
    // 依次取 all、tech、all：前两次各新取一次 feed，第三次来自缓存。
    // 三个受限来源各被请求恰好 2 次（由 expect(2) 在 server drop 时校验）。
}

#[tokio::test]
async fn unknown_tab_is_not_found_and_not_cached() {
    // /feed/tab/zzzz.xml 返回 200、空正文，expect(2)
    // 连取两次 Listing::Tab("zzzz")，两次都是 Err(Error::NotFound)
}
```

```rust
// crates/v2ex-rs/src/http.rs 的 tests
#[tokio::test]
async fn storing_an_entry_drops_expired_ones() {
    // 以 Duration::ZERO 取 /a，再以 TTL 取 /b
    // assert_eq!(lock(&t.inner.cache).len(), 1);
}

#[tokio::test]
async fn fresh_is_true_only_when_a_request_was_sent() {
    // 同一路径取两次：第一次 fresh == true，第二次 fresh == false
}
```

```rust
// crates/v2exe/tests/cli.rs
#[tokio::test]
async fn unknown_tab_exits_3() {
    // /feed/tab/zzzz.xml 返回 200、空正文
    // run_against(&server, &["tab", "zzzz"]).code == 3，stderr 含「找不到」
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test --workspace --locked --no-fail-fast`
Expected: 上面五个测试失败，其余通过。

- [ ] **Step 3: 实现**

`Transport::get` 返回 `Fetched`；存入缓存前 `retain` 掉过期条目。`Client::fetch` 取 `fetched.body` 解析，并把 `fresh` 交还给需要它的调用方（只有 `Listing::Tab` 分支用）。`restricted_ids(refresh: bool)`：`refresh` 为真时对三个来源的路径调用 `Transport::forget`。feed 的空正文在 `feed_topics` 里映射为 `NotFound`，走 `fetch` 已有的「解析失败即丢缓存」路径。

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test --workspace --locked`
Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add crates docs
git commit -m "fix(v2ex-rs): refresh restricted ids with each fresh tab feed"
```

---

### Task 2: `store` 的底座：目录、原子写入、配置

**Files:**
- Create: `crates/v2exe/src/store/mod.rs`、`crates/v2exe/src/store/paths.rs`
- Move: `crates/v2exe/src/config.rs` → `crates/v2exe/src/store/config.rs`（`v2exe::config` 这个路径随之消失，引用处改为 `v2exe::store::config`）
- Modify: `crates/v2exe/src/lib.rs`、`crates/v2exe/src/cli/run.rs`
- Modify: `README.md` 的配置一节（加 `segment_edge`）
- Test: `crates/v2exe/src/store/paths.rs` 与 `config.rs` 的单元测试、`crates/v2exe/tests/cli.rs`

**Interfaces:**
- Produces：

```rust
// store/paths.rs
/// 每一项都已经带上 `v2exe` 这一层；取不到（变量不是绝对路径且没有 HOME）时为 None。
pub struct Dirs { pub config: Option<PathBuf>, pub state: Option<PathBuf>, pub cache: Option<PathBuf> }
impl Dirs {
    pub fn from_env(var: impl Fn(&str) -> Option<OsString>) -> Dirs;
    pub fn from_process() -> Dirs;
}
/// 建好父目录，写同目录下的临时文件，再改名为 `path`。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()>;

// store/config.rs
#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentEdge { #[default] Plain, Powerline }
pub struct Config { pub base_url: Option<String>, pub proxy: Option<String>, pub timeout: Option<u64>, pub segment_edge: Option<SegmentEdge> }
pub fn load(path: &Path) -> Result<Config, ConfigError>;   // 不变
```

- `config_path_from` 与 `config_path` 删除；`RunEnv::from_process` 用 `Dirs::from_process().config` 拼出 `config.toml`。
- **配置文件里的坏值。** 配置文件给了 `base_url` 或 `proxy` 时，先只用文件里的值建一次 `Client`；失败就以退出码 1 结束，信息是 `配置文件 <路径> 有误：<原因>`，不管命令行有没有覆盖它（与 `timeout = 0` 的处理一致）。之后再用命令行覆盖后的值建正式的 `Client`，这一步失败仍是退出码 2。

- [ ] **Step 1: 写失败的测试**

```rust
// store/paths.rs
#[test]
fn dirs_follow_xdg_then_home() {
    let env = |pairs: &'static [(&str, &str)]| move |k: &str| {
        pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| OsString::from(v))
    };
    let d = Dirs::from_env(env(&[("HOME", "/h")]));
    assert_eq!(d.config, Some("/h/.config/v2exe".into()));
    assert_eq!(d.state, Some("/h/.local/state/v2exe".into()));
    assert_eq!(d.cache, Some("/h/.cache/v2exe".into()));

    let d = Dirs::from_env(env(&[
        ("HOME", "/h"), ("XDG_CONFIG_HOME", "/c"), ("XDG_STATE_HOME", "relative"), ("XDG_CACHE_HOME", ""),
    ]));
    assert_eq!(d.config, Some("/c/v2exe".into()));
    assert_eq!(d.state, Some("/h/.local/state/v2exe".into()));
    assert_eq!(d.cache, Some("/h/.cache/v2exe".into()));

    let d = Dirs::from_env(env(&[]));
    assert!(d.config.is_none() && d.state.is_none() && d.cache.is_none());
}

#[test]
fn write_atomic_creates_parents_replaces_and_leaves_no_temp_file() {
    // 在 tempdir 下写 a/b/file 两次（不同内容）
    // 读回是第二次的内容；a/b 目录里只有 file 这一个条目
}

#[test]
fn write_atomic_reports_an_unwritable_directory() {
    // 父路径是一个普通文件 → Err，且不 panic
}
```

```rust
// store/config.rs
#[test]
fn segment_edge_parses_and_rejects_unknown_values() {
    // "segment_edge = \"powerline\"" → Some(SegmentEdge::Powerline)
    // 缺省 → None；"segment_edge = \"round\"" → Err，信息含文件路径
}
```

```rust
// tests/cli.rs
#[tokio::test]
async fn bad_base_url_or_proxy_in_config_exits_1_naming_the_file() {
    // config.toml 写 base_url = "not a url"，命令行另给 --base-url <server>：退出码 1，stderr 含配置文件路径
    // config.toml 写 proxy = "not a url"：同样退出码 1
}
```

- [ ] **Step 2: 运行，确认失败**（`cargo test -p v2exe --locked --no-fail-fast`）
- [ ] **Step 3: 实现**。临时文件名用目标文件名加 `.tmp.<进程号>`。`tests/cli.rs` 的 `CLEARED_ENV` 补上 `NO_PROXY` 与 `no_proxy`，让代理测试不受开发者 shell 的影响。
- [ ] **Step 4: 运行，确认通过**（全 workspace）
- [ ] **Step 5: Commit**

```bash
git add crates README.md
git commit -m "refactor(v2exe): add store with XDG dirs, atomic writes and segment_edge"
```

---

### Task 3: `store` 的三份数据：我的最爱、已读状态、节点目录

**Files:**
- Create: `crates/v2exe/src/store/favorites.rs`、`crates/v2exe/src/store/read_state.rs`、`crates/v2exe/src/store/node_cache.rs`
- Modify: `crates/v2exe/src/store/mod.rs`
- Test: 三个文件各自的单元测试

**Interfaces:**
- Consumes：`write_atomic`（Task 2）；`v2ex_rs::{Listing, Node}`。
- Produces：

```rust
// store/favorites.rs
pub const TABS: [(&str, &str); 9];                 // Global Constraints 的 tab 名单，按该顺序
pub fn listing_title(listing: &Listing) -> Option<&'static str>;   // Hot → 最热，Latest → 最新，已知 tab → 显示名，其余 None

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Favorite { pub listing: Listing, pub title: String }

pub struct Favorites { /* path: Option<PathBuf>, items: Vec<Favorite> */ }
impl Favorites {
    /// 文件不存在或读不出来时是预置的那七项。`path` 为 None 时只在内存里。
    pub fn load(path: Option<PathBuf>) -> Favorites;
    pub fn items(&self) -> &[Favorite];
    pub fn contains(&self, listing: &Listing) -> bool;
    /// 已经在里面时不动，返回 false。
    pub fn add(&mut self, favorite: Favorite) -> bool;
    pub fn remove(&mut self, index: usize);           // 越界时不动
    pub fn swap(&mut self, a: usize, b: usize);        // 任一越界时不动
    pub fn save(&self) -> io::Result<()>;              // path 为 None 时 Ok(())
}

// store/read_state.rs
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark { Unread, Updated(u32), Read }       // Updated 带新增的回复数
pub const KEEP: usize = 10_000;
pub struct ReadState { /* path, HashMap<u64, Seen { replies: u32, at: DateTime<Utc> }> */ }
impl ReadState {
    pub fn load(path: Option<PathBuf>) -> ReadState;   // 不存在或读不出来 → 空
    pub fn mark(&self, topic_id: u64, replies: u32) -> Mark;
    pub fn record(&mut self, topic_id: u64, replies: u32, now: DateTime<Utc>);
    pub fn forget(&mut self, topic_id: u64);
    pub fn save(&self) -> io::Result<()>;              // 只留 at 最新的 KEEP 条
}

// store/node_cache.rs
pub const MAX_AGE: chrono::Duration;                // 24 小时
pub fn load_nodes(path: &Path, now: DateTime<Utc>) -> Option<Vec<Node>>;   // 不存在、读不出来、过期 → None
pub fn save_nodes(path: &Path, nodes: &[Node], now: DateTime<Utc>) -> io::Result<()>;
```

文件格式都是 JSON：`favorites.json` 是 `Favorite` 数组；`read.json` 是 `{"<帖子编号>": {"replies": n, "at": "<RFC 3339>"}}`；`nodes.json` 是 `{"fetched": "<RFC 3339>", "nodes": [Node]}`。

- [ ] **Step 1: 写失败的测试**

```rust
// favorites.rs
#[test]
fn first_run_has_the_seven_presets_in_order() {
    let f = Favorites::load(None);
    let titles: Vec<_> = f.items().iter().map(|x| x.title.as_str()).collect();
    assert_eq!(titles, ["最热", "最新", "技术", "创意", "好玩", "问与答", "全部"]);
    assert_eq!(f.items()[0].listing, Listing::Hot);
    assert_eq!(f.items()[2].listing, Listing::Tab("tech".into()));
}

#[test]
fn add_remove_swap_and_round_trip() {
    // tempdir 下 load → add(Node("python"), "Python") 返回 true，再 add 同一项返回 false
    // swap(0, 7)、remove(1)、save；重新 load 后 items 与内存里的相同
    // remove(99)、swap(0, 99) 不改变内容
}

#[test]
fn a_corrupt_or_missing_file_gives_the_presets_and_save_without_a_path_is_ok() {
    // 文件内容 "{" → 七项预置；Favorites::load(None).save() 是 Ok
}

// read_state.rs
#[test]
fn marks_follow_the_recorded_reply_count() {
    let mut r = ReadState::load(None);
    assert_eq!(r.mark(1, 5), Mark::Unread);
    r.record(1, 5, now);
    assert_eq!(r.mark(1, 5), Mark::Read);
    assert_eq!(r.mark(1, 8), Mark::Updated(3));
    assert_eq!(r.mark(1, 4), Mark::Read);          // 回复数变少不算有新回复
    r.forget(1);
    assert_eq!(r.mark(1, 5), Mark::Unread);
}

#[test]
fn save_keeps_the_newest_entries_only() {
    // 记录 KEEP + 5 条，at 依次递增；save 后重新 load
    // 最早的 5 条是 Unread，其余是 Read
}

#[test]
fn a_corrupt_file_reads_as_empty() { /* 内容 "[1,2" → mark(1, 0) == Unread */ }

// node_cache.rs
#[test]
fn nodes_round_trip_until_they_are_a_day_old() {
    // save_nodes(path, &nodes, t0)；load_nodes(path, t0 + 23h) == Some(nodes)
    // load_nodes(path, t0 + 25h) == None；不存在的路径 → None；内容 "x" → None
}
```

- [ ] **Step 2: 运行，确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行，确认通过**
- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "feat(v2exe): store favorites, read state and the node directory"
```

---

### Task 4: 配色、外框与段落

**Files:**
- Modify: `crates/v2exe/Cargo.toml`（加 `ratatui`；`tokio` 加 `sync`）、`Cargo.lock`
- Modify: `crates/v2exe/src/lib.rs`（`pub mod tui;`）、`crates/v2exe/src/render.rs`（`display_width`）
- Create: `crates/v2exe/src/tui/mod.rs`、`crates/v2exe/src/tui/theme.rs`、`crates/v2exe/src/tui/frame.rs`
- Test: `crates/v2exe/tests/render.rs`、`crates/v2exe/src/tui/frame.rs` 的单元测试

**Interfaces:**
- Consumes：`SegmentEdge`（Task 2）。
- Produces：

```rust
// tui/theme.rs —— 语义到样式；color 为 false 时只返回 REVERSED / BOLD / DIM 的组合
#[derive(Debug, Clone, Copy)]
pub struct Theme { pub color: bool }
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    Plain, Bold, Minor,            // 正文、粗体、次要信息（暗色）
    Location, Fresh, Changed, Alert, Person, Op,   // 蓝、绿、黄、红、青、洋红
    Link, Mention, Code, CodeBar, Quote, Image,
    Cursor, Selected,              // 游标 ❯ 与所在行
    RepliesFew, RepliesMany, RepliesHuge,          // 1–9、10–99、100 以上
}
impl Theme { pub fn style(&self, paint: Paint) -> ratatui::style::Style; }

// tui/frame.rs
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone { Site, Location, Status, Ok, Warn, Alert, Identity }
pub struct Segment { pub text: String, pub tone: Tone }
pub struct Chrome {
    pub left: Vec<Segment>, pub right: Vec<Segment>,
    pub hints: Vec<(String, String)>,     // (按键, 说明)
    pub position: String,                 // 如 "3/50"、"62%"
}
pub const MIN_SIZE: (u16, u16) = (40, 8);
/// 画外框，返回框内可用区域。放不下时先从右组的最后一个段落删起，再从按键提示的最后一项删起；左组与位置保留。
pub fn draw_frame(frame: &mut Frame, area: Rect, chrome: &Chrome, theme: &Theme, edge: SegmentEdge) -> Rect;
/// 终端小于 MIN_SIZE 时画的一行提示：「终端太小，至少需要 40×8」。
pub fn draw_too_small(frame: &mut Frame, area: Rect);
pub fn fits(area: Rect) -> bool;
```

- `render::display_width`：U+FE0F 跟在显示宽度为 1 的字符后面时，这一对算 2 格（`"❤\u{fe0f}"` 是 2）。`wrap` 用同一规则，变体选择符不与它前面的字符分到两行。
- 段落画法：一块底色、字用反白（`Modifier::REVERSED` 配前景色），`plain` 两侧各留一格空白；`powerline` 在段落之间用 U+E0B0（左组）与 U+E0B2（右组）。

- [ ] **Step 1: 写失败的测试**

```rust
// tests/render.rs
#[test]
fn variation_selector_makes_a_narrow_emoji_two_columns() {
    assert_eq!(display_width("❤\u{fe0f}"), 2);
    assert_eq!(display_width("a\u{fe0f}"), 2);
    // 宽度 2、内容 "❤\u{fe0f}❤\u{fe0f}" 折成两行，每行都是 "❤\u{fe0f}"
}
```

```rust
// tui/frame.rs —— 用 ratatui::backend::TestBackend 画一帧再读 buffer
fn chrome() -> Chrome { /* 左：V2EX、python、+4 ~2；右：✔ API 571/600、guest；四项提示；位置 3/50 */ }

#[test]
fn corners_and_right_border_are_intact_at_every_width() {
    for (w, h) in [(40, 8), (80, 24), (200, 50)] {
        // 四角是 ╭ ╮ ╰ ╯；第 1..h-1 行的第一格与最后一格都是 │
        // 返回的内区是 Rect { x: 1, y: 1, width: w - 2, height: h - 2 }
    }
}

#[test]
fn a_narrow_frame_keeps_the_left_segments_and_the_position() {
    // 宽 40：上边框含 "V2EX" 与 "python"，下边框含 "3/50"；不含 "guest"
    // 宽 120：上边框同时含 "✔ API 571/600" 与 "guest"，下边框含四项提示
}

#[test]
fn plain_edges_use_no_powerline_glyphs_and_powerline_does() {
    // Plain：整个 buffer 没有 U+E0B0..=U+E0B3 的字符；Powerline：至少有一个 U+E0B0
}

#[test]
fn wide_text_in_a_segment_never_overwrites_the_corner() {
    // 位置段落文字是 30 个「节」，宽 40：右上角仍是 ╮
}

#[test]
fn too_small_draws_the_notice_without_panicking() {
    // 宽高 (10, 2)、(1, 1)、(39, 8)、(40, 7)：fits 为 false，draw_too_small 不 panic
    // (40, 8)：fits 为 true
}
```

- [ ] **Step 2: 运行，确认失败**
- [ ] **Step 3: 实现**。外框用 `Block::bordered().border_type(BorderType::Rounded)` 加 `title_top` / `title_bottom`；段落的文字先经 `render::sanitize`。
- [ ] **Step 4: 运行，确认通过**
- [ ] **Step 5: Commit**

```bash
git add Cargo.lock crates
git commit -m "feat(v2exe): add the TUI theme and the panel frame with segments"
```

---

### Task 5: 状态机骨架与节点列表

**Files:**
- Create: `crates/v2exe/src/tui/app.rs`、`crates/v2exe/src/tui/list.rs`、`crates/v2exe/src/tui/prompt.rs`、`crates/v2exe/src/tui/nodes.rs`
- Modify: `crates/v2exe/src/tui/mod.rs`
- Test: 各文件的单元测试

**Interfaces:**
- Consumes：`Favorites`、`Favorite`、`ReadState`、`TABS`、`listing_title`（Task 3）；`Chrome`、`Segment`、`Tone`、`draw_frame`、`fits`、`draw_too_small`、`Theme`、`Paint`（Task 4）。
- Produces：

```rust
// tui/mod.rs
#[derive(Debug, Clone, PartialEq)]
pub enum Start { Home, Listing(Listing), Topic(u64) }

// tui/app.rs
pub enum Msg {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
    TopicsLoaded { ticket: u64, result: v2ex_rs::Result<Vec<Topic>>, quota: Option<RateLimit> },
    ThreadLoaded { ticket: u64, result: v2ex_rs::Result<(Topic, Vec<Reply>)>, quota: Option<RateLimit> },
    DirectoryLoaded { result: v2ex_rs::Result<Vec<Node>>, quota: Option<RateLimit> },
    SaveFailed(String),
}
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    LoadTopics { ticket: u64, listing: Listing, refresh: bool },
    LoadThread { ticket: u64, id: u64, refresh: bool },
    LoadDirectory { refresh: bool },
    OpenUrl(String),
    SaveFavorites,
    SaveRead,
}
pub enum Screen { Nodes(NodesScreen), Topics(TopicsScreen), Reader(ReaderScreen), Help }
pub struct App {
    pub stack: Vec<Screen>,              // 栈底总是 Nodes
    pub favorites: Favorites,
    pub read: ReadState,
    pub directory: Option<Vec<Node>>,
    pub quota: Option<RateLimit>,
    pub error: Option<String>,           // 上一次请求或写文件失败的原因；下一次请求成功时清掉
    pub size: (u16, u16),
    pub tz: FixedOffset,
    pub quit: bool,
    /* next_ticket: u64 */
}
impl App {
    pub fn new(start: Start, favorites: Favorites, read: ReadState, size: (u16, u16), tz: FixedOffset) -> (App, Vec<Cmd>);
    pub fn update(&mut self, msg: Msg, now: DateTime<Utc>) -> Vec<Cmd>;
    pub fn ticket(&mut self) -> u64;     // 每次调用加一
    /// 右上两个段落：接口余量（两成以下 Warn，耗尽 Alert 并写恢复时间，error 非空时 Alert 并写原因）与 guest。
    pub fn right_segments(&self, now: DateTime<Utc>) -> Vec<Segment>;
}
pub fn view(app: &App, theme: &Theme, edge: SegmentEdge, now: DateTime<Utc>, frame: &mut Frame);

// tui/list.rs —— 节点列表与帖子列表共用的游标
pub struct ListNav { pub cursor: usize, pub offset: usize, pub typed: String }
pub enum Nav { Handled, Jump(usize), Open, Back, Pass }   // Jump 带人输入的编号（从 1 起）
impl ListNav {
    /// 处理 spec §8.5「列表通用」里的移动、翻页、首尾、数字加 Enter、进入、返回。其余按键返回 Pass。
    pub fn key(&mut self, key: KeyEvent, len: usize, page: usize) -> Nav;
    /// 让 cursor < len（len 为 0 时 cursor 为 0），并让 cursor 落在 [offset, offset + page) 里。
    pub fn clamp(&mut self, len: usize, page: usize);
}

// tui/prompt.rs —— 下边框里的一行输入
pub struct Prompt { pub label: &'static str, pub text: String }
pub enum PromptKey { Edited, Submit, Cancel, Complete }    // Complete 是 Tab 键
impl Prompt { pub fn key(&mut self, key: KeyEvent) -> PromptKey; }   // 可打印字符追加，Backspace 删一个字符，Enter 提交，Esc 取消

// tui/nodes.rs
pub enum NodesView { Favorites, All }
pub struct NodesScreen { pub view: NodesView, pub nav: ListNav, pub filter: Option<String>, pub prompt: Option<Prompt>, pub moving: bool, pub loading: bool }
pub struct Row { pub number: usize, pub listing: Listing, pub name: String, pub title: String, pub topics: Option<u32> }
impl NodesScreen {
    /// 当前视图过滤后的行。全部节点：先是最热、最新和九个 tab，再是节点目录（按帖子数从多到少）。
    pub fn rows(&self, favorites: &Favorites, directory: Option<&[Node]>) -> Vec<Row>;
}
```

行为（节点列表）：

- `→` `Enter` `l`：压入该行的帖子列表，发 `LoadTopics`。
- `y` 切换视图；切到全部节点而 `directory` 为空时发 `LoadDirectory { refresh: false }`，`loading` 为真。
- `a`（全部节点视图）把该行加入我的最爱；`d`（我的最爱视图）移出该行；`M`（我的最爱视图）进入移动状态，这时 `↑` `↓` 把该行与相邻行对调，`M`、`Enter`、`Esc` 结束。三者改动后都发 `SaveFavorites`。
- `/` 打开过滤输入，边打边过滤（节点名与标题，不分大小写）；`Enter` 保留过滤，`Esc` 取消。有过滤时 `←` `q` `h` 先清掉过滤，再按才返回。
- 编号是未过滤时的位置，从 1 起；`Jump(n)` 跳到编号为 n 的行，那一行不在当前可见行里时不动。
- `r`：全部节点视图发 `LoadDirectory { refresh: true }`；我的最爱视图无动作。
- `←` `q` `h` 在栈底时把 `quit` 置为真。`Ctrl-C` 在任何画面、任何输入状态下都把 `quit` 置为真。`?` 压入 `Screen::Help`。
- 一页的行数是框内高度（每行一个节点）。`Resize` 后对每个列表画面调用 `clamp`。
- `s` 的直达输入在 Task 6 实现；本任务里 `s` 无动作。

- [ ] **Step 1: 写失败的测试**

```rust
// tui/list.rs
#[test]
fn movement_paging_and_ends() {
    // len 10、page 4：j ↓ 各下移一行；k ↑ 上移；Home 到 0；End 与 $ 到 9
    // PgDn、空格、Ctrl-F 下移 4 行并且不越过 9；PgUp、Ctrl-B 上移 4 行并且不小于 0
    // 每一步之后 offset <= cursor < offset + 4
}
#[test]
fn digits_then_enter_jump_and_other_keys_clear_them() {
    // 键入 '1' '2' Enter → Nav::Jump(12)，typed 清空
    // 键入 '3' 再按 j → typed 清空，返回 Handled
    // 没有数字时 Enter → Nav::Open；→ 与 l → Open；← q h → Back
}
#[test]
fn an_empty_list_ignores_movement() {
    // len 0：所有移动键之后 cursor == 0、offset == 0，不 panic；clamp(0, 0) 不 panic
}
#[test]
fn clamp_pulls_the_cursor_back_after_the_list_or_the_page_shrinks() {
    // cursor 9、offset 6，clamp(5, 2) 之后 cursor == 4、offset == 3
}

// tui/nodes.rs 与 tui/app.rs —— 用 Favorites::load(None)、ReadState::load(None)、size (80, 24) 建 App
#[test]
fn starts_on_favorites_and_enter_opens_the_listing() {
    // App::new(Start::Home, ..) 的栈是 [Nodes]，没有 Cmd
    // 按 Enter：栈是 [Nodes, Topics]，返回 [LoadTopics { listing: Listing::Hot, refresh: false, .. }]
}
#[test]
fn start_listing_and_start_topic_open_on_top_of_home() {
    // Start::Listing(Node("python")) → 栈 [Nodes, Topics]，Cmd 是 LoadTopics
    // Start::Topic(123) → 栈 [Nodes, Reader]，Cmd 是 LoadThread { id: 123, .. }
}
#[test]
fn y_switches_to_all_nodes_and_loads_the_directory_once() {
    // 第一次 y → [LoadDirectory { refresh: false }]；DirectoryLoaded(Ok(两个节点)) 之后 rows 是 11 个固定项加 2 个节点，节点按帖子数从多到少
    // 再按两次 y 回到全部节点：没有新的 Cmd
}
#[test]
fn add_remove_and_move_change_favorites_and_ask_for_a_save() {
    // 全部节点视图里游标在某个节点上按 a → favorites 多一项，Cmd 含 SaveFavorites；再按 a 不重复添加
    // 我的最爱视图：d 移出游标所在行；M 后按 ↓ 把该行下移一位，再按 M 结束；每次都含 SaveFavorites
}
#[test]
fn filter_narrows_rows_keeps_numbers_and_back_clears_it_first() {
    // 我的最爱视图按 / 键入「技」Enter：rows 只剩「技术」，它的 number 仍是 3
    // 键入 '1' Enter（编号 1 不在可见行里）游标不动
    // 按 ← ：过滤清掉、栈不变、quit 为 false；再按 ← ：quit 为 true
}
#[test]
fn a_filter_with_no_match_leaves_an_empty_list_that_ignores_keys() {
    // 过滤「zzz」：rows 为空；j、Enter、a、d、M 都不 panic，栈不变，没有 Cmd
}
#[test]
fn ctrl_c_quits_from_any_state_and_question_mark_opens_help() {
    // 过滤输入中按 Ctrl-C → quit；初始状态按 ? → 栈顶是 Help，再按 q 弹出
}
#[test]
fn resize_keeps_the_cursor_visible_and_survives_tiny_sizes() {
    // 全部节点视图（目录 100 个节点）游标在末行；Resize(80, 10) 之后 offset <= cursor < offset + 8
    // Resize(10, 2) 再 Resize(80, 24)：不 panic，游标仍在末行
}
#[test]
fn right_segments_follow_the_quota_and_the_last_error() {
    // quota None → 第一个段落文字含 "API"，Tone::Ok；600/571 → "✔ API 571/600"，Ok
    // 600/100 → Warn；remaining 0 → Alert 且文字含恢复时间（按 tz 的 HH:MM）
    // error = Some("网络错误：…") → Alert 且文字含该原因；最后一个段落是 "guest"，Tone::Identity
}
#[test]
fn a_directory_error_keeps_the_view_and_sets_the_error() {
    // DirectoryLoaded(Err(Error::Network(..))) → loading 为 false，error 非空，rows 仍是 11 个固定项
}

// tui/app.rs 的 view —— TestBackend
#[test]
fn the_node_list_fills_the_height_and_keeps_its_border() {
    // 80×24 与 80×12、全部节点视图 100 个节点：框内行数分别是 22 与 10，每行最右一格是 │
    // 下边框含位置 "1/111"
}
#[test]
fn below_the_minimum_size_only_the_notice_is_drawn() {
    // size (30, 5)：buffer 里有「终端太小」，没有 ╭
}
```

- [ ] **Step 2: 运行，确认失败**
- [ ] **Step 3: 实现**。节点行的画法：编号（暗色，右对齐）、名字（蓝）、显示名、帖子数（暗色，靠右）；游标行最左是 `❯`。左组段落：`V2EX`、`我的最爱` 或 `全部节点`、`loading` 时的加载标记 `…`。按键提示：`↑↓ 移动`、`→ 进入`、`y 全部节点`（或 `y 我的最爱`）、`/ 过滤`、`? 说明`。`Screen::Topics`、`Screen::Reader` 在本任务里先放最小的占位结构，Task 6、Task 7 填实。
- [ ] **Step 4: 运行，确认通过**
- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "feat(v2exe): add the TUI state machine and the node list"
```

---

### Task 6: 帖子列表与节点直达

**Files:**
- Create: `crates/v2exe/src/tui/topics.rs`
- Modify: `crates/v2exe/src/tui/app.rs`、`crates/v2exe/src/tui/nodes.rs`、`crates/v2exe/src/tui/mod.rs`
- Test: `topics.rs` 与 `app.rs` 的单元测试

**Interfaces:**
- Consumes：Task 5 的全部；`ReadState::{mark, record, forget}`、`Mark`（Task 3）。
- Produces：

```rust
// tui/topics.rs
pub struct TopicsScreen {
    pub listing: Listing, pub title: String,
    pub topics: Vec<Topic>, pub pending: Option<u64>,       // pending 是在等的票号
    pub nav: ListNav, pub filter: Option<String>, pub author: Option<String>, pub prompt: Option<Prompt>,
}
impl TopicsScreen {
    pub fn new(listing: Listing, title: String, ticket: u64) -> TopicsScreen;
    /// 过滤后的 (编号, 帖子)。编号是未过滤时的位置，从 1 起。
    pub fn rows(&self) -> Vec<(usize, &Topic)>;
    /// (未读数, 读过之后有新回复的数)，按未过滤的全部帖子算。
    pub fn counts(&self, read: &ReadState) -> (usize, usize);
}
/// 节点直达输入的候选：`hot`、`latest`、九个 tab 名，再加目录里的节点名；按前缀匹配，不分大小写，最多 8 个。
pub fn completions(input: &str, directory: Option<&[Node]>) -> Vec<String>;
/// 把直达输入变成 Listing 与显示名：`hot`、`latest`、tab 名各归其位，其余当节点名。空串是 None。
pub fn resolve(input: &str, directory: Option<&[Node]>) -> Option<(Listing, String)>;
```

行为：

- `TopicsLoaded`：票号等于 `pending` 才收下，否则丢弃。成功则替换 `topics`、清 `pending`、清 `App::error`，并 `clamp`；失败则保留原有 `topics`、清 `pending`、把原因写进 `App::error`（文字用 `cli::exit::message` 同一套措辞；把那个函数移到 `tui` 与 `cli` 都能引用而不互相依赖的位置，如 `crate::errors`）。`quota` 为 Some 时更新 `App::quota`。
- `→` `Enter` `l`：压入阅读页并发 `LoadThread`，`new_replies` 由 `read.mark(id, 列表里的回复数)` 算出。打开时不记已读，帖子载入成功时才记（Task 7 的 `ThreadLoaded`）。
- `v`：该行是 `Unread` 时记为已读，否则 `forget`；发 `SaveRead`。
- `a`：按该行的作者过滤；已在按作者过滤时再按一次取消。`/` 按标题过滤，规则同节点列表。有任一过滤时 `←` `q` `h` 先清过滤。
- `o`：发 `OpenUrl(该行的 url)`。`r`：新票号，发 `LoadTopics { refresh: true }`。
- `s`（节点列表与帖子列表都有）：打开直达输入；`Tab` 把输入补成第一个候选；`Enter` 用 `resolve` 的结果：在帖子列表里替换栈顶的列表，在节点列表里压入新的列表；`Esc` 取消。候选需要节点目录，`directory` 为空时打开输入的同时发 `LoadDirectory { refresh: false }`。
- 一页的条数是框内高度除以 2（向下取整，至少 1）。
- 左组段落：`V2EX`、显示名、`+N ~M`（都为 0 时不出现）；等结果时第三段是 `…`。位置是 `游标所在编号/总数`，列表为空时是 `0/0`。

每条帖子两行（spec §8.2）：第一行 `❯`（仅游标行）、标记（`+` 绿、`~` 黄、空白）、标题、行尾的回复数（1–9 绿，10–99 黄粗体，100 以上写「爆」红粗体，0 不显示）；第二行缩进后是作者（青）、日期（暗色，`MM/DD`）、已知时的 `最后回复 <名字>`。标题太长时截断并以 `…` 结尾，截断按显示宽度算。标题、名字先经 `render::sanitize`。

- [ ] **Step 1: 写失败的测试**

```rust
// tui/topics.rs 与 tui/app.rs
#[test]
fn loaded_topics_are_shown_and_a_stale_ticket_is_dropped() {
    // 进入最热（票号 t1）→ 按 r（票号 t2）
    // TopicsLoaded { ticket: t1, Ok(3 条) } 被丢弃：topics 为空、pending 仍是 Some(t2)
    // TopicsLoaded { ticket: t2, Ok(2 条) } 被收下：topics 2 条、pending 为 None
}
#[test]
fn a_result_for_a_screen_already_left_is_dropped() {
    // 进入最热后按 ← 返回，再送来 TopicsLoaded { ticket: t1, .. }：栈是 [Nodes]，不 panic，没有 Cmd
}
#[test]
fn a_failed_reload_keeps_the_topics_and_reports_the_reason() {
    // 已有 3 条时按 r，送来 Err(Error::RateLimited { reset: Some(..) })
    // topics 仍是 3 条；App::error 含「额度」；随后一次 Ok 清掉 error
}
#[test]
fn an_opened_topic_is_recorded_once_its_thread_loads_and_marks_follow() {
    // 3 条帖子（回复数 5、0、12）：counts == (3, 0)
    // 在第一条上按 Enter：栈顶是 Reader，Cmd 只有 LoadThread { id }，仍是未读；ThreadLoaded Ok 之后 Cmd 是 SaveRead；返回后 counts == (2, 0)
    // 重新加载后第一条回复数变 8：counts == (2, 1)
}
#[test]
fn v_toggles_read_and_a_filters_by_author() {
    // v：Unread → Read → Unread，每次都有 SaveRead
    // a：rows 只剩同一作者的帖子，编号不变；再按 a 恢复全部
}
#[test]
fn title_filter_and_back_behave_like_the_node_list() {
    // / 键入标题片段 Enter：rows 变少；← 清过滤而不出栈；再 ← 出栈
}
#[test]
fn an_empty_listing_ignores_every_key() {
    // TopicsLoaded(Ok(vec![])) 之后 Enter、v、a、o、空格、End、'5' Enter 都不 panic，没有 Cmd，位置是 "0/0"
}
#[test]
fn o_opens_the_url_and_r_reloads_bypassing_the_cache() {
    // o → [OpenUrl("https://www.v2ex.com/t/1001")]；r → [LoadTopics { refresh: true, .. }]
}
#[test]
fn goto_completes_resolves_and_replaces_the_listing() {
    // completions("py", Some(目录含 python、pytorch、go)) == ["python", "pytorch"]
    // completions("", None) 的前两项是 "hot"、"latest"；completions("TE", None) == ["tech"]
    // resolve("hot", None) == Some((Listing::Hot, "最热"))；resolve("tech", None) == Some((Tab("tech"), "技术"))
    // resolve("python", Some(目录)) == Some((Node("python"), "Python"))；目录里没有时显示名就是输入；resolve("", None) == None
    // 帖子列表里按 s、键入 "py"、Tab、Enter：栈仍是 [Nodes, Topics]，栈顶 listing 是 Node("python")，Cmd 含 LoadTopics
    // 节点列表里按 s、键入 "latest"、Enter：栈是 [Nodes, Topics]
    // directory 为空时按 s：Cmd 含 LoadDirectory { refresh: false }
}

// view —— TestBackend
#[test]
fn rows_take_two_lines_and_the_count_follows_the_height() {
    // 50 条帖子：80×24 画出 11 条（框内 22 行），80×13 画出 5 条；第一条的两行里分别有标题与作者
}
#[test]
fn wide_titles_emoji_and_control_characters_stay_inside_the_frame() {
    // 标题分别是 60 个全角字符、30 个 "👨‍👩‍👧"、"a\x1b[31mb"、空串；作者名 40 个字符
    // 宽 40、41、80：每行最右一格是 │；buffer 里没有 U+001B；长标题的那一行含 "…"
}
#[test]
fn reply_counts_show_the_three_bands() {
    // 回复数 0、5、42、230：第一行行尾分别是空、"5"、"42"、"爆"
}
```

- [ ] **Step 2: 运行，确认失败**
- [ ] **Step 3: 实现**。按键提示：`↑↓ 移动`、`→ 阅读`、`s 换节点`、`/ 找标题`、`? 说明`；直达输入打开时下边框左侧改为 `节点：<输入>` 加候选。
- [ ] **Step 4: 运行，确认通过**
- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "feat(v2exe): add the topic list with read marks and node goto"
```

---

### Task 7: 阅读页

**Files:**
- Create: `crates/v2exe/src/tui/reader.rs`
- Modify: `crates/v2exe/src/tui/app.rs`、`crates/v2exe/src/tui/mod.rs`
- Test: `reader.rs` 与 `app.rs` 的单元测试

**Interfaces:**
- Consumes：`render::{render_html, wrap, display_width, sanitize, Line, LineKind, Role}`；`Paint`（Task 4）；Task 5、Task 6 的 `App`、`TopicsScreen`。
- Produces：

```rust
// tui/reader.rs
pub struct RLine { pub spans: Vec<(String, Paint)> }
impl RLine { pub fn text(&self) -> String; }
pub struct Layout { pub lines: Vec<RLine>, pub floors: Vec<usize> }   // floors[n - 1] 是 n 楼那一块第一行的行号
/// 标题、作者与时间与回复数一行、空行、正文，然后每则回复一块。`width` 是可用列数，正文按 min(width, 100) 折行。
pub fn layout(topic: &Topic, replies: &[Reply], width: usize, tz: &FixedOffset) -> Layout;

pub struct ReaderScreen {
    pub id: u64, pub pending: Option<u64>,
    pub topic: Option<Topic>, pub replies: Vec<Reply>,
    pub layout: Layout, pub scroll: usize,
    pub new_replies: u32,                    // 打开时比上次记录多出的回复数
    pub typed: String, pub prompt: Option<Prompt>, pub search: Option<String>,
}
impl ReaderScreen { pub fn new(id: u64, ticket: u64, new_replies: u32) -> ReaderScreen; }
```

版面（spec §8.2 第二幅）：

- 标题一行或多行（粗体）；下一行 `作者 · YYYY-MM-DD HH:MM · N 回复`；空一行；正文。
- 正文行：代码行前面是 `  ▎`（竖条绿色）；引用行前面是 `  │ `；图片是一行 `[图片] 网址`。
- 每则回复一块：第一行左边 `#楼层号 名字`，作者是楼主时后面加 ` 楼主`（洋红），时间 `MM/DD HH:MM` 靠右（暗色）；内容每行缩进 4 格。块与块之间不空行，正文与第一块之间空一行。
- 没有正文的帖子：标题与元信息之后直接是回复；零回复时到正文为止。

行为：

- `ThreadLoaded`：票号等于 `pending` 才收下。成功则存下帖子与回复、按当前宽度 `layout`、`read.record(id, max(topic.replies, 回复的条数), now)` 并发 `SaveRead`；失败保留已有内容并写 `App::error`，不记已读。已读只在这里记：载入失败的帖子仍是未读；取两个数里较大的，是因为列表拿 `topic.replies` 来比，记得比它小就会一直显示 `~`。
- `Resize` 后按新宽度重新 `layout`，`scroll` 收回到合法范围。
- `↑` `k` / `↓` `j` 滚一行；`空格` `PgDn` `Ctrl-F` / `PgUp` `Ctrl-B` 翻一页（框内高度）；`Home` `g` / `End` `G` 到顶到底。`scroll` 的最大值是 `行数 - 框内高度`（不小于 0）。
- 已经在末尾时按 `空格`：等同 `]`。
- `]` / `[`：栈里紧挨着的下一层是帖子列表时，取它当前可见行里的下一篇 / 上一篇，把那个列表的游标移过去，用新的阅读页替换栈顶并发 `LoadThread`（像从列表打开一样算 `new_replies`，不记已读）。已经是最后一篇 / 第一篇，或下面不是帖子列表时无动作。
- 数字后 `Enter`：滚到该楼层那一块的第一行；楼层不存在时不动。
- `/` 打开查找输入，`Enter` 后滚到从当前位置往下第一处包含该文字的行（不分大小写）；`n` 下一处，`N` 上一处；到头不绕回；没有匹配时不动。匹配的行整行反白。
- `o`：`OpenUrl(帖子的 url)`。`r`：新票号，`LoadThread { refresh: true }`。`←` `q` `h` 返回。
- 左组段落：`V2EX`、节点的显示名（未知时省略）、`new_replies > 0` 时 `~ N 则新回复`；等结果时 `…`。位置是百分比 `(scroll + 框内高度) / 行数`，封顶 `100%`；行数不满一屏时是 `100%`。

- [ ] **Step 1: 写失败的测试**

```rust
// tui/reader.rs
#[test]
fn layout_puts_title_meta_body_then_one_block_per_reply() {
    // 帖子：标题「示例标题」、作者 alice、正文 "<p>第一段</p><pre>x = 1</pre>"；回复两则：bob「第一则」、alice「第二则」
    // 各行文字依次为：
    //   "示例标题"
    //   "alice · 2026-11-19 09:00 · 2 回复"
    //   ""
    //   "第一段"
    //   ""
    //   "  ▎x = 1"
    //   ""
    //   以 "#1 bob" 开头、以 "11/19 10:00" 结尾、显示宽度等于 width 的一行
    //   "    第一则"
    //   以 "#2 alice 楼主" 开头的一行
    //   "    第二则"
    // floors == [7, 9]
}
#[test]
fn no_line_is_wider_than_the_width_and_body_stops_at_100_columns() {
    // 正文是 500 个全角字符、一个 300 字符的网址；回复含 400 个 ASCII 字符的代码行
    // width 取 40、41、80、160：每一行 display_width <= width；width 160 时正文与回复内容的行不超过 100
}
#[test]
fn a_topic_without_body_or_replies_has_no_empty_blocks() {
    // content_html 为 None、零回复：行是标题、元信息两行；floors 为空
    // content_html 为 None、一则回复：标题、元信息、空行、回复块
}
#[test]
fn control_characters_in_names_and_titles_are_dropped() {
    // 标题 "a\x1b[2Jb"、用户名 "al\u{9b}ice"：所有行都不含控制字符
}

// tui/app.rs
#[test]
fn a_loaded_thread_is_laid_out_recorded_and_a_stale_one_is_dropped() {
    // 打开 id 1（票号 t1）后按 r（t2）：ThreadLoaded { ticket: t1 } 被丢弃
    // ThreadLoaded { ticket: t2, Ok } 之后 layout 非空，Cmd 含 SaveRead，read.mark(1, 回复条数) == Mark::Read
}
#[test]
fn a_failed_load_keeps_what_was_shown() {
    // 已有内容时 r，再送 Err(Error::Network(..))：layout 不变，App::error 非空
    // 首次加载就失败（如 Error::Restricted）：layout 为空，error 含「需要登录」，按键不 panic
}
#[test]
fn scrolling_and_paging_stay_in_range() {
    // 行数 100、框内高度 22：j 一次 scroll == 1；空格 → 23；End / G → 78；再 j 仍是 78；Home / g → 0；k 仍是 0
    // 行数 5（不满一屏）：所有滚动键之后 scroll == 0，位置是 "100%"
}
#[test]
fn a_number_then_enter_jumps_to_that_floor() {
    // '2' Enter → scroll 是 floors[1] 与最大值中较小的那个；'9' '9' Enter（没有这一楼）→ 不动
}
#[test]
fn bracket_keys_walk_the_list_and_space_at_the_end_goes_on() {
    // 栈 [Nodes, Topics(3 条), Reader(第 1 条)]：] → 栈顶换成第 2 条的 Reader，列表游标是 1，Cmd 含 LoadThread
    // [ 回到第 1 条；在第 1 条上按 [ 无动作；在第 3 条上按 ] 无动作
    // 滚到末尾后按空格等同 ]
    // 栈 [Nodes, Reader]（由 Start::Topic 进入）：] 与 [ 无动作，不 panic
}
#[test]
fn search_moves_between_matches_without_wrapping() {
    // / 键入 "uv" Enter → scroll 到第一处匹配的行；n → 下一处；最后一处再按 n 不动；N 往回
    // 查找一个不存在的词 → scroll 不动
}
#[test]
fn resize_reflows_and_keeps_scroll_in_range() {
    // scroll 在末尾时 Resize(160, 50)：行数变少，scroll <= 行数 - 框内高度（不小于 0）
    // Resize(20, 4) 再 Resize(80, 24) 不 panic
}
#[test]
fn opening_a_topic_with_more_replies_than_recorded_reports_the_difference() {
    // read 里记着回复数 5，列表里该帖回复数 8：打开后 ReaderScreen.new_replies == 3
}

// view —— TestBackend
#[test]
fn the_reader_keeps_its_border_and_shows_the_position() {
    // 80×24 与 40×8：每行最右一格是 │；下边框含 "%"
}
```

- [ ] **Step 2: 运行，确认失败**
- [ ] **Step 3: 实现**。`render::Role` 到 `Paint`：`Bold`→`Bold`、`Link`→`Link`、`Mention`→`Mention`、`Code`→`Code`、`Image`→`Image`、`Plain`→`Plain`。按键提示：`↑↓ 滚动`、`空格 翻页`、`] 下一篇`、`← 返回`。
- [ ] **Step 4: 运行，确认通过**
- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "feat(v2exe): add the reader with floor jump, search and next topic"
```

---

### Task 8: 说明页、运行时与命令行入口

**Files:**
- Create: `crates/v2exe/src/tui/help.rs`、`crates/v2exe/src/tui/runtime.rs`
- Modify: `crates/v2exe/src/tui/mod.rs`、`crates/v2exe/src/tui/app.rs`、`crates/v2exe/src/cli/args.rs`、`crates/v2exe/src/cli/run.rs`、`crates/v2exe/src/main.rs`
- Modify: `README.md`（状态行、特性、`-i` 选项与示例、配置一节）、`AGENTS.md` 目录地图（`crates/v2exe/` 的说明保持「非互动式子命令、互动式界面、正文渲染」，加上本地状态）
- Test: `help.rs` 的单元测试、`crates/v2exe/tests/cli.rs`、`crates/v2exe/src/cli/args.rs` 的单元测试

**Interfaces:**
- Consumes：前面全部。
- Produces：

```rust
// tui/mod.rs
pub struct Options { pub start: Start, pub edge: SegmentEdge, pub color: bool, pub tz: FixedOffset, pub dirs: Dirs }
/// 进入界面直到退出。返回前一定恢复终端（包括 panic 时）。
pub async fn run(client: Client, options: Options) -> io::Result<()>;

// tui/help.rs
/// 说明页的内容：spec §8.5 的四张按键表，每项是 (按键, 动作)，按「所有画面 / 列表通用 / 节点列表 / 帖子列表 / 阅读页」分组。
pub fn sections() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)>;

// cli/args.rs —— Cli 增加
#[arg(short = 'i', long, global = true)]
pub interactive: bool,
```

行为：

- **入口。** 不带子命令、或带 `-i`：进入界面。`-i hot` / `latest` / `tab <名>` / `node <名>` → `Start::Listing`；`-i topic <引用>` → `Start::Topic`（引用解析失败仍是退出码 2）；`-i nodes` → `Start::Home`。去掉 `arg_required_else_help`。
- **不是终端。** 要进入界面而 stdin 或 stdout 不是终端：退出码 2，stderr 写 `v2exe: 互动式界面需要在终端里运行；非互动式用法见 v2exe --help`。`--json` 与 `-i` 同时给：退出码 2。
- **运行时循环。** 一个线程阻塞读 `crossterm::event::read()`，把按键（只收 `KeyEventKind::Press`）与尺寸变化送进通道；主循环每收到一条 `Msg` 就 `update` 再画一帧；`App::quit` 为真时退出。
- **执行 `Cmd`。**
  - `LoadTopics` / `LoadThread` / `LoadDirectory`：各起一个任务。`refresh` 为真时先 `client.clear_cache()`。`LoadThread` 先 `topic(id)` 再 `replies(id, 1)`，前者失败就不调用后者。`LoadDirectory`：`refresh` 为假且 `nodes.json` 未过期时直接用它；否则调用 `client.nodes()`，成功后写 `nodes.json`。结果连同 `client.rate_limit()` 作为 `Msg` 送回。
  - `SaveFavorites` / `SaveRead`：调用对应的 `save()`；失败时送回 `Msg::SaveFailed(原因)`，`update` 把它写进 `App::error`。
  - `OpenUrl`：macOS 用 `open`，Linux 用 `xdg-open`，不等它结束；起不来时同样 `SaveFailed` 那条路径报原因。网址以 `http://` 或 `https://` 开头才打开。
- **终端。** 进入时开备用屏幕与 raw mode，退出与 panic 时恢复（用 ratatui 的 `init` / `restore`）。
- **说明页。** `Screen::Help` 画 `sections()` 的内容，可用 `↑↓`、翻页键滚动；`←` `q` `h` `?` `Esc` 返回。

- [ ] **Step 1: 写失败的测试**

```rust
// tui/help.rs
#[test]
fn help_lists_every_key_of_the_spec() {
    let keys: Vec<&str> = sections().iter().flat_map(|(_, rows)| rows.iter().map(|(k, _)| *k)).collect();
    for k in ["?", "Ctrl-C", "PgUp", "Home", "/", "s", "r", "y", "a", "d", "M", "v", "o", "]", "[", "n", "N", "g", "G"] {
        assert!(keys.iter().any(|x| x.contains(k)), "{k}");
    }
}

// cli/args.rs
#[test]
fn no_subcommand_parses_and_interactive_is_global() {
    // Cli::try_parse_from(["v2exe"]) 是 Ok，command 为 None
    // ["v2exe", "-i", "node", "python"] 与 ["v2exe", "node", "python", "-i"] 的 interactive 都为 true
}

// tests/cli.rs —— assert_cmd 起的进程没有终端
#[tokio::test]
async fn without_a_terminal_the_interactive_mode_exits_2_with_a_hint() {
    // `v2exe`、`v2exe -i hot`：退出码 2，stderr 含「需要在终端里运行」，stdout 为空，没有 "panicked"
}
#[tokio::test]
async fn interactive_rejects_json_and_a_bad_topic_reference() {
    // `v2exe -i --json hot` → 2；`v2exe -i topic abc` → 2
}
#[tokio::test]
async fn state_directories_that_cannot_be_used_do_not_stop_the_subcommands() {
    // HOME 未设、XDG_STATE_HOME 指向一个普通文件：`v2exe hot` 照常输出，退出码 0
}
```

原有的 `missing_subcommand_prints_help_and_exits_2` 改为上面第一个测试的行为。

- [ ] **Step 2: 运行，确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行，确认通过；再手动走一遍**

Run: `cargo test --workspace --locked`，然后 `cargo run -p v2exe`。
手动确认（各记一行结果进 commit message）：启动画面是我的最爱；进最热、打开一个帖子、`]` 到下一篇、返回；`y` 切到全部节点并加一个节点进我的最爱，退出重进后还在；读过的帖子标记消失；缩小终端到 30 列出现提示、放大后恢复；`Ctrl-C` 退出后终端正常。

- [ ] **Step 5: 更新 README 并 Commit**

README：状态行改为两种模式都可用；「非互动式」的选项表加 `-i`，示例加 `v2exe -i node python`；配置一节加 `segment_edge`。截图位保持 TODO。

```bash
git add crates README.md AGENTS.md Cargo.lock
git commit -m "feat(v2exe): run the interactive UI by default and with -i"
```
