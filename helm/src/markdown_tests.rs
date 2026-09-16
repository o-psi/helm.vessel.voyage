use super::*;

fn plain(text: &Text<'_>) -> String {
    text.lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn commonmark_and_gfm_are_readable_without_terminal_controls() {
    let source = "# Heading\n\n## Second\n\n### Third\n\n#### Fourth\n\n##### Fifth\n\n###### Sixth\n\n> quoted **bold** and *italic*\n>\n> continuation\n\n1. ordered\n2. next\n   - nested\n\n- [x] done\n- [ ] pending\n\n~~deleted~~ `inline` [label](https://example.invalid \"title\")\n\n![alt](https://example.invalid/image)\n\n---\n\n| left | right |\n|:---|---:|\n| alpha | beta |\n| longer cell | value |\n\nfootnote[^n]\n\n[^n]: explanation\n\n<div>inert</div>\n\nhard  \nbreak\n";
    for width in [1, 8, 20, 80] {
        let text = render_markdown(
            source,
            RenderOptions {
                width,
                ..Default::default()
            },
        );
        let output = plain(&text);
        assert!(!output.contains('\x1b'));
        assert!(!text.lines.is_empty());
        if width == 80 {
            for expected in [
                "Heading",
                "quoted",
                "ordered",
                "done",
                "pending",
                "inline",
                "label",
                "alt",
                "alpha",
                "beta",
                "explanation",
                "inert",
            ] {
                assert!(output.contains(expected), "missing {expected}: {output}");
            }
        }
    }
}

#[test]
fn code_blocks_handle_known_unknown_and_missing_languages() {
    for language in ["rust", "python", "unknown-language", ""] {
        for highlighting in [true, false] {
            let source = format!("```{language}\nlet value = 42;\n\tvalue\n```\n\n    indented\n");
            let text = render_markdown(
                &source,
                RenderOptions {
                    syntax_highlighting: highlighting,
                    ..Default::default()
                },
            );
            let output = plain(&text);
            assert!(output.contains("let value = 42;"));
            assert!(output.contains("indented"));
            assert!(!output.contains('\t'));
        }
    }
}

#[test]
fn budgets_preserve_unicode_and_report_truncation() {
    assert_eq!(bounded_source("é界", 1), ("", true));
    assert_eq!(bounded_source("é界", 2), ("é", true));
    assert_eq!(bounded_source("é界", 5), ("é界", false));
    for (bytes, lines) in [(0, 0), (1, 1), (4, 3), (4096, 2)] {
        let text = render_markdown(
            "é界\n\nsecond\n\nthird\n\nfourth",
            RenderOptions {
                width: 0,
                max_input_bytes: bytes,
                max_output_lines: lines,
                ..Default::default()
            },
        );
        assert!(text.lines.len() <= lines.max(1));
        assert!(plain(&text).contains("truncated"));
    }
    assert_eq!(render_plain("", 80), "");
}

#[test]
fn hostile_controls_and_incomplete_streaming_markup_are_inert() {
    assert_eq!(
        sanitize("a\t\x1b\0\u{7f}\u{85}\u{202e}\u{2066}\nb"),
        "a    [U+001B][U+0000][U+007F][U+0085][U+202E][U+2066]\nb"
    );
    for source in [
        "**unfinished",
        "[label](",
        "```rust\nlet x",
        "| a | b |\n|---|---|\n|x|",
        "<script>bad()</script>",
        "a\x1b[2J\u{202e}b",
    ] {
        let output = render_plain(source, 80);
        assert!(!output.contains('\x1b'));
        assert!(!output.contains('\u{202e}'));
    }
}

#[test]
fn wrapping_retains_styles_and_handles_wide_graphemes() {
    let style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let text = Text::from(vec![
        Line::from(vec![Span::styled("alpha beta 界 e\u{301}", style)]),
        Line::from(""),
    ]);
    let wrapped = wrap_text(text, 7);
    assert!(wrapped.lines.len() > 2);
    assert!(
        wrapped
            .lines
            .iter()
            .flat_map(|l| &l.spans)
            .filter(|s| !s.content.trim().is_empty())
            .all(|s| s.style == style)
    );
    let fragments = vec![StyledText {
        text: "longword\t界\n next".into(),
        style,
    }];
    for preserve in [true, false] {
        let (lines, _) = wrap_fragments(&fragments, 3, preserve, 100, 0);
        assert!(!lines.is_empty());
    }
    let (_, truncated) = wrap_fragments(&fragments, 1, false, 1, 0);
    assert!(truncated);
}
