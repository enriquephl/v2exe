use v2exe::render::{Line, LineKind, Role, Seg, display_width, render_html, wrap};

fn texts(l: &[Line]) -> Vec<String> {
    l.iter().map(Line::text).collect()
}

#[test]
fn paragraphs_and_breaks() {
    assert_eq!(
        texts(&render_html("<p>第一段</p><p>第二段<br>第二行</p>")),
        vec!["第一段", "", "第二段", "第二行"]
    );
}

#[test]
fn pre_keeps_indentation() {
    let l = render_html("<pre><code>fn main() {\n    x();\n}</code></pre>");
    assert_eq!(texts(&l), vec!["fn main() {", "    x();", "}"]);
    assert!(l.iter().all(|l| l.kind == LineKind::Code));
}

#[test]
fn mention_absorbs_at_sign() {
    let l = render_html(r#"@<a href="/member/alice">alice</a> 同意"#);
    assert_eq!(
        l[0].segs,
        vec![
            Seg {
                text: "@alice".into(),
                role: Role::Mention
            },
            Seg {
                text: " 同意".into(),
                role: Role::Plain
            }
        ]
    );
}

#[test]
fn links() {
    assert_eq!(
        render_html(r#"<a href="https://a.example/x">https://a.example/x</a>"#)[0].segs,
        vec![Seg {
            text: "https://a.example/x".into(),
            role: Role::Link
        }]
    );
    assert_eq!(
        render_html(r#"<a href="https://a.example/x">文档</a>"#)[0].segs,
        vec![Seg {
            text: "文档 (https://a.example/x)".into(),
            role: Role::Link
        }]
    );
}

#[test]
fn image_becomes_placeholder_line() {
    assert_eq!(
        render_html(r#"<img src="https://i.example/a.png">"#)[0].segs,
        vec![Seg {
            text: "[图片] https://i.example/a.png".into(),
            role: Role::Image
        }]
    );
}

#[test]
fn entities_lists_and_quotes() {
    assert_eq!(texts(&render_html("<p>a &amp; b</p>")), vec!["a & b"]);
    assert_eq!(
        texts(&render_html("<ul><li>一</li><li>二</li></ul>")),
        vec!["• 一", "• 二"]
    );
    assert_eq!(
        render_html("<blockquote>引用</blockquote>")[0].kind,
        LineKind::Quote
    );
}

#[test]
fn display_width_counts_wide_chars() {
    assert_eq!(display_width("中a🤖"), 5);
}

#[test]
fn wrap_cjk() {
    assert_eq!(
        texts(&wrap(&render_html("一二三四五"), 6)),
        vec!["一二三", "四五"]
    );
}

#[test]
fn wrap_prefers_spaces() {
    assert_eq!(
        texts(&wrap(&render_html("hello world foo"), 11)),
        vec!["hello world", "foo"]
    );
}

#[test]
fn wrap_hard_breaks_long_tokens() {
    let url = format!("https://example.com/{}", "a".repeat(100));
    let out = wrap(&render_html(&url), 30);
    assert!(out.iter().all(|l| display_width(&l.text()) <= 30));
    assert_eq!(out.iter().map(Line::text).collect::<String>(), url);
}

#[test]
fn wrap_keeps_roles_and_kind() {
    let out = wrap(&render_html("<pre>abcdefghij</pre>"), 4);
    assert!(
        out.len() == 3
            && out
                .iter()
                .all(|l| l.kind == LineKind::Code && l.segs[0].role == Role::Code)
    );
}

#[test]
fn wrap_splits_segments_across_lines() {
    let out = wrap(&render_html("ab <b>cd ef</b>"), 5);
    assert_eq!(
        out.iter().map(|l| l.segs.clone()).collect::<Vec<_>>(),
        vec![
            vec![
                Seg {
                    text: "ab ".into(),
                    role: Role::Plain
                },
                Seg {
                    text: "cd".into(),
                    role: Role::Bold
                }
            ],
            vec![Seg {
                text: "ef".into(),
                role: Role::Bold
            }]
        ]
    );
}

#[test]
fn ordered_lists_number_items_and_nested_lists_stay_tight() {
    assert_eq!(
        texts(&render_html("<ol><li>甲</li><li>乙</li></ol>")),
        vec!["1. 甲", "2. 乙"]
    );
    assert_eq!(
        texts(&render_html(
            "<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>"
        )),
        vec!["• a", "• b", "• c"]
    );
}

#[test]
fn blank_lines_collapse_and_edges_are_trimmed() {
    assert_eq!(
        texts(&render_html("<br><p></p><p>a</p><br><br><p>b</p><br>")),
        vec!["a", "", "b"]
    );
}

#[test]
fn whitespace_collapses_and_lines_are_trimmed() {
    assert_eq!(texts(&render_html("<p>  a \n  b  </p>")), vec!["a b"]);
    assert_eq!(
        texts(&render_html("第一行<br />\n第二行<br />\n")),
        vec!["第一行", "第二行"]
    );
}

#[test]
fn headings_bold_and_inline_code_have_roles() {
    let l = render_html("<h1>标题</h1><p>x <b>粗</b> <code>c  d</code></p>");
    assert_eq!(
        l[0].segs,
        vec![Seg {
            text: "标题".into(),
            role: Role::Bold
        }]
    );
    let roles: Vec<_> = l[2].segs.iter().map(|s| s.role.clone()).collect();
    assert_eq!(
        roles,
        vec![Role::Plain, Role::Bold, Role::Plain, Role::Code]
    );
    assert_eq!(l[2].text(), "x 粗 c d");
}

#[test]
fn blockquote_paragraphs_stay_quoted() {
    let l = render_html("<p>前</p><blockquote><p>a</p><p>b</p></blockquote><p>后</p>");
    assert_eq!(texts(&l), vec!["前", "", "a", "", "b", "", "后"]);
    let kinds: Vec<_> = l.iter().map(|l| l.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            LineKind::Text,
            LineKind::Text,
            LineKind::Quote,
            LineKind::Quote,
            LineKind::Quote,
            LineKind::Text,
            LineKind::Text
        ]
    );
}

#[test]
fn link_wrapping_an_image_renders_the_image() {
    let l = render_html(
        r#"<a href="https://i.example/a.png" rel="nofollow"><img src="https://i.example/a.png" class="embedded_image"></a>"#,
    );
    assert_eq!(
        l[0].segs,
        vec![Seg {
            text: "[图片] https://i.example/a.png".into(),
            role: Role::Image
        }]
    );
}
