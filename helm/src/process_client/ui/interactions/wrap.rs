use ratatui::text::{Line, Text};

pub(super) fn wrap_lines(text: Text<'_>, width: u16) -> Vec<Line<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut lines = Vec::new();
    for line in text.lines {
        let content: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let mut output = String::new();
        let mut used = 0;
        for grapheme in content.graphemes(true) {
            let size = grapheme.width();
            if used + size > width.max(1) as usize && !output.is_empty() {
                lines.push(Line::from(std::mem::take(&mut output)));
                used = 0;
            }
            output.push_str(grapheme);
            used += size;
        }
        lines.push(Line::from(output));
    }
    lines
}
