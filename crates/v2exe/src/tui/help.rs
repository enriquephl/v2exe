//! The help page: every key of every screen.

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line as TextLine, Span};

use super::app::{App, Screen, hint, segment};
use super::frame::{Chrome, Tone, draw_frame};
use super::reader::RLine;
use super::theme::{Paint, Theme};
use crate::render::{Line, LineKind, Role, Seg, display_width, wrap};
use crate::store::config::SegmentEdge;

/// The keys of spec §8.5 as (keys, action), grouped by where they work.
pub fn sections() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        ("所有画面", vec![("?", "说明"), ("Ctrl-C", "退出")]),
        (
            "列表通用",
            vec![
                ("↑ k / ↓ j", "上一行 / 下一行"),
                ("PgUp Ctrl-B", "上一页"),
                ("PgDn 空格 Ctrl-F", "下一页"),
                ("Home / End $", "第一行 / 最后一行"),
                ("数字后 Enter", "跳到该编号"),
                ("→ Enter l", "进入"),
                ("← q h", "返回；在启动画面则退出"),
                (
                    "/",
                    "过滤（节点列表按节点名和标题，帖子列表按标题）；再按 ← 取消过滤",
                ),
                (
                    "s",
                    "输入节点名直达，按 Tab 补全；也接受 hot、latest 和 tab 名",
                ),
                ("r", "重新请求"),
            ],
        ),
        (
            "节点列表",
            vec![
                ("y", "切换我的最爱 / 全部节点"),
                ("a", "加入我的最爱"),
                ("d", "移出我的最爱"),
                ("M", "移动位置"),
            ],
        ),
        (
            "帖子列表",
            vec![
                ("a", "按作者过滤"),
                ("v", "切换已读"),
                ("o", "用浏览器打开"),
            ],
        ),
        (
            "阅读页",
            vec![
                ("↑ k / ↓ j", "滚动一行"),
                ("空格 PgDn Ctrl-F", "下一页；在末尾按空格进入下一篇"),
                ("PgUp Ctrl-B", "上一页"),
                ("Home g / End G", "顶部 / 底部"),
                ("数字后 Enter", "跳到该楼层"),
                ("/", "页内查找"),
                ("n / N", "下一个 / 上一个"),
                ("] / [", "列表里的下一篇 / 上一篇"),
                ("o", "用浏览器打开"),
                ("r", "重新请求"),
                ("← q h", "返回"),
            ],
        ),
    ]
}

/// Columns before the keys of a row.
const INDENT: usize = 2;

/// Columns between the widest keys and the actions.
const GAP: usize = 2;

/// The sections as lines of at most `width` columns: each section's title, then a row per entry
/// with the keys in a column as wide as the widest and the action wrapped beside them, and a
/// blank line between sections.
fn lines(width: usize) -> Vec<RLine> {
    let sections = sections();
    let keys_width = sections
        .iter()
        .flat_map(|(_, rows)| rows.iter().map(|(keys, _)| display_width(keys)))
        .max()
        .unwrap_or(0);
    let column = INDENT + keys_width + GAP;
    let mut out = Vec::new();
    for (i, (title, rows)) in sections.into_iter().enumerate() {
        if i > 0 {
            out.push(RLine::default());
        }
        out.push(RLine {
            spans: vec![(title.to_string(), Paint::Bold)],
        });
        for (keys, action) in rows {
            let action = Line {
                kind: LineKind::Text,
                segs: vec![Seg {
                    text: action.to_string(),
                    role: Role::Plain,
                }],
            };
            for (j, piece) in wrap(&[action], width.saturating_sub(column))
                .iter()
                .enumerate()
            {
                let lead = if j == 0 {
                    vec![
                        (" ".repeat(INDENT), Paint::Plain),
                        (keys.to_string(), Paint::Location),
                        (
                            " ".repeat(keys_width - display_width(keys) + GAP),
                            Paint::Plain,
                        ),
                    ]
                } else {
                    vec![(" ".repeat(column), Paint::Plain)]
                };
                let mut spans = lead;
                spans.push((piece.text(), Paint::Plain));
                out.push(RLine { spans });
            }
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct HelpScreen {
    /// The line at the top of the page.
    pub scroll: usize,
}

impl HelpScreen {
    /// Keeps `scroll` in range for lines of `width` columns on a page of `page` rows.
    pub(super) fn clamp(&mut self, width: usize, page: usize) {
        self.scroll = self.scroll.min(max_scroll(width, page));
    }
}

/// The last line the page can start at: the one that puts the last line at the bottom.
fn max_scroll(width: usize, page: usize) -> usize {
    lines(width).len().saturating_sub(page)
}

impl App {
    /// A key while the help page is on top: the reader's scrolling keys, and back.
    pub(super) fn help_key(&mut self, key: KeyEvent) {
        let (width, page) = (self.reader_width(), self.inner_height());
        let Some(Screen::Help(screen)) = self.stack.last_mut() else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let (scroll, last) = (screen.scroll, max_scroll(width, page));
        let target = match key.code {
            KeyCode::Up => scroll.saturating_sub(1),
            KeyCode::Down => scroll + 1,
            KeyCode::PageUp => scroll.saturating_sub(page),
            KeyCode::PageDown => scroll + page,
            KeyCode::Home => 0,
            KeyCode::End => last,
            KeyCode::Left | KeyCode::Esc => {
                self.stack.pop();
                return;
            }
            KeyCode::Char('b') if ctrl => scroll.saturating_sub(page),
            KeyCode::Char('f') if ctrl => scroll + page,
            KeyCode::Char(c) if plain => match c {
                'k' => scroll.saturating_sub(1),
                'j' => scroll + 1,
                ' ' => scroll + page,
                'g' => 0,
                'G' => last,
                'q' | 'h' | '?' => {
                    self.stack.pop();
                    return;
                }
                _ => return,
            },
            _ => return,
        };
        screen.scroll = target.min(last);
    }
}

/// Draws the help page.
pub(super) fn draw(
    app: &App,
    screen: &HelpScreen,
    theme: &Theme,
    edge: SegmentEdge,
    now: DateTime<Utc>,
    frame: &mut Frame,
) {
    let lines = lines(app.reader_width());
    let page = app.inner_height();
    let position = if lines.len() <= page {
        "100%".to_string()
    } else {
        format!("{}%", ((screen.scroll + page) * 100 / lines.len()).min(100))
    };
    let chrome = Chrome {
        left: vec![segment("V2EX", Tone::Site), segment("说明", Tone::Location)],
        right: app.right_segments(now),
        hints: vec![hint("↑↓", "滚动"), hint("←", "返回")],
        position,
    };
    let inner = draw_frame(frame, frame.area(), &chrome, theme, edge);
    let area = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    let buf = frame.buffer_mut();
    for (y, line) in (area.top()..area.bottom()).zip(lines.iter().skip(screen.scroll)) {
        let spans: Vec<Span> = line
            .spans
            .iter()
            .map(|(text, paint)| Span::styled(text.as_str(), theme.style(*paint)))
            .collect();
        buf.set_line(area.x, y, &TextLine::from(spans), area.width);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::render::display_width;
    use crate::tui::Start;
    use crate::tui::app::testing::draw;
    use crate::tui::app::testing::*;
    use crate::tui::app::{App, Msg, Screen};

    #[test]
    fn help_lists_every_key_of_the_spec() {
        let keys: Vec<&str> = sections()
            .iter()
            .flat_map(|(_, rows)| rows.iter().map(|(k, _)| *k))
            .collect();
        for k in [
            "?", "Ctrl-C", "PgUp", "Home", "/", "s", "r", "y", "a", "d", "M", "v", "o", "]", "[",
            "n", "N", "g", "G",
        ] {
            assert!(keys.iter().any(|x| x.contains(k)), "{k}");
        }
        let titles: Vec<&str> = sections().iter().map(|(title, _)| *title).collect();
        assert_eq!(
            titles,
            ["所有画面", "列表通用", "节点列表", "帖子列表", "阅读页"]
        );
    }

    #[test]
    fn no_line_is_wider_than_the_width_and_no_action_is_lost() {
        for width in [36, 37, 76, 196] {
            let laid = lines(width);
            for line in &laid {
                assert!(display_width(&line.text()) <= width, "{width}: {line:?}");
            }
            let text: String = laid
                .iter()
                .map(|line| line.text().replace(' ', ""))
                .collect();
            for (_, rows) in sections() {
                for (_, action) in rows {
                    assert!(text.contains(&action.replace(' ', "")), "{width}: {action}");
                }
            }
        }
    }

    fn help(app: &App) -> &HelpScreen {
        match app.stack.last() {
            Some(Screen::Help(help)) => help,
            _ => panic!("the help page is not on top"),
        }
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_help_page_scrolls_like_the_reader_and_stays_in_range() {
        let (mut app, _) = launch(Start::Home);
        app.update(Msg::Resize(80, 12), now());
        code(&mut app, KeyCode::Char('?'));
        let page = 10;
        let last = lines(76).len() - page;
        assert!(last > 2 * page, "{last}");
        let steps = [
            (plain(KeyCode::Char('j')), 1),
            (plain(KeyCode::Down), 2),
            (plain(KeyCode::Char('k')), 1),
            (plain(KeyCode::Up), 0),
            (plain(KeyCode::Up), 0),
            (plain(KeyCode::PageDown), page),
            (plain(KeyCode::Char(' ')), 2 * page),
            (ctrl('b'), page),
            (ctrl('f'), 2 * page),
            (plain(KeyCode::PageUp), page),
            (plain(KeyCode::End), last),
            (plain(KeyCode::Char('j')), last),
            (plain(KeyCode::PageDown), last),
            (plain(KeyCode::Home), 0),
            (plain(KeyCode::PageUp), 0),
            (KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT), last),
            (plain(KeyCode::Char('g')), 0),
        ];
        for (key, expected) in steps {
            assert!(press(&mut app, key).is_empty(), "{key:?}");
            assert_eq!(help(&app).scroll, expected, "{key:?}");
        }

        // A taller terminal fits more lines, so the last line comes up.
        code(&mut app, KeyCode::End);
        app.update(Msg::Resize(80, 24), now());
        assert_eq!(help(&app).scroll, lines(76).len() - 22);
        app.update(Msg::Resize(200, 200), now());
        assert_eq!(help(&app).scroll, 0);
    }

    #[test]
    fn the_help_page_returns_to_where_it_was_opened() {
        for back in [
            KeyCode::Left,
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Char('h'),
            KeyCode::Char('?'),
        ] {
            let mut app = listing(Vec::new());
            code(&mut app, KeyCode::Char('?'));
            chars(&mut app, "jj");
            assert_eq!(help(&app).scroll, 2);
            code(&mut app, back);
            assert!(
                matches!(app.stack.as_slice(), [Screen::Nodes(_), Screen::Topics(_)]),
                "{back:?}"
            );
            // Opened again, it starts at the top.
            code(&mut app, KeyCode::Char('?'));
            assert_eq!(help(&app).scroll, 0);
        }
    }

    #[test]
    fn the_help_page_draws_the_sections_inside_the_frame() {
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('?'));
        let buf = draw(&app, 80, 24);
        assert_framed(&buf);
        assert!(row(&buf, 0).contains("说明"), "{}", row(&buf, 0));
        assert!(row(&buf, 1).contains("所有画面"), "{}", row(&buf, 1));
        let ctrl_c = row(&buf, 3);
        assert!(
            ctrl_c.contains("Ctrl-C") && ctrl_c.contains("退出"),
            "{ctrl_c}"
        );
        let bottom = row(&buf, 23);
        assert!(
            bottom.contains("← 返回") && bottom.contains('%'),
            "{bottom}"
        );

        code(&mut app, KeyCode::End);
        let buf = draw(&app, 80, 24);
        assert!(row(&buf, 22).contains("返回"), "{}", row(&buf, 22));
        assert!(row(&buf, 23).contains("100%"), "{}", row(&buf, 23));

        for (w, h) in [(40, 8), (41, 9), (200, 60)] {
            app.update(Msg::Resize(w, h), now());
            assert_framed(&draw(&app, w, h));
        }
    }
}
