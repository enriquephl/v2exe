//! Draws the node list, a topic list and the reader with made-up content and writes each as a
//! standalone HTML page, one terminal cell per fixed-width box, for turning into the README's
//! pictures:
//!
//! ```text
//! cargo run -p v2exe --example screenshots -- <output directory>
//! ```
//!
//! A headless browser then prints each page: `--headless --screenshot=nodes.png nodes.html`.
//! Add `?theme=light` to the page address for the light palette.

use std::fmt::Write as _;
use std::path::Path;

use chrono::{DateTime, Duration, FixedOffset, TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;
use v2ex_rs::{Listing, MemberRef, NodeRef, RateLimit, Reply, Topic};
use v2exe::store::config::SegmentEdge;
use v2exe::store::favorites::Favorites;
use v2exe::store::read_state::ReadState;
use v2exe::tui::Start;
use v2exe::tui::app::{App, Cmd, Msg, view};
use v2exe::tui::theme::Theme;

const SIZE: (u16, u16) = (96, 28);

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: screenshots <output directory>");
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).expect("the output directory can be created");

    let now = Utc.with_ymd_and_hms(2026, 11, 19, 2, 40, 0).unwrap();
    let tz = FixedOffset::east_opt(8 * 3600).unwrap();

    let (mut home, _) = App::new(
        Start::Home,
        Favorites::load(None),
        ReadState::load(None),
        SIZE,
        tz,
    );
    home.quota = quota(now, 600);
    press(&mut home, KeyCode::Char('j'), now);
    press(&mut home, KeyCode::Char('j'), now);
    save(dir, "nodes", &draw(&home, now));

    let mut read = ReadState::load(None);
    for (id, replies) in [(1003, 27), (1004, 3), (1006, 41), (1008, 0)] {
        read.record(id, replies, now);
    }
    read.record(1001, 4, now);
    let (mut app, cmds) = App::new(
        Start::Listing(Listing::Node("python".into())),
        Favorites::load(None),
        read,
        SIZE,
        tz,
    );
    let ticket = ticket_of(&cmds);
    app.update(
        Msg::TopicsLoaded {
            ticket,
            result: Ok(topics(now)),
            quota: quota(now, 571),
        },
        now,
    );
    save(dir, "topics", &draw(&app, now));

    let cmds = press(&mut app, KeyCode::Enter, now);
    let ticket = ticket_of(&cmds);
    let (topic, replies) = thread(now);
    app.update(
        Msg::ThreadLoaded {
            ticket,
            result: Ok((topic, replies)),
            quota: quota(now, 569),
        },
        now,
    );
    save(dir, "reader", &draw(&app, now));
}

fn quota(now: DateTime<Utc>, remaining: u32) -> Option<RateLimit> {
    Some(RateLimit {
        limit: 600,
        remaining,
        reset: now + Duration::minutes(20),
    })
}

fn press(app: &mut App, code: KeyCode, now: DateTime<Utc>) -> Vec<Cmd> {
    app.update(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)), now)
}

fn ticket_of(cmds: &[Cmd]) -> u64 {
    cmds.iter()
        .find_map(|cmd| match cmd {
            Cmd::LoadTopics { ticket, .. } | Cmd::LoadThread { ticket, .. } => Some(*ticket),
            _ => None,
        })
        .expect("the screen asked for its content")
}

fn member(name: &str) -> MemberRef {
    MemberRef {
        username: name.into(),
        id: None,
    }
}

fn topic(
    id: u64,
    title: &str,
    author: &str,
    replies: u32,
    last: Option<&str>,
    created: DateTime<Utc>,
) -> Topic {
    Topic {
        id,
        title: title.into(),
        url: format!("https://www.v2ex.com/t/{id}"),
        author: member(author),
        node: Some(NodeRef {
            name: Some("python".into()),
            title: "Python".into(),
        }),
        created,
        last_touched: None,
        replies,
        last_reply_by: last.map(Into::into),
        content: None,
        content_html: None,
    }
}

fn topics(now: DateTime<Utc>) -> Vec<Topic> {
    let ago = |hours: i64| now - Duration::hours(hours);
    vec![
        topic(
            1001,
            "你们的项目现在还在用 requirements.txt 吗",
            "cedar_lin",
            6,
            Some("ginkgo"),
            ago(1),
        ),
        topic(
            1002,
            "类型标注写到什么程度算够",
            "elm",
            12,
            Some("ivy"),
            ago(3),
        ),
        topic(
            1003,
            "用 uv 替换 pip 一个月后的体会",
            "dogwood",
            27,
            Some("hazel"),
            ago(5),
        ),
        topic(
            1004,
            "asyncio 里的任务取消，哪些地方最容易踩坑",
            "juniper",
            3,
            Some("elm"),
            ago(9),
        ),
        topic(
            1005,
            "一个三千行的脚本要不要拆成包",
            "larch",
            142,
            Some("maple"),
            ago(14),
        ),
        topic(
            1006,
            "给内部工具写命令行界面：argparse、click 还是 typer",
            "maple",
            41,
            Some("dogwood"),
            ago(20),
        ),
        topic(
            1007,
            "有没有人把测试从 unittest 迁到 pytest，值得吗",
            "nettle",
            8,
            None,
            ago(26),
        ),
        topic(1008, "求推荐一本讲解释器实现的书", "oak", 0, None, ago(31)),
        topic(
            1009,
            "在 CI 里缓存虚拟环境的几种做法 🚀",
            "poplar",
            19,
            Some("cedar_lin"),
            ago(40),
        ),
        topic(
            1010,
            "数据类、命名元组和普通字典，你们怎么选",
            "quince",
            5,
            Some("larch"),
            ago(52),
        ),
        topic(
            1011,
            "打包成单个可执行文件，现在哪种方案最省心",
            "rowan",
            33,
            Some("oak"),
            ago(60),
        ),
        topic(
            1012,
            "写了五年 Python 才知道的标准库模块",
            "sorrel",
            76,
            Some("nettle"),
            ago(71),
        ),
        topic(
            1013,
            "日志该打多少：一次线上排查之后的反思",
            "thyme",
            2,
            Some("ivy"),
            ago(80),
        ),
    ]
}

fn reply(floor: u32, author: &str, html: &str, created: DateTime<Utc>) -> Reply {
    Reply {
        id: 9000 + u64::from(floor),
        floor,
        author: member(author),
        content: String::new(),
        content_html: html.into(),
        created,
    }
}

fn thread(now: DateTime<Utc>) -> (Topic, Vec<Reply>) {
    let at = |minutes: i64| now - Duration::minutes(60 - minutes);
    let mut opened = topic(
        1001,
        "你们的项目现在还在用 requirements.txt 吗",
        "cedar_lin",
        6,
        Some("ginkgo"),
        at(0),
    );
    opened.content_html = Some(
        "<p>接手了一个五年前的项目，依赖还是一份 requirements.txt 加一份开发用的。\
         想问问大家现在都怎么管依赖，迁移的时候有什么要留意的。</p>\
         <pre><code>pip install -r requirements.txt\npip install -r requirements-dev.txt</code></pre>\
         <p>目前的想法是先迁到 <a href=\"https://example.org/pyproject\">pyproject.toml</a>，锁文件之后再说。</p>"
            .into(),
    );
    let replies = vec![
        reply(
            1,
            "dogwood",
            "<p>早就换 pyproject.toml 了，锁文件交给 uv。</p>",
            at(7),
        ),
        reply(2, "elm", "<p>小项目没必要折腾，能跑就行。</p>", at(12)),
        reply(
            3,
            "ivy",
            "<p>@<a href=\"/member/elm\">elm</a> 能跑和能复现是两回事，新同事装环境的时候就知道了。</p>",
            at(20),
        ),
        reply(
            4,
            "hazel",
            "<blockquote><p>锁文件之后再说</p></blockquote><p>建议一步到位，不然会长期停在半迁移的状态。</p>",
            at(31),
        ),
        reply(
            5,
            "cedar_lin",
            "<p>谢谢各位，先从开发依赖开始迁。</p>",
            at(49),
        ),
        reply(
            6,
            "ginkgo",
            "<p>迁完记得把 CI 里的安装命令一起改掉。</p>",
            at(55),
        ),
    ];
    (opened, replies)
}

fn draw(app: &App, now: DateTime<Utc>) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).expect("a test terminal");
    terminal
        .draw(|frame| view(app, &Theme { color: true }, SegmentEdge::Plain, now, frame))
        .expect("drawing into a test terminal");
    terminal.backend().buffer().clone()
}

fn save(dir: &Path, name: &str, buffer: &Buffer) {
    let path = dir.join(format!("{name}.html"));
    std::fs::write(&path, page(name, buffer)).expect("the page can be written");
    println!("{}", path.display());
}

/// The CSS name of a colour; `None` is the terminal's default.
fn color_name(color: Color) -> Option<&'static str> {
    Some(match color {
        Color::Black => "black",
        Color::Red | Color::LightRed => "red",
        Color::Green | Color::LightGreen => "green",
        Color::Yellow | Color::LightYellow => "yellow",
        Color::Blue | Color::LightBlue => "blue",
        Color::Magenta | Color::LightMagenta => "magenta",
        Color::Cyan | Color::LightCyan => "cyan",
        Color::Gray | Color::White => "white",
        Color::DarkGray => "gray",
        _ => return None,
    })
}

fn page(title: &str, buffer: &Buffer) -> String {
    let mut rows = String::new();
    for y in 0..buffer.area.height {
        rows.push_str("<div class=\"r\">");
        let mut hidden = 0;
        for x in 0..buffer.area.width {
            if hidden > 0 {
                hidden -= 1;
                continue;
            }
            let cell = &buffer[(x, y)];
            let symbol = cell.symbol();
            let width = symbol.width().max(1);
            hidden = width - 1;

            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (fg, bg) = (color_name(cell.fg), color_name(cell.bg));
            let (fg, bg) = if reversed {
                (Some(bg.unwrap_or("bg")), Some(fg.unwrap_or("fg")))
            } else {
                (fg, bg)
            };
            let mut classes = String::new();
            if width == 2 {
                classes.push_str(" w");
            }
            if let Some(fg) = fg {
                let _ = write!(classes, " f-{fg}");
            }
            if let Some(bg) = bg {
                let _ = write!(classes, " b-{bg}");
            }
            if cell.modifier.contains(Modifier::BOLD) {
                classes.push_str(" bold");
            }
            if cell.modifier.contains(Modifier::DIM) {
                classes.push_str(" dim");
            }
            let text = match symbol {
                "&" => "&amp;",
                "<" => "&lt;",
                ">" => "&gt;",
                " " => "&nbsp;",
                other => other,
            };
            let _ = write!(rows, "<i class=\"{}\">{text}</i>", classes.trim_start());
        }
        rows.push_str("</div>\n");
    }
    PAGE.replace("{title}", title)
        .replace("{columns}", &buffer.area.width.to_string())
        .replace("{rows}", &rows)
}

const PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>v2exe {title}</title>
<style>
:root {
  --bg: #1b1d23; --fg: #d7dae0; --black: #2a2d35; --gray: #7d8590;
  --red: #e06c75; --green: #8cc265; --yellow: #d9b25a; --blue: #61a0ef;
  --magenta: #c678dd; --cyan: #4fb6c2; --white: #abb2bf; --page: #0f1014;
}
:root.light {
  --bg: #fbfbfc; --fg: #2b2f36; --black: #e6e8eb; --gray: #8a919b;
  --red: #c4343f; --green: #3f8f2f; --yellow: #a8720a; --blue: #2f6fd0;
  --magenta: #9b3fb5; --cyan: #17808c; --white: #5c6370; --page: #e9eaee;
}
html, body { margin: 0; background: var(--page); }
.term {
  display: inline-block; margin: 24px; padding: 14px 16px; border-radius: 10px;
  background: var(--bg); color: var(--fg);
  font: 15px/1.25 "SF Mono", Menlo, "DejaVu Sans Mono", "PingFang SC", monospace;
  width: {columns}ch;
}
.r { height: 1.25em; white-space: nowrap; }
i { display: inline-block; width: 1ch; height: 1.25em; font-style: normal; text-align: center; vertical-align: top; overflow: hidden; }
i.w { width: 2ch; }
.bold { font-weight: 700; }
.dim { color: var(--gray); }
.f-fg { color: var(--fg); } .f-bg { color: var(--bg); }
.b-fg { background: var(--fg); } .b-bg { background: var(--bg); }
.f-black { color: var(--black); } .b-black { background: var(--black); }
.f-red { color: var(--red); } .b-red { background: var(--red); }
.f-green { color: var(--green); } .b-green { background: var(--green); }
.f-yellow { color: var(--yellow); } .b-yellow { background: var(--yellow); }
.f-blue { color: var(--blue); } .b-blue { background: var(--blue); }
.f-magenta { color: var(--magenta); } .b-magenta { background: var(--magenta); }
.f-cyan { color: var(--cyan); } .b-cyan { background: var(--cyan); }
.f-white { color: var(--white); } .b-white { background: var(--white); }
.f-gray { color: var(--gray); } .b-gray { background: var(--gray); }
</style>
<script>
if (new URLSearchParams(location.search).get("theme") === "light") {
  document.documentElement.classList.add("light");
}
</script>
<div class="term">
{rows}</div>
"#;
