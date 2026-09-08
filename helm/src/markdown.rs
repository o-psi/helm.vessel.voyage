//! Safe, width-aware CommonMark/GFM rendering for terminal frontends.
//!
//! The renderer deliberately returns owned ratatui text. It never changes the
//! canonical Markdown supplied by the model and never places terminal control
//! sequences in its output.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use std::sync::OnceLock;
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const DEFAULT_MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_MAX_OUTPUT_LINES: usize = 20_000;
const MAX_NESTING: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkdownTheme {
    pub text: Color,
    pub heading: Color,
    pub link: Color,
    pub code: Color,
    pub code_background: Color,
    pub quote: Color,
    pub rule: Color,
    pub table_header: Color,
    pub warning: Color,
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        Self {
            text: crate::theme::Role::Primary
                .style()
                .fg
                .unwrap_or(Color::Reset),
            heading: crate::theme::Role::Focus.style().fg.unwrap_or(Color::Reset),
            link: Color::Blue,
            code: crate::theme::Role::Code.style().fg.unwrap_or(Color::Reset),
            code_background: crate::theme::Role::Code.style().bg.unwrap_or(Color::Reset),
            quote: Color::DarkGray,
            rule: Color::DarkGray,
            table_header: Color::Magenta,
            warning: Color::Yellow,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderOptions {
    /// Display columns available to the Markdown body. Zero is treated as one.
    pub width: usize,
    pub theme: MarkdownTheme,
    pub max_input_bytes: usize,
    pub max_output_lines: usize,
    /// Apply language-aware syntax colors to recognized fenced code blocks.
    /// Disable this for monochrome or `NO_COLOR` frontends.
    pub syntax_highlighting: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 80,
            theme: MarkdownTheme::default(),
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_output_lines: DEFAULT_MAX_OUTPUT_LINES,
            syntax_highlighting: true,
        }
    }
}

#[derive(Clone, Debug)]
struct StyledText {
    text: String,
    style: Style,
}

#[derive(Default)]
struct TableState {
    rows: Vec<Vec<Vec<StyledText>>>,
    row: Vec<Vec<StyledText>>,
    cell: Vec<StyledText>,
    header_rows: usize,
    in_head: bool,
}

struct Renderer {
    options: RenderOptions,
    lines: Vec<Line<'static>>,
    current: Vec<StyledText>,
    style_stack: Vec<Style>,
    list_stack: Vec<Option<u64>>,
    quote_depth: usize,
    code_block: bool,
    code_language: Option<String>,
    code_buffer: String,
    link_stack: Vec<(String, String)>,
    table: Option<TableState>,
    truncated: bool,
}

/// Render CommonMark plus the GFM table, task-list and strikethrough extensions.
///
/// Malformed/incomplete streaming input is rendered best-effort. This function
/// does not return an error and applies explicit resource bounds. Raw HTML is
/// displayed as inert text; images use a `[image: alt] (URL)` textual fallback
/// because a terminal transcript cannot embed or safely interpret either.
pub fn render_markdown(source: &str, mut options: RenderOptions) -> Text<'static> {
    options.width = options.width.max(1);
    options.max_input_bytes = options.max_input_bytes.max(1);
    options.max_output_lines = options.max_output_lines.max(1);
    let (input, input_truncated) = bounded_source(source, options.max_input_bytes);
    let mut renderer = Renderer::new(options);
    let parser_options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES;
    for event in Parser::new_ext(input, parser_options) {
        if renderer.lines.len() >= renderer.options.max_output_lines {
            renderer.truncated = true;
            break;
        }
        renderer.event(event);
    }
    renderer.finish(input_truncated)
}

/// Render Markdown to safe, unstyled plain text for redirected/non-TTY output.
pub fn render_plain(source: &str, width: usize) -> String {
    let rendered = render_markdown(
        source,
        RenderOptions {
            width,
            syntax_highlighting: false,
            ..RenderOptions::default()
        },
    );
    rendered
        .lines
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

fn bounded_source(source: &str, maximum: usize) -> (&str, bool) {
    if source.len() <= maximum {
        return (source, false);
    }
    let mut end = maximum;
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    (&source[..end], true)
}

impl Renderer {
    fn new(options: RenderOptions) -> Self {
        Self {
            options,
            lines: Vec::new(),
            current: Vec::new(),
            style_stack: vec![Style::default().fg(options.theme.text)],
            list_stack: Vec::new(),
            quote_depth: 0,
            code_block: false,
            code_language: None,
            code_buffer: String::new(),
            link_stack: Vec::new(),
            table: None,
            truncated: false,
        }
    }

    fn style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, addition: Style) {
        if self.style_stack.len() < MAX_NESTING {
            self.style_stack.push(self.style().patch(addition));
        }
    }

    fn pop_style(&mut self) {
        if self.style_stack.len() > 1 {
            self.style_stack.pop();
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.add_text(&text),
            Event::Code(text) => {
                self.push_style(
                    Style::default()
                        .fg(self.options.theme.code)
                        .bg(self.options.theme.code_background),
                );
                self.add_text(&text);
                self.pop_style();
            }
            Event::Html(html) | Event::InlineHtml(html) => self.add_text(&html),
            Event::SoftBreak => self.add_text(if self.code_block { "\n" } else { " " }),
            Event::HardBreak => self.flush_line(false),
            Event::Rule => {
                self.flush_line(false);
                let rule = "─".repeat(self.options.width.min(72));
                self.current.push(StyledText {
                    text: rule,
                    style: Style::default().fg(self.options.theme.rule),
                });
                self.flush_line(false);
            }
            Event::TaskListMarker(checked) => self.add_text(if checked { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(name) => self.add_text(&format!("[^{name}]")),
            Event::InlineMath(math) => self.add_text(&math),
            Event::DisplayMath(math) => {
                self.flush_line(false);
                self.add_text(&math);
                self.flush_line(false);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_line(false);
                let level = heading_number(level);
                let style = Style::default()
                    .fg(self.options.theme.heading)
                    .add_modifier(if level <= 2 {
                        Modifier::BOLD
                    } else {
                        Modifier::ITALIC
                    });
                self.push_style(style);
            }
            Tag::BlockQuote(_) => {
                self.flush_line(false);
                self.quote_depth = (self.quote_depth + 1).min(MAX_NESTING);
            }
            Tag::CodeBlock(kind) => {
                self.flush_line(false);
                self.code_block = true;
                self.code_language = match kind {
                    CodeBlockKind::Fenced(language) if !language.is_empty() => {
                        Some(sanitize(&language))
                    }
                    _ => None,
                };
                if let Some(language) = self.code_language.clone() {
                    self.current.push(StyledText {
                        text: language,
                        style: Style::default()
                            .fg(self.options.theme.quote)
                            .add_modifier(Modifier::ITALIC),
                    });
                    self.flush_line(false);
                }
                self.push_style(
                    Style::default()
                        .fg(self.options.theme.code)
                        .bg(self.options.theme.code_background),
                );
            }
            Tag::List(start) => {
                self.flush_line(false);
                if self.list_stack.len() < MAX_NESTING {
                    self.list_stack.push(start);
                }
            }
            Tag::Item => {
                self.flush_line(false);
                let indent = "  ".repeat(self.list_stack.len().saturating_sub(1));
                let marker = match self.list_stack.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number = number.saturating_add(1);
                        marker
                    }
                    _ => "• ".to_owned(),
                };
                self.add_text(&format!("{indent}{marker}"));
            }
            Tag::Emphasis => self.push_style(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => {
                self.push_style(Style::default().add_modifier(Modifier::CROSSED_OUT))
            }
            Tag::Link { dest_url, .. } => {
                self.link_stack.push((sanitize(&dest_url), String::new()));
                self.push_style(
                    Style::default()
                        .fg(self.options.theme.link)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Image { dest_url, .. } => {
                self.add_text("[image: ");
                self.link_stack.push((sanitize(&dest_url), String::new()));
            }
            Tag::Table(_) => {
                self.flush_line(false);
                self.table = Some(TableState::default());
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_head = true;
                }
            }
            Tag::TableRow => {}
            Tag::TableCell => {}
            Tag::FootnoteDefinition(name) => {
                self.flush_line(false);
                self.add_text(&format!("[^{name}]: "));
            }
            Tag::HtmlBlock
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
            Tag::MetadataBlock(_) => {}
            Tag::Superscript => self.push_style(Style::default()),
            Tag::Subscript => self.push_style(Style::default()),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_line(false);
                if self.list_stack.is_empty() {
                    self.blank_line();
                }
            }
            TagEnd::Heading(_) => {
                self.pop_style();
                self.flush_line(false);
                self.blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_line(false);
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                self.pop_style();
                self.flush_code_block();
                self.code_block = false;
                self.code_language = None;
                self.blank_line();
            }
            TagEnd::List(_) => {
                self.flush_line(false);
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.blank_line();
                }
            }
            TagEnd::Item => self.flush_line(false),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some((url, label)) = self.link_stack.pop()
                    && !url.is_empty()
                    && label.trim() != url
                {
                    self.add_styled(
                        format!(" ({url})"),
                        Style::default().fg(self.options.theme.link),
                    );
                }
            }
            TagEnd::Image => {
                if let Some((url, _)) = self.link_stack.pop() {
                    self.add_text(&format!("] ({url})"));
                } else {
                    self.add_text("]");
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    table.row.push(std::mem::take(&mut table.cell));
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    table.rows.push(std::mem::take(&mut table.row));
                    if table.in_head {
                        table.header_rows += 1;
                    }
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    // pulldown-cmark models the header as TableHead -> TableCell,
                    // without a surrounding TableRow event.
                    if !table.row.is_empty() {
                        table.rows.push(std::mem::take(&mut table.row));
                        table.header_rows += 1;
                    }
                    table.in_head = false;
                }
            }
            TagEnd::Table => self.flush_table(),
            TagEnd::FootnoteDefinition => self.flush_line(false),
            TagEnd::HtmlBlock
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => {}
            TagEnd::Superscript | TagEnd::Subscript => self.pop_style(),
        }
    }

    fn add_text(&mut self, raw: &str) {
        let safe = sanitize(raw);
        if safe.is_empty() {
            return;
        }
        if self.code_block {
            self.code_buffer.push_str(&safe);
            return;
        }
        if let Some((_, label)) = self.link_stack.last_mut() {
            label.push_str(&safe);
        }
        if let Some(table) = &mut self.table {
            table.cell.push(StyledText {
                text: safe,
                style: self.style_stack.last().copied().unwrap_or_default(),
            });
            return;
        }
        self.current.push(StyledText {
            text: safe,
            style: self.style(),
        });
    }

    fn add_styled(&mut self, text: String, style: Style) {
        if let Some(table) = &mut self.table {
            table.cell.push(StyledText { text, style });
        } else {
            self.current.push(StyledText { text, style });
        }
    }

    fn flush_code_block(&mut self) {
        let code = std::mem::take(&mut self.code_buffer);
        let language = self.code_language.as_deref();
        let highlighted = self.options.syntax_highlighting
            && language.is_some()
            && syntax_assets()
                .0
                .find_syntax_by_token(language.unwrap_or_default())
                .is_some();

        if highlighted {
            let (syntaxes, theme) = syntax_assets();
            let syntax = syntaxes
                .find_syntax_by_token(language.unwrap_or_default())
                .expect("checked above");
            let mut highlighter = HighlightLines::new(syntax, theme);
            for line in LinesWithEndings::from(&code) {
                match highlighter.highlight_line(line, syntaxes) {
                    Ok(regions) => {
                        for (syntax_style, value) in regions {
                            let value = value.strip_suffix('\n').unwrap_or(value);
                            if !value.is_empty() {
                                self.current.push(StyledText {
                                    text: value.to_owned(),
                                    style: syntax_style_to_ratatui(syntax_style),
                                });
                            }
                        }
                    }
                    Err(_) => self.current.push(StyledText {
                        text: line.strip_suffix('\n').unwrap_or(line).to_owned(),
                        style: Style::default()
                            .fg(self.options.theme.code)
                            .bg(self.options.theme.code_background),
                    }),
                }
                self.flush_line(true);
            }
        } else {
            let style = Style::default()
                .fg(self.options.theme.code)
                .bg(self.options.theme.code_background);
            for line in code.split_terminator('\n') {
                self.current.push(StyledText {
                    text: line.to_owned(),
                    style,
                });
                self.flush_line(true);
            }
            if code.is_empty() {
                self.flush_line(true);
            }
        }
    }

    fn blank_line(&mut self) {
        if self.lines.len() < self.options.max_output_lines
            && self.lines.last().is_some_and(|l| !l.spans.is_empty())
        {
            self.lines.push(Line::default());
        }
    }

    fn flush_line(&mut self, preserve_empty: bool) {
        if self.current.is_empty() && !preserve_empty {
            return;
        }
        let mut content = std::mem::take(&mut self.current);
        if self.quote_depth > 0 {
            content.insert(
                0,
                StyledText {
                    text: "│ ".repeat(self.quote_depth.min(8)),
                    style: Style::default().fg(self.options.theme.quote),
                },
            );
        }
        let remaining = self
            .options
            .max_output_lines
            .saturating_sub(self.lines.len())
            .max(1);
        let prefix: String = content.iter().map(|p| p.text.as_str()).collect();
        let trimmed = prefix.trim_start();
        let marker = if trimmed.starts_with("• ") {
            2
        } else {
            trimmed
                .find(". ")
                .filter(|&i| i > 0 && trimmed[..i].chars().all(|c| c.is_ascii_digit()))
                .map_or(0, |i| i + 2)
        };
        let indent = if !self.list_stack.is_empty() && marker > 0 {
            prefix.len() - trimmed.len() + marker
        } else {
            0
        };
        let (wrapped, wrapping_truncated) = wrap_fragments(
            &content,
            self.options.width,
            self.code_block,
            remaining,
            indent,
        );
        self.truncated |= wrapping_truncated;
        for line in wrapped {
            if self.lines.len() >= self.options.max_output_lines {
                self.truncated = true;
                break;
            }
            self.lines.push(Line::from(
                line.into_iter()
                    .map(|part| Span::styled(part.text, part.style))
                    .collect::<Vec<_>>(),
            ));
        }
    }

    fn flush_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let plain = table
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| fragments_text(cell))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let widths = (0..columns)
            .map(|column| {
                plain
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|value| UnicodeWidthStr::width(value.as_str()))
                    .max()
                    .unwrap_or(1)
                    .max(1)
            })
            .collect::<Vec<_>>();
        let grid_width = widths.iter().sum::<usize>() + columns.saturating_sub(1) * 3 + 4;
        if grid_width <= self.options.width {
            for (row_index, row) in plain.iter().enumerate() {
                let mut text = String::from("│ ");
                for (column, column_width) in widths.iter().enumerate() {
                    if column > 0 {
                        text.push_str(" │ ");
                    }
                    let value = row.get(column).map(String::as_str).unwrap_or("");
                    text.push_str(value);
                    text.push_str(
                        &" ".repeat(column_width.saturating_sub(UnicodeWidthStr::width(value))),
                    );
                }
                text.push_str(" │");
                let style = if row_index < table.header_rows {
                    Style::default()
                        .fg(self.options.theme.table_header)
                        .add_modifier(Modifier::BOLD)
                } else {
                    self.style()
                };
                self.current.push(StyledText { text, style });
                self.flush_line(false);
                if row_index + 1 == table.header_rows {
                    let rule = format!("├{}┤", "─".repeat(grid_width.saturating_sub(2)));
                    self.current.push(StyledText {
                        text: rule,
                        style: Style::default().fg(self.options.theme.rule),
                    });
                    self.flush_line(false);
                }
            }
        } else {
            // A narrow viewport gets a deterministic, readable record layout.
            let headers = plain.first().filter(|_| table.header_rows > 0);
            for (row_index, row) in plain.iter().enumerate().skip(table.header_rows) {
                if row_index > table.header_rows {
                    self.flush_line(false);
                }
                for column in 0..columns {
                    let label = headers
                        .and_then(|header| header.get(column))
                        .filter(|header| !header.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| format!("Column {}", column + 1));
                    let value = row.get(column).map(String::as_str).unwrap_or("");
                    self.current.push(StyledText {
                        text: format!("{label}: "),
                        style: Style::default()
                            .fg(self.options.theme.table_header)
                            .add_modifier(Modifier::BOLD),
                    });
                    self.current.push(StyledText {
                        text: value.to_owned(),
                        style: self.style(),
                    });
                    self.flush_line(false);
                }
            }
        }
    }

    fn finish(mut self, input_truncated: bool) -> Text<'static> {
        self.flush_line(false);
        if input_truncated || self.truncated {
            if self.lines.len() >= self.options.max_output_lines {
                self.lines
                    .truncate(self.options.max_output_lines.saturating_sub(1));
            }
            self.lines.push(Line::styled(
                "… Markdown output truncated …",
                Style::default().fg(self.options.theme.warning),
            ));
        }
        Text::from(self.lines)
    }
}

fn syntax_assets() -> &'static (SyntaxSet, Theme) {
    static ASSETS: OnceLock<(SyntaxSet, Theme)> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let mut themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .remove("base16-ocean.dark")
            .or_else(|| themes.themes.into_values().next())
            .unwrap_or_default();
        (syntaxes, theme)
    })
}

fn syntax_style_to_ratatui(value: syntect::highlighting::Style) -> Style {
    let mut modifiers = Modifier::empty();
    if value.font_style.contains(FontStyle::BOLD) {
        modifiers |= Modifier::BOLD;
    }
    if value.font_style.contains(FontStyle::ITALIC) {
        modifiers |= Modifier::ITALIC;
    }
    if value.font_style.contains(FontStyle::UNDERLINE) {
        modifiers |= Modifier::UNDERLINED;
    }
    Style::default()
        .fg(Color::Rgb(
            value.foreground.r,
            value.foreground.g,
            value.foreground.b,
        ))
        .add_modifier(modifiers)
}

fn heading_number(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn fragments_text(fragments: &[StyledText]) -> String {
    fragments
        .iter()
        .map(|fragment| fragment.text.as_str())
        .collect()
}

fn wrap_fragments(
    parts: &[StyledText],
    width: usize,
    preserve_whitespace: bool,
    max_lines: usize,
    indent: usize,
) -> (Vec<Vec<StyledText>>, bool) {
    let width = width.max(1);
    let indent = indent.min(width.saturating_sub(1));
    let mut lines = vec![Vec::<StyledText>::new()];
    let mut used = 0usize;
    let mut truncated = false;
    let mut units = parts
        .iter()
        .flat_map(|part| part.text.graphemes(true).map(move |g| (g, part.style)))
        .peekable();
    let mut continuing_word = false;
    while let Some((g, style)) = units.next() {
        let continuation = continuing_word;
        // Look ahead only one display-width, even for a very large word.
        let mut word = vec![(g, style)];
        let mut word_width = g.width();
        if !preserve_whitespace && !g.chars().all(char::is_whitespace) {
            while word_width <= width
                && word.len() <= width
                && units
                    .peek()
                    .is_some_and(|(next, _)| !next.chars().all(char::is_whitespace))
            {
                let next = units.next().expect("peeked grapheme");
                word_width += next.0.width();
                word.push(next);
            }
        }
        continuing_word = !preserve_whitespace
            && !g.chars().all(char::is_whitespace)
            && units
                .peek()
                .is_some_and(|(next, _)| !next.chars().all(char::is_whitespace));
        if g == "\n"
            || (!preserve_whitespace
                && !continuation
                && used > indent
                && word_width + used > width
                && !g.chars().all(char::is_whitespace))
        {
            if lines.len() >= max_lines {
                truncated = true;
                break;
            }
            let continuation = if preserve_whitespace { 0 } else { indent };
            lines.push(if continuation > 0 {
                vec![StyledText {
                    text: " ".repeat(continuation),
                    style: Style::default(),
                }]
            } else {
                Vec::new()
            });
            used = continuation;
            if g == "\n" {
                continue;
            }
        }
        for &(grapheme, style) in &word {
            let size = grapheme.width();
            if used > 0 && used + size > width {
                if lines.len() >= max_lines {
                    truncated = true;
                    break;
                }
                let continuation = if preserve_whitespace { 0 } else { indent };
                lines.push(if continuation > 0 {
                    vec![StyledText {
                        text: " ".repeat(continuation),
                        style: Style::default(),
                    }]
                } else {
                    Vec::new()
                });
                used = continuation;
            }
            if !preserve_whitespace
                && lines.len() > 1
                && used <= indent
                && grapheme.chars().all(char::is_whitespace)
            {
                continue;
            }
            if let Some(last) = lines.last_mut().and_then(|line| line.last_mut())
                && last.style == style
            {
                last.text.push_str(grapheme);
            } else {
                lines.last_mut().unwrap().push(StyledText {
                    text: grapheme.to_owned(),
                    style,
                });
            }
            used += size;
        }
        if truncated {
            break;
        }
    }
    (lines, truncated)
}

/// Remove terminal controls, C0/C1 controls and invisible bidi formatting.
fn sanitize(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\n' => output.push(character),
            '\t' => output.push_str("    "),
            '\u{0000}'..='\u{001f}'
            | '\u{007f}'..='\u{009f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => {
                use std::fmt::Write;
                let _ = write!(output, "[U+{:04X}]", character as u32);
            }
            _ => output.push(character),
        }
    }
    output
}

/// Word-aware plain text wrapping with original styles, for transcript notices.
pub fn wrap_text(text: Text<'_>, width: usize) -> Text<'static> {
    let mut output = Vec::new();
    for line in text.lines {
        let parts = line
            .spans
            .into_iter()
            .map(|s| StyledText {
                text: s.content.into_owned(),
                style: s.style,
            })
            .collect::<Vec<_>>();
        let (lines, _) = wrap_fragments(&parts, width.max(1), false, DEFAULT_MAX_OUTPUT_LINES, 0);
        output.extend(lines.into_iter().map(|parts| {
            Line::from(
                parts
                    .into_iter()
                    .map(|p| Span::styled(p.text, p.style))
                    .collect::<Vec<_>>(),
            )
            .style(line.style)
        }));
    }
    Text::from(output)
}
