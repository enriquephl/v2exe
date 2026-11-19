//! Topic and reply HTML rendered into styled lines for terminal output.
//!
//! Roles are semantic; callers map them to colors.

use scraper::{ElementRef, Html, Node};
use unicode_width::UnicodeWidthChar;

/// Semantic style of a run of text.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    Plain,
    Bold,
    Link,
    Mention,
    Code,
    Image,
}

/// What a line belongs to.
#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
    Text,
    Code,
    Quote,
}

/// A run of text with one role.
#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    pub text: String,
    pub role: Role,
}

/// One terminal line. A blank line has no segments.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub kind: LineKind,
    pub segs: Vec<Seg>,
}

impl Line {
    /// The segments' text joined together.
    pub fn text(&self) -> String {
        self.segs.iter().map(|s| s.text.as_str()).collect()
    }

    fn is_blank(&self) -> bool {
        self.segs.is_empty() && self.kind != LineKind::Code
    }
}

/// Display width in terminal columns: CJK characters and emoji count 2,
/// combining and control characters 0.
pub fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Drops control characters (C0, DEL, C1) and expands tabs to the next multiple
/// of 4 columns, counted from the start of `s`.
pub fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let n = 4 - col % 4;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else if !c.is_control() {
            out.push(c);
            col += char_width(c);
        }
    }
    out
}

/// Elements nested deeper than this are not walked one by one; their text is kept as plain
/// inline text, so the walk's recursion stays shallow whatever the HTML.
const MAX_DEPTH: usize = 64;

/// Renders an HTML fragment into logical lines, without wrapping.
pub fn render_html(html: &str) -> Vec<Line> {
    let doc = Html::parse_fragment(html);
    let mut r = Renderer::new();
    r.children(doc.root_element());
    r.finish()
}

/// Wraps lines to at most `width.max(2)` columns.
///
/// Breaks at the last space (dropped), between two CJK characters, or hard at
/// the limit when neither exists. Every piece keeps the line's kind and the
/// roles of its segments.
pub fn wrap(lines: &[Line], width: usize) -> Vec<Line> {
    let width = width.max(2);
    let mut out = Vec::new();
    for line in lines {
        if display_width(&line.text()) <= width {
            out.push(line.clone());
        } else {
            wrap_line(line, width, &mut out);
        }
    }
    out
}

fn wrap_line(line: &Line, width: usize, out: &mut Vec<Line>) {
    let cells: Vec<(char, usize)> = line
        .segs
        .iter()
        .enumerate()
        .flat_map(|(i, s)| s.text.chars().map(move |c| (c, i)))
        .collect();
    let cells_width =
        |cells: &[(char, usize)]| -> usize { cells.iter().map(|(c, _)| char_width(*c)).sum() };
    // Width of everything from `start` on.
    let mut rest = cells_width(&cells);
    let mut start = 0;
    while start < cells.len() {
        if rest <= width {
            out.push(piece(line, &cells[start..]));
            return;
        }
        let mut w = 0;
        let mut brk: Option<(usize, usize)> = None;
        let mut i = start;
        let (end, next) = loop {
            let c = cells[i].0;
            if i > start {
                if c == ' ' {
                    brk = Some((i, i + 1));
                } else if char_width(c) == 2 && char_width(cells[i - 1].0) == 2 {
                    brk = Some((i, i));
                }
            }
            let cw = char_width(c);
            if w + cw > width {
                break brk.unwrap_or((i, i));
            }
            w += cw;
            i += 1;
        };
        out.push(piece(line, &cells[start..end]));
        rest -= cells_width(&cells[start..next]);
        start = next;
    }
}

fn piece(line: &Line, cells: &[(char, usize)]) -> Line {
    let mut segs: Vec<Seg> = Vec::new();
    let mut last = usize::MAX;
    for &(c, idx) in cells {
        if idx != last {
            segs.push(Seg {
                text: String::new(),
                role: line.segs[idx].role.clone(),
            });
            last = idx;
        }
        if let Some(seg) = segs.last_mut() {
            seg.text.push(c);
        }
    }
    Line {
        kind: line.kind.clone(),
        segs,
    }
}

enum List {
    Bullet,
    Ordered(usize),
}

struct Renderer {
    lines: Vec<Line>,
    cur: Vec<Seg>,
    /// True at line start and after a space; collapses runs of whitespace.
    at_space: bool,
    /// A blank line is owed before the next content.
    gap: bool,
    /// List marker for the next line of the open `<li>`.
    marker: Option<String>,
    bold: usize,
    quote: usize,
    lists: Vec<List>,
    /// How many elements deep the walk is.
    depth: usize,
}

impl Renderer {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            cur: Vec::new(),
            at_space: true,
            gap: false,
            marker: None,
            bold: 0,
            quote: 0,
            lists: Vec::new(),
            depth: 0,
        }
    }

    fn finish(mut self) -> Vec<Line> {
        self.flush();
        while self.lines.last().is_some_and(Line::is_blank) {
            self.lines.pop();
        }
        self.lines
    }

    fn text_kind(&self) -> LineKind {
        if self.quote > 0 {
            LineKind::Quote
        } else {
            LineKind::Text
        }
    }

    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| !l.is_blank()) {
            let kind = self.text_kind();
            self.lines.push(Line {
                kind,
                segs: Vec::new(),
            });
        }
    }

    /// Ends the current line, dropping trailing whitespace.
    fn flush(&mut self) {
        while let Some(last) = self.cur.last_mut() {
            let kept = last.text.trim_end().len();
            last.text.truncate(kept);
            if last.text.is_empty() {
                self.cur.pop();
            } else {
                break;
            }
        }
        if !self.cur.is_empty() {
            let kind = self.text_kind();
            let segs = std::mem::take(&mut self.cur);
            self.lines.push(Line { kind, segs });
        }
        self.at_space = true;
    }

    fn open_block(&mut self) {
        self.flush();
        self.blank();
    }

    fn close_block(&mut self) {
        self.flush();
        self.gap = true;
    }

    fn line_break(&mut self) {
        if self.cur.is_empty() {
            self.blank();
        } else {
            self.flush();
        }
    }

    fn start_line(&mut self) {
        if self.gap {
            self.gap = false;
            self.blank();
        }
        if let Some(marker) = self.marker.take() {
            self.cur.push(Seg {
                text: marker,
                role: Role::Plain,
            });
        }
    }

    fn push_seg(&mut self, text: String, role: Role) {
        if self.cur.is_empty() {
            self.start_line();
        }
        self.cur.push(Seg { text, role });
        self.at_space = false;
    }

    /// Adds text with whitespace runs collapsed to one space and other control
    /// characters dropped.
    fn push_text(&mut self, raw: &str, role: Role) {
        let mut text = String::new();
        let mut at_space = self.at_space;
        for c in raw.chars() {
            if c.is_whitespace() {
                if !at_space {
                    text.push(' ');
                }
                at_space = true;
            } else if !c.is_control() {
                text.push(c);
                at_space = false;
            }
        }
        if !text.is_empty() {
            self.push_seg(text, role);
            self.at_space = at_space;
        }
    }

    fn push_line(&mut self, kind: LineKind, segs: Vec<Seg>) {
        self.flush();
        if self.gap {
            self.gap = false;
            self.blank();
        }
        self.lines.push(Line { kind, segs });
    }

    fn children(&mut self, el: ElementRef) {
        for child in el.children() {
            match child.value() {
                Node::Text(t) => {
                    let role = self.text_role();
                    self.push_text(t, role);
                }
                Node::Element(_) => {
                    if let Some(e) = ElementRef::wrap(child) {
                        self.element(e);
                    }
                }
                _ => {}
            }
        }
    }

    fn element(&mut self, el: ElementRef) {
        if self.depth == MAX_DEPTH {
            let role = self.text_role();
            for text in el.text() {
                self.push_text(text, role.clone());
            }
            return;
        }
        self.depth += 1;
        self.dispatch(el);
        self.depth -= 1;
    }

    fn text_role(&self) -> Role {
        if self.bold > 0 {
            Role::Bold
        } else {
            Role::Plain
        }
    }

    fn dispatch(&mut self, el: ElementRef) {
        match el.value().name() {
            "p" | "div" => {
                self.open_block();
                self.children(el);
                self.close_block();
            }
            "br" => self.line_break(),
            "pre" => self.pre(el),
            "blockquote" => {
                self.open_block();
                self.quote += 1;
                self.children(el);
                self.flush();
                self.quote -= 1;
                self.close_block();
            }
            "ul" => self.list(el, List::Bullet),
            "ol" => self.list(el, List::Ordered(0)),
            "li" => self.list_item(el),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.open_block();
                self.bold += 1;
                self.children(el);
                self.bold -= 1;
                self.close_block();
            }
            "strong" | "b" => {
                self.bold += 1;
                self.children(el);
                self.bold -= 1;
            }
            "code" => {
                let text = collapse(&el.text().collect::<String>());
                if !text.is_empty() {
                    self.push_seg(text, Role::Code);
                }
            }
            "a" => self.link(el),
            "img" => {
                let src = sanitize(el.value().attr("src").unwrap_or("").trim());
                if !src.is_empty() {
                    let kind = self.text_kind();
                    self.push_line(
                        kind,
                        vec![Seg {
                            text: format!("[图片] {src}"),
                            role: Role::Image,
                        }],
                    );
                }
            }
            _ => self.children(el),
        }
    }

    fn pre(&mut self, el: ElementRef) {
        self.open_block();
        let source = pre_text(el);
        let source = source.strip_suffix('\n').unwrap_or(&source);
        for line in source.split('\n') {
            let line = sanitize(line);
            let segs = if line.is_empty() {
                Vec::new()
            } else {
                vec![Seg {
                    text: line,
                    role: Role::Code,
                }]
            };
            self.push_line(LineKind::Code, segs);
        }
        self.close_block();
    }

    fn list(&mut self, el: ElementRef, list: List) {
        let nested = !self.lists.is_empty();
        if nested {
            self.flush();
        } else {
            self.open_block();
        }
        self.lists.push(list);
        self.children(el);
        self.lists.pop();
        if nested {
            self.flush();
        } else {
            self.close_block();
        }
    }

    fn list_item(&mut self, el: ElementRef) {
        self.flush();
        self.marker = Some(match self.lists.last_mut() {
            Some(List::Ordered(n)) => {
                *n += 1;
                format!("{n}. ")
            }
            _ => "• ".to_string(),
        });
        self.children(el);
        self.flush();
        self.marker = None;
    }

    fn link(&mut self, el: ElementRef) {
        let href = sanitize(el.value().attr("href").unwrap_or("").trim());
        let wraps_image = el
            .descendants()
            .any(|n| matches!(n.value(), Node::Element(e) if e.name() == "img"));
        if href.is_empty() || wraps_image {
            self.children(el);
            return;
        }
        let label = collapse(&el.text().collect::<String>());
        let label = if label.is_empty() { &href } else { &label };
        if href.starts_with("/member/") {
            let mut text = String::new();
            if let Some(last) = self.cur.last_mut()
                && last.text.ends_with('@')
            {
                last.text.pop();
                if last.text.is_empty() {
                    self.cur.pop();
                }
                text.push('@');
            }
            text.push_str(label);
            self.push_seg(text, Role::Mention);
        } else if *label == href {
            self.push_seg(href, Role::Link);
        } else {
            self.push_seg(format!("{label} ({href})"), Role::Link);
        }
    }
}

/// Trims, collapses whitespace runs to single spaces and drops control
/// characters.
fn collapse(s: &str) -> String {
    sanitize(&s.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Source text of a `<pre>`, with `<br>` as a newline.
fn pre_text(el: ElementRef) -> String {
    let mut out = String::new();
    for node in el.descendants() {
        match node.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) if e.name() == "br" => out.push('\n'),
            _ => {}
        }
    }
    out
}
