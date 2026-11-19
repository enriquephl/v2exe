# v2exe 设计

以 PTT（BBS）为灵感、在终端里运行的 V2EX 阅读器。本地运行，没有服务器、数据库和部署。

## 1. 意图

现有的终端 V2EX 客户端各占下面三件事中的一到两件，没有一个三件都有。v2exe 要三件同时成立：

1. **不登录就能读。** 不需要 token，打开即用。
2. **PTT 式的操作。** 全键盘、列表占满终端高度、按编号跳转、列表内过滤、分页器式的阅读页、常驻的按键提示。
3. **可被脚本和 agent 调用。** 同一个程序不进界面也能把列表和帖子内容输出到 stdout，文本或 JSON。

数据层单独成库（`v2ex-rs`），界面和命令行都只通过它取数据，别的 Rust 项目也可以直接引用。

## 2. 目标、非目标、反目标

**第一版的目标**

- guest 只读：最热、最新、分 tab 列表、节点目录、节点的帖子列表、帖子正文与全部回复。
- 互动式界面：节点列表（我的最爱 / 全部节点）、帖子列表、阅读页、说明页。
- 非互动式子命令，输出文本或 JSON。
- 本地保存：我的最爱、已读状态、配置。
- 在 Linux 与 macOS 上从源码安装（`cargo install`）。

**第二版**：个人访问令牌登录（见第 11 节）。**更后面**：发帖与回帖、HTML 解析来源、搜索。

**第一版不做**：登录、任何写操作、搜索、解析网页 HTML、终端内显示图片、自更新、改键、用户资料页；预编译二进制、把 `v2ex-rs` 发布到 crates.io、Windows 支持。

**反目标**（技术上能跑也算失败）

- 绕过站方对游客的限制。网页对游客挡住的节点，guest 模式同样不显示。
- 不带 token 就什么都做不了。
- 不顾限流地请求接口。
- 在没有 Powerline 字体的终端上显示成乱码方块。

## 3. 约束

### 3.1 V2EX 数据源

以下为 2026-11-19 不带 cookie、不带 token 的实测。

| 来源 | 给什么 | 限制 |
|---|---|---|
| API 1.0 `/api/topics/hot.json` | 最热 10 条 | 不能翻页 |
| API 1.0 `/api/topics/latest.json` | 最新约 42 条 | 不能翻页 |
| API 1.0 `/api/topics/show.json?id=` | 单个帖子，返回只含一个元素的数组 | 没有点击数、附言 |
| API 1.0 `/api/topics/show.json?node_name=` | 节点最新 10 条 | 翻页参数被忽略 |
| API 1.0 `/api/replies/show.json?topic_id=` | 整帖全部回复，一次返回 | 没有楼层号、感谢数；楼层号按数组顺序推出 |
| API 1.0 `/api/nodes/all.json` | 全部节点，约 1400 个、约 800 KB | |
| API 1.0 `/api/nodes/show.json?name=` | 单个节点 | |
| Atom `/feed/{node}.xml` | 节点最新 50 条，含完整正文 HTML | 按发帖时间排；不能翻页；回复数只在链接的 `#replyN` 片段里 |
| Atom `/feed/tab/hot.xml`、`/index.xml` | 前一天的最热 40 条；全站最新 50 条 | 每条标题都带 `[节点标题]` 前缀，没有节点名 |
| Atom `/feed/tab/{tab}.xml`（hot 以外） | 分 tab 列表，50 到 60 条 | 不带任何节点信息，标题里的方括号是作者自己写的；会包含受限节点的帖子 |
| API 2.0 `/api/v2/…` | 节点列表翻页、回复翻页、通知 | 没有 token 一律 401 |

- **限流**：600 次 / 小时 / IP，API 1.0 与 2.0 共用一个额度，响应头 `X-Rate-Limit-Limit / -Remaining / -Reset` 给出余量。CDN 命中的响应不扣额度。feed 不带限流头。
- **排序**：API 1.0 与 feed 的节点列表按发帖时间排，网页按最后回复时间排。v2exe 第一版按发帖时间排，与 PTT 的文章列表习惯一致。
- **受限节点**：`flamewar`、`nsfw`、`sandbox` 的网页对游客跳转到登录页，但 API 1.0 仍返回其内容。网页的 tab 页对游客不显示这些节点的帖子，tab feed 却会包含。`nsfw` 与 `sandbox` 的节点 feed 正常，`flamewar` 的返回 404。
- **站点没有对非浏览器客户端设挑战**；主站在部分地区连不上，站点地址和代理必须可配置。
- **登录**：没有面向第三方的 OAuth。个人访问令牌在网页上生成，最长 180 天。

### 3.2 许可证

- 本仓 MIT。
- 可以贴着移植并署名的参考项目（均为 MIT）：`fuzy112/v2ex-tui`、`synrise25/v2term`、`seth-shi/go-v2ex`。贴近移植的代码在 `THIRD-PARTY-NOTICES` 里保留对方的版权与许可声明。
- `ptt/pttbbs` 是 GPL-2.0，只作为交互模型的参考，不复制其代码和格式串。

## 4. 做法的选择

| 决定 | 选了 | 没选的 | 理由 |
|---|---|---|---|
| 语言 | Rust（ratatui、tokio、reqwest、clap） | Go、TypeScript | Go 的理由原本是复用 go-v2ex，但它的数据层直接返回界面消息、从全局变量读 token，当不了库；crates.io 上没有 V2EX crate，数据层无论如何要自己写。TypeScript 分发要带运行时。 |
| 与 go-v2ex 的关系 | 对照物 | fork | 它没有非互动模式、节点目录、搜索，每页写死 10 行，交互模型不同；要改的比留下的多。 |
| 仓库切法 | 一个仓两个包：`v2ex-rs`（库）+ `v2exe`（程序） | 单个包内分模块；库单独一个仓 | 库不含界面代码才能被别处引用；接口没稳定前放在同一个仓，改一次不用跨两个仓。 |
| 第一版数据源 | 只用 API 1.0 + feed | 再加 HTML 解析；以 HTML 为主 | 只依赖站方承诺不改的接口。代价是每个节点最多 50 条、没有感谢数和附言。接口按可以再加来源的方式定义。 |
| 启动画面 | 「我的最爱」节点列表 | 主功能表；直接进帖子列表 | 少一层菜单，节点这一层始终可见。 |
| 版面 | 面板式外框 + 彩色段落 + 回复每则一块 | 原味 PTT 色块；素色骨架 | 见第 8 节。 |

## 5. 仓库结构

```
v2exe/
├── Cargo.toml              # workspace
├── crates/
│   ├── v2ex-rs/            # 库：类型、来源、限流、缓存；不含界面和命令行代码
│   └── v2exe/              # 程序
│       └── src/
│           ├── cli/        # 非互动式子命令与输出
│           ├── tui/        # 互动式界面
│           ├── render/     # 正文 HTML → 终端文本，cli 与 tui 共用
│           └── store/      # 配置、我的最爱、已读状态
├── docs/
├── LICENSE                 # MIT
└── THIRD-PARTY-NOTICES
```

依赖方向只有一条：`v2exe` → `v2ex-rs`。`cli` 与 `tui` 互不依赖，都依赖 `render`、`store` 和库。

## 6. `v2ex-rs`：数据层接口

本节是契约。实现之后以代码为准，本节不再同步维护。

### 6.1 调用面

```rust
pub struct Client;                       // 可廉价 clone
pub struct ClientBuilder;                // base_url、proxy、timeout、user_agent

pub enum Listing {
    Hot,
    Latest,
    Tab(String),                         // "tech"、"creative"、"all" …
    Node(String),                        // 节点名，如 "python"
}

pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,                       // 从 1 起
    pub has_more: bool,
}

impl Client {
    pub fn builder() -> ClientBuilder;

    pub async fn topics(&self, listing: &Listing, page: u32) -> Result<Page<Topic>>;
    pub async fn topic(&self, id: u64) -> Result<Topic>;
    pub async fn replies(&self, topic_id: u64, page: u32) -> Result<Page<Reply>>;
    pub async fn node(&self, name: &str) -> Result<Node>;
    pub async fn nodes(&self) -> Result<Vec<Node>>;

    pub fn rate_limit(&self) -> Option<RateLimit>;   // 最近一次 API 响应给出的余量
}
```

`page` 现在就进签名，是为了第二版接入 API 2.0 的翻页时调用方不用改。第一版里每个调用只有第 1 页：`has_more` 恒为 `false`，请求第 2 页及以后返回空的 `items`。

### 6.2 第一版各调用的来源

| 调用 | 来源 | 条数 |
|---|---|---|
| `topics(Hot)` | API 1.0 `hot.json` | 10 |
| `topics(Latest)` | API 1.0 `latest.json` | 约 42 |
| `topics(Tab(t))` | Atom `/feed/tab/{t}.xml` | 40 到 60 |
| `topics(Node(n))` | Atom `/feed/{n}.xml` | 50 |
| `topic(id)` | API 1.0 `topics/show.json?id=` | |
| `replies(id, 1)` | API 1.0 `replies/show.json?topic_id=` | 全部 |
| `node(name)` | API 1.0 `nodes/show.json?name=` | |
| `nodes()` | API 1.0 `nodes/all.json` | 约 1400 |

节点列表只取 feed，不再合并 API 1.0 的那 10 条：feed 已经有标题、作者、时间、回复数和正文，而且不扣额度。

### 6.3 类型

```rust
pub struct Topic {
    pub id: u64,
    pub title: String,                   // tab/hot feed 的 "[节点标题]" 前缀已拆进 node
    pub url: String,
    pub author: MemberRef,
    pub node: Option<NodeRef>,           // hot 以外的 tab feed 为 None
    pub created: DateTime<Utc>,
    pub last_touched: Option<DateTime<Utc>>,   // feed 取 <updated>，与 API 1.0 的 last_touched 一致
    pub replies: u32,                    // feed 取链接的 #replyN，没有片段为 0
    pub last_reply_by: Option<String>,   // 只有 API 1.0 有
    pub content: Option<String>,         // 原文，只有 API 1.0 有
    pub content_html: Option<String>,    // 渲染后的 HTML
}

pub struct Reply {
    pub id: u64,
    pub floor: u32,                      // 从 1 起，按返回顺序
    pub author: MemberRef,
    pub content: String,
    pub content_html: String,
    pub created: DateTime<Utc>,
}

pub struct Node {
    pub id: u64,
    pub name: String,                    // 节点名，URL 里的那一段
    pub title: String,
    pub title_alternative: Option<String>,
    pub topics: u32,
    pub stars: u32,
    pub header: Option<String>,
    pub parent_node_name: Option<String>,
}

pub struct MemberRef { pub username: String, pub id: Option<u64> }
pub struct NodeRef   { pub name: Option<String>, pub title: String }   // tab/hot feed 只有标题
pub struct RateLimit { pub limit: u32, pub remaining: u32, pub reset: DateTime<Utc> }
```

所有类型实现 `Serialize`，时间序列化为 RFC 3339。非互动式的 JSON 输出就是这些类型的序列化结果。

### 6.4 错误

```rust
pub enum Error {
    Network(..),                         // 连接失败、超时
    Status(u16),                         // 其余非 2xx
    NotFound,
    RateLimited { reset: Option<DateTime<Utc>> },   // 拿不到恢复时间时为空
    Restricted { node: String },         // 需要登录才能看的节点
    Decode(..),                          // 响应解析失败
}
```

### 6.5 行为

- **受限节点**：库内置一份受限节点名单，每项含节点名、节点标题、取其最近帖子编号的来源。初始三项：`flamewar`（水深火热，API 1.0 `topics/show.json?node_name=`，10 条）、`nsfw`（NSFW，节点 feed，50 条）、`sandbox`（沙盒，节点 feed，50 条）。发现新的再补。未登录时：
  - `topics(Node(受限))` 与 `node(受限)` 返回 `Restricted`。
  - `topic(id)` 取回后发现属于受限节点，返回 `Restricted`。
  - `Hot / Latest` 列表按节点名滤；`Tab("hot")` 按标题前缀里的节点标题滤。
  - 所有 `Tab` 列表（`hot` 以外的不带节点信息）：取列表时另取各受限节点最近的帖子编号，滤掉编号对得上的条目。tab feed 是这次新取的，就重新取这份编号集合；tab feed 来自缓存，就沿用已缓存的编号集合（最长缓存 10 分钟）；某个来源取失败时跳过它，列表照常返回。发帖较早、最近又有人回的受限帖子可能漏过，列表里只露出标题，打开时由 `topic(id)` 拦下。
  - `nodes()` 不含受限节点。
  - `replies(id)` 的响应不带节点信息，自己不做检查。程序在取回复之前一定先调用 `topic(id)`，由它把关。
- **限流**：每次 API 响应后记下余量。余量为 0 且未到恢复时间时，不发请求，直接返回 `RateLimited`。站方拒绝请求时同样映射为 `RateLimited`，不重试。余量只记在进程内，不跨进程保存：非互动式每次运行最多两次 API 请求，超额由站方拒绝、程序以退出码 4 结束。
- **重试**：连接失败、超时、5xx 最多重试 2 次，带退避。其余错误不自动重试。
- **缓存**：内存里按调用缓存，有效期与站方 CDN 一致（列表与帖子 120 秒，回复 60 秒，单个节点 1 小时，节点目录 2 小时）。提供绕过缓存重新请求的方式。
- **网络**：`base_url` 默认 `https://www.v2ex.com`；代理默认取环境变量，也可显式指定；超时默认 10 秒；User-Agent 为 `v2exe/<版本>`。

## 7. `v2exe`：非互动式

不带子命令进入互动式界面；带子命令则输出到 stdout 后退出。

```
v2exe                        进入互动式界面
v2exe hot                    最热
v2exe latest                 最新
v2exe tab <名字>             分 tab 列表
v2exe node <节点名>          该节点的帖子列表
v2exe topic <编号或网址>      帖子正文与全部回复
v2exe nodes [关键词]          节点目录，可按节点名或标题过滤
```

| 选项 | 作用 |
|---|---|
| `--json` | 输出 JSON。列表是 `Topic` 数组；`topic` 是 `{"topic": Topic, "replies": [Reply]}`；`nodes` 是 `Node` 数组 |
| `-i`、`--interactive` | 不输出，改为打开互动式界面并直接停在该位置（如 `v2exe -i node python`、`v2exe -i topic 123`） |
| `--limit <n>` | 列表最多输出 n 条 |
| `--no-replies` | `topic` 只输出正文 |
| `--no-color` | 关闭颜色；也遵守 `NO_COLOR`，stdout 不是终端时自动关闭 |
| `--base-url`、`--proxy`、`--timeout` | 覆盖配置文件 |

**文本输出**：列表每行一个帖子，编号在最前，后面是回复数、时间、作者、标题，这样编号可以直接接给 `v2exe topic`。`topic` 先输出标题与作者、节点、时间，再输出正文，然后每则回复一块，带楼层号。正文与回复经 `render` 转成纯文本。

**退出码**：0 成功；1 其他错误；2 用法错误；3 找不到；4 被限流；5 需要登录；6 网络错误。错误信息写到 stderr。

## 8. `v2exe`：互动式

### 8.1 画面

三个画面加一个说明页。导航是一条栈：节点列表 → 帖子列表 → 阅读页。

- **节点列表**（启动画面）：默认显示「我的最爱」，可切到全部节点。我的最爱里的条目可以是最热、最新、某个 tab 或某个节点；首次启动预置最热、最新和几个常用 tab。「全部节点」的最上面固定列出最热、最新和各个 tab，下面才是节点目录，所以这些条目被移出我的最爱后可以从这里加回来。tab 的名单没有接口可取，写死在程序里。
- **帖子列表**：每个帖子占两行，第一行是标记、标题、回复数，第二行是作者、日期，以及已知时的最后回复者。最新的在最上面，编号从 1 起。
- **阅读页**：标题、作者与时间、正文，然后是回复。每则回复一块：第一行是楼层号、名字、楼主标记，时间靠右；内容缩进 4 格在下面。打开帖子时总是调用 `topic(id)` 与 `replies(id)`，不复用列表里 feed 带的正文：这样字段齐全，受限节点的检查也只有一条路径。
- **说明页**：完整的按键表。

列表占满终端的可用高度，随终端大小变化。

### 8.2 外框与段落

每个画面是一个圆角边框的面板。上边框嵌两组段落，下边框左边是按键提示、右边是位置。

```
╭ V2EX  python  +4 ~2 ─────────────────────────────  ✔ API 571/600  guest ╮
│ ❯ ~ 你们的项目现在还在用 requirements.txt 吗                         爆 │
│     cedar_lin · 11/19 · 最后回复 ginkgo                                 │
│   + 类型标注写到什么程度算够                                         12 │
│     elm · 11/18 · 最后回复 ivy                                          │
│     用 uv 替换 pip 一个月后的体会                                    27 │
│     dogwood · 11/19 · 最后回复 hazel                                    │
╰ ↑↓ 移动   → 阅读   s 换节点   / 找标题   ? 说明 ────────────────  3/50 ╯
```

```
╭ V2EX  python  ~ 2 则新回复 ──────────────────────  ✔ API 570/600  guest ╮
│ 你们的项目现在还在用 requirements.txt 吗                                │
│ cedar_lin · 2026-11-19 09:41 · 6 回复                                   │
│                                                                         │
│ 接手了一个五年前的项目，依赖还是一份 requirements.txt 加一份开发用的。  │
│   ▎pip install -r requirements.txt                                      │
│                                                                         │
│ #1 dogwood                                                  11/19 09:48 │
│     早就换 pyproject.toml 了，锁文件交给 uv。                           │
│ #5 cedar_lin 楼主                                           11/19 10:30 │
│     谢谢各位，先从开发依赖开始迁。                                      │
╰ ↑↓ 滚动   空格 翻页   ] 下一篇   ← 返回 ──────────────────────────  62% ╯
```

上面两幅只示意内容的位置，不按列对齐。

**段落的内容**

| 位置 | 段落 | 内容 |
|---|---|---|
| 左 1 | 站点 | `V2EX` |
| 左 2 | 位置 | 我的最爱 / 全部节点 / 节点名 / tab 名 |
| 左 3 | 状态 | 列表：`+N ~M`（未读数与有新回复数，都为 0 时不显示）；阅读页：有新回复时 `~ N 则新回复`；加载中时显示加载标记 |
| 右 1 | 接口 | `✔ API 余量/总量`；余量低于两成变黄；耗尽时变红并写恢复时间；上一次请求失败时变红并写原因 |
| 右 2 | 身份 | `guest` |

**段落的画法**：每个段落是一块底色，字用终端的背景色（反白）。底色只用终端的 16 色，所以跟随终端主题。段落边缘有两种：`plain` 方角，`powerline` 箭头形（需要带 Powerline 字符的字体）。由配置决定，默认 `plain`。

### 8.3 颜色

只用终端自己的 16 色，不用固定的 256 色或真彩色。遵守 `NO_COLOR`，关闭颜色后只靠反白、粗体、暗色区分。

| 颜色 | 含义 |
|---|---|
| 蓝 | 位置：节点名、按键、`@` 提及、链接 |
| 绿 | 新的：未读标记 `+`、游标 `❯`、额度充足、代码块左边的竖条 |
| 黄 | 有变化：读过之后有新回复 `~`、回复数 10 到 99、额度偏低 |
| 红 | 回复数 100 以上的「爆」、出错 |
| 青 | 人名 |
| 洋红 | 楼主标记 |
| 暗色 | 次要信息：日期、编号、楼层号 |

回复数分三档：100 以上写「爆」（红、粗体），10 到 99 黄色粗体，1 到 9 绿色，0 不显示。

### 8.4 标记与已读状态

| 标记 | 含义 |
|---|---|
| `+` | 没打开过 |
| `~` | 打开过，之后回复数增加了 |
| 空白 | 已读 |

打开一个帖子时记下它当时的回复数。`v` 在已读和未读之间切换。

### 8.5 按键

所有画面：`?` 说明，`Ctrl-C` 退出。

**列表通用**（节点列表、帖子列表）

| 按键 | 动作 |
|---|---|
| `↑` `k` / `↓` `j` | 上下一行 |
| `PgUp` `Ctrl-B` / `PgDn` `空格` `Ctrl-F` | 翻页 |
| `Home` / `End` `$` | 第一行 / 最后一行 |
| 数字后 `Enter` | 跳到该编号 |
| `→` `Enter` `l` | 进入 |
| `←` `q` `h` | 返回；在启动画面则退出 |
| `/` | 过滤（节点列表按节点名和标题，帖子列表按标题）；再按 `←` 取消过滤 |
| `s` | 输入节点名直达，带补全；也接受 `hot`、`latest` 和 tab 名 |
| `r` | 重新请求 |

**节点列表**：`y` 切换我的最爱 / 全部节点，`a` 加入我的最爱，`d` 移出，`M` 移动位置。

**帖子列表**：`a` 按作者过滤，`v` 切换已读，`o` 用浏览器打开。

**阅读页**

| 按键 | 动作 |
|---|---|
| `↑` `k` / `↓` `j` | 滚动一行 |
| `空格` `PgDn` `Ctrl-F` / `PgUp` `Ctrl-B` | 翻页；在末尾按 `空格` 进入下一篇 |
| `Home` `g` / `End` `G` | 顶部 / 底部 |
| 数字后 `Enter` | 跳到该楼层 |
| `/`，`n`，`N` | 页内查找，下一个，上一个 |
| `]` / `[` | 列表里的下一篇 / 上一篇 |
| `o` | 用浏览器打开 |
| `r` | 重新请求 |
| `←` `q` `h` | 返回 |

`h` 是返回而不是 PTT 的说明：`h j k l` 四个键保持 vim 的方向含义，说明用 `?`。

### 8.6 正文渲染

`render` 把 `content_html` 转成带样式的行，界面和文本输出共用。支持：段落与换行、链接、图片（显示为一行带网址的占位，不在终端里画图）、代码块、引用、列表、标题、粗体、`@` 提及。按显示宽度折行，中日韩字符算两格。

### 8.7 网络与错误

界面不因网络请求而卡住。请求返回时如果用户已经离开那个画面，结果丢弃。请求失败时保留已有内容，右上的接口段落变红并写原因，`r` 重试。被限流时写明恢复时间。

终端小于最小尺寸时只显示一行提示，不崩溃。

## 9. 本地文件

| 文件 | 内容 |
|---|---|
| `$XDG_CONFIG_HOME/v2exe/config.toml`（默认 `~/.config/v2exe/`） | 站点地址、代理、超时、段落边缘（`plain` / `powerline`） |
| `$XDG_STATE_HOME/v2exe/`（默认 `~/.local/state/v2exe/`） | 我的最爱（有序）、已读状态（帖子编号 → 当时的回复数与时间，保留最近 10000 条） |
| `$XDG_CACHE_HOME/v2exe/`（默认 `~/.cache/v2exe/`） | 节点目录，24 小时内不重新下载 |

状态文件先写临时文件再改名，避免写到一半损坏。配置文件缺失时全部取默认值，程序不自动创建它。

## 10. 测试

- **库**：用保存下来的真实响应（API 1.0 的 JSON、Atom）做解析测试；用本地模拟服务器测限流、重试、受限节点、错误映射。
- **render**：HTML 片段 → 行的对照测试，含中日韩字符折行。
- **cli**：文本与 JSON 输出的对照测试、退出码。
- **tui**：按键 → 状态变化；列表行数随终端高度变化；任何一行都不超出终端宽度。
- 颜色、间距、版面好不好看由人看了算，不写断言。实现中改动版面时，先给人看过再定。
- CI 在 Linux 与 macOS 上跑格式检查、lint 和测试。

## 11. 第二版：个人访问令牌（方向）

不在第一版的计划里，这里只写到足以确认第 6 节的接口不用改。

- `ClientBuilder` 增加 `token`。带 token 时：`topics(Node, page)` 与 `replies(id, page)` 改用 API 2.0 并真正翻页；受限节点可见；新增通知与本人资料两个调用。
- 令牌存系统钥匙串，可用环境变量覆盖；不落明文文件。
- 新增 `v2exe login`（粘贴令牌、校验、显示到期时间）与 `v2exe logout`；临近到期时在界面里提示去哪里换。
- 右上的身份段落从 `guest` 变成用户名。

## 12. 留给实现者

- 各依赖的具体选型与版本（Atom 解析、HTML 解析、时间库等），Rust 用当时的稳定版。
- 包内的模块划分、界面的事件循环与状态组织。
- 文本输出的具体列宽与对齐。
- 绕过缓存的调用形式。
- 最小终端尺寸的具体数值、阅读页正文的最大行宽。
- 写死的 tab 名单，以及预置进我的最爱的条目，以对应 feed 实际存在为准。
- 额度耗尽时站方返回的状态码与响应头没有实测过。实现时把 429 以及余量头为 0 的拒绝响应都当作被限流处理。

## 13. 参考项目

| 项目 | 用途 |
|---|---|
| [seth-shi/go-v2ex](https://github.com/seth-shi/go-v2ex) | 最接近的现成品，对照物；已读记录与中日韩宽度处理的做法 |
| [fuzy112/v2ex-tui](https://github.com/fuzy112/v2ex-tui) | 同栈（ratatui + tokio + reqwest）；子命令与 JSON 输出、节点数据、按键表 |
| [synrise25/v2term](https://github.com/synrise25/v2term) | 不依赖界面的 HTML 解析数据层，日后加 HTML 来源时参考 |
| [liaoliao666/v2ex](https://github.com/liaoliao666/v2ex) | 登录流程最完整的实现，日后做 cookie 登录时参考 |
| [ptt/pttbbs](https://github.com/ptt/pttbbs) | PTT 的交互模型：画面分区、按键表（只参考，不复制代码） |
| [ptt/ptt-term](https://github.com/ptt/ptt-term) | term.ptt.cc 的源码；配色表与字符宽度处理 |
| [romkatv/powerlevel10k](https://github.com/romkatv/powerlevel10k) | 段落的画法与颜色含义 |
| [aome510/hackernews-TUI](https://github.com/aome510/hackernews-TUI) | HTML → 带样式文本、按键与主题配置 |
| [pierreyoda/hncli](https://github.com/pierreyoda/hncli) | ratatui 下 router / screens / components 的划分 |
