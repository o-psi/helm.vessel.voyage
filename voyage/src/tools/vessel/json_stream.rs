//! Incremental public-JSON decoding and value-string redaction. Raw pending text stays in private cursor state.
use super::{ToolError, failed};
use crate::tools::Redactor;

const SECRET_BYTES: usize = 64 * 1024;
const BLOCK: usize = 4096;

#[derive(Clone)]
struct Strings {
    secrets: Vec<String>,
    hold: usize,
    pending: String,
    masked_prefix: usize,
    redacting: bool,
}
impl Strings {
    fn new(redactor: &Redactor) -> Result<Self, ToolError> {
        let bytes = redactor.secrets.iter().map(String::len).sum::<usize>();
        if bytes > SECRET_BYTES {
            return Err(failed(
                "configured secrets exceed the streaming redaction budget",
            ));
        }
        Ok(Self {
            secrets: redactor.secrets.clone(),
            hold: redactor.secrets.iter().map(String::len).max().unwrap_or(1) - 1,
            pending: String::new(),
            masked_prefix: 0,
            redacting: false,
        })
    }
    fn emit(&mut self, flush: bool) -> String {
        let mut cut = if flush {
            self.pending.len()
        } else {
            self.pending.len().saturating_sub(self.hold)
        };
        while !self.pending.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == 0 {
            return String::new();
        }
        // One endpoint per source byte bounds memory even for overlapping secrets.
        let mut ends = vec![0; self.pending.len() + 1];
        ends[0] = self.masked_prefix;
        for secret in &self.secrets {
            let mut from = 0;
            while let Some(relative) = self.pending[from..].find(secret) {
                let start = from + relative;
                ends[start] = ends[start].max(start + secret.len());
                from = start + self.pending[start..].chars().next().unwrap().len_utf8();
            }
        }
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in ends.into_iter().enumerate().filter(|(s, e)| e > s) {
            if let Some(last) = merged.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let mut out = String::new();
        let mut offset = 0;
        self.masked_prefix = 0;
        for (start, end) in merged {
            if start >= cut {
                break;
            }
            if start > offset {
                out.push_str(&self.pending[offset..start]);
                self.redacting = false;
            }
            if !self.redacting {
                out.push_str("[REDACTED]");
            }
            self.redacting = true;
            offset = end.min(cut);
            if end > cut {
                self.masked_prefix = end - cut;
            }
        }
        if offset < cut {
            out.push_str(&self.pending[offset..cut]);
            self.redacting = false;
        }
        self.pending.drain(..cut);
        if flush {
            self.redacting = false;
            self.masked_prefix = 0;
        }
        out
    }
}

#[derive(Clone, Copy)]
enum Frame {
    ObjectKey(bool),
    ObjectColon,
    ObjectValue,
    ObjectComma,
    ArrayValue(bool),
    ArrayComma,
}
#[derive(Clone)]
pub(super) struct Stream {
    json: bool,
    strings: Strings,
    stack: Vec<Frame>,
    root_done: bool,
    in_string: bool,
    key: bool,
    escape: String,
    token: String,
    pub(super) ready: String,
    pub(super) done: bool,
}
impl Stream {
    pub(super) fn new(json: bool, redactor: &Redactor) -> Result<Self, ToolError> {
        Ok(Self {
            json,
            strings: Strings::new(redactor)?,
            stack: Vec::new(),
            root_done: false,
            in_string: false,
            key: false,
            escape: String::new(),
            token: String::new(),
            ready: String::new(),
            done: false,
        })
    }
    fn invalid() -> ToolError {
        failed("invalid streamed public message JSON")
    }
    fn value_expected(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Frame::ObjectValue | Frame::ArrayValue(_))
        ) || (self.stack.is_empty() && !self.root_done)
    }
    fn finish_value(&mut self) -> Result<(), ToolError> {
        match self.stack.last_mut() {
            Some(frame @ Frame::ObjectValue) => *frame = Frame::ObjectComma,
            Some(frame @ Frame::ArrayValue(_)) => *frame = Frame::ArrayComma,
            None if !self.root_done => self.root_done = true,
            _ => return Err(Self::invalid()),
        }
        Ok(())
    }
    fn emit_string(&mut self, flush: bool) -> Result<(), ToolError> {
        let value = if self.key {
            std::mem::take(&mut self.strings.pending)
        } else {
            self.strings.emit(flush)
        };
        if self.json {
            let encoded = serde_json::to_string(&value).map_err(|_| Self::invalid())?;
            self.ready.push_str(&encoded[1..encoded.len() - 1]);
        } else {
            self.ready.push_str(&value);
        }
        Ok(())
    }
    fn decoded(&mut self, c: char) -> Result<(), ToolError> {
        self.strings.pending.push(c);
        if self.strings.pending.len() >= self.strings.hold + BLOCK {
            self.emit_string(false)?;
        }
        Ok(())
    }
    fn escaped(&mut self, c: char) -> Result<(), ToolError> {
        self.escape.push(c);
        if self.escape.len() == 2 {
            let decoded = match c {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'u' => return Ok(()),
                _ => return Err(Self::invalid()),
            };
            self.escape.clear();
            return self.decoded(decoded);
        }
        if !self.escape.is_ascii() || self.escape.len() > 12 {
            return Err(Self::invalid());
        }
        let bytes = self.escape.as_bytes();
        if self.escape.len() <= 6 && !c.is_ascii_hexdigit() {
            return Err(Self::invalid());
        }
        if self.escape.len() == 6 {
            let n = u16::from_str_radix(&self.escape[2..6], 16).map_err(|_| Self::invalid())?;
            if (0xd800..=0xdbff).contains(&n) {
                return Ok(());
            }
            let c = char::from_u32(n as u32).ok_or_else(Self::invalid)?;
            self.escape.clear();
            return self.decoded(c);
        }
        if self.escape.len() > 6 {
            if (bytes.len() == 7 && c != '\\')
                || (bytes.len() == 8 && c != 'u')
                || (bytes.len() > 8 && !c.is_ascii_hexdigit())
            {
                return Err(Self::invalid());
            }
            if bytes.len() == 12 {
                let high =
                    u16::from_str_radix(&self.escape[2..6], 16).map_err(|_| Self::invalid())?;
                let low =
                    u16::from_str_radix(&self.escape[8..12], 16).map_err(|_| Self::invalid())?;
                if !(0xdc00..=0xdfff).contains(&low) {
                    return Err(Self::invalid());
                }
                let c = char::from_u32(
                    0x10000 + ((high as u32 - 0xd800) << 10) + (low as u32 - 0xdc00),
                )
                .ok_or_else(Self::invalid)?;
                self.escape.clear();
                return self.decoded(c);
            }
        }
        Ok(())
    }
    fn finish_token(&mut self) -> Result<(), ToolError> {
        if self.token.is_empty() {
            return Ok(());
        }
        let value: serde_json::Value =
            serde_json::from_str(&self.token).map_err(|_| Self::invalid())?;
        if !matches!(
            value,
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_)
        ) {
            return Err(Self::invalid());
        }
        self.finish_value()?;
        self.ready.push_str(&self.token);
        self.token.clear();
        Ok(())
    }
    pub(super) fn push(&mut self, c: char) -> Result<(), ToolError> {
        if self.done {
            return Err(Self::invalid());
        }
        if !self.json {
            return self.decoded(c);
        }
        if self.in_string {
            if !self.escape.is_empty() {
                return self.escaped(c);
            }
            match c {
                '\\' => self.escape.push(c),
                '"' => {
                    self.emit_string(true)?;
                    self.ready.push('"');
                    self.in_string = false;
                    if self.key {
                        let frame = self.stack.last_mut().ok_or_else(Self::invalid)?;
                        if !matches!(frame, Frame::ObjectKey(_)) {
                            return Err(Self::invalid());
                        }
                        *frame = Frame::ObjectColon;
                    } else {
                        self.finish_value()?;
                    }
                }
                c if c < '\u{20}' => return Err(Self::invalid()),
                c => self.decoded(c)?,
            }
            return Ok(());
        }
        let delimiter = c.is_ascii_whitespace() || matches!(c, ',' | ']' | '}');
        if !self.token.is_empty() {
            if delimiter {
                self.finish_token()?;
            } else {
                if !c.is_ascii() || self.token.len() >= 128 {
                    return Err(Self::invalid());
                }
                self.token.push(c);
                return Ok(());
            }
        }
        match c {
            '"' => {
                self.key = matches!(self.stack.last(), Some(Frame::ObjectKey(_)));
                if !self.key && !self.value_expected() {
                    return Err(Self::invalid());
                }
                self.in_string = true;
                self.ready.push(c);
            }
            '{' | '[' => {
                if !self.value_expected() || self.stack.len() >= 128 {
                    return Err(Self::invalid());
                }
                self.stack.push(if c == '{' {
                    Frame::ObjectKey(true)
                } else {
                    Frame::ArrayValue(true)
                });
                self.ready.push(c);
            }
            '}' => {
                if !matches!(
                    self.stack.last(),
                    Some(Frame::ObjectKey(true) | Frame::ObjectComma)
                ) {
                    return Err(Self::invalid());
                }
                self.stack.pop();
                self.finish_value()?;
                self.ready.push(c);
            }
            ']' => {
                if !matches!(
                    self.stack.last(),
                    Some(Frame::ArrayValue(true) | Frame::ArrayComma)
                ) {
                    return Err(Self::invalid());
                }
                self.stack.pop();
                self.finish_value()?;
                self.ready.push(c);
            }
            ':' => {
                let frame = self.stack.last_mut().ok_or_else(Self::invalid)?;
                if !matches!(frame, Frame::ObjectColon) {
                    return Err(Self::invalid());
                }
                *frame = Frame::ObjectValue;
                self.ready.push(c);
            }
            ',' => {
                let frame = self.stack.last_mut().ok_or_else(Self::invalid)?;
                *frame = match frame {
                    Frame::ObjectComma => Frame::ObjectKey(false),
                    Frame::ArrayComma => Frame::ArrayValue(false),
                    _ => return Err(Self::invalid()),
                };
                self.ready.push(c);
            }
            ' ' | '\n' | '\r' | '\t' => self.ready.push(c),
            '-' | '0'..='9' | 't' | 'f' | 'n' => {
                if !self.value_expected() {
                    return Err(Self::invalid());
                }
                self.token.push(c);
            }
            _ => return Err(Self::invalid()),
        }
        Ok(())
    }
    pub(super) fn finish(&mut self) -> Result<(), ToolError> {
        if self.json {
            self.finish_token()?;
            if self.in_string || !self.stack.is_empty() || !self.root_done {
                return Err(Self::invalid());
            }
        } else {
            self.emit_string(true)?;
        }
        self.done = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(source: &str, json: bool, secrets: &[&str]) -> String {
        let redactor = Redactor::new(secrets.iter().map(|s| s.to_string()));
        let mut state = Stream::new(json, &redactor).unwrap();
        let mut out = String::new();
        for c in source.chars() {
            state.push(c).unwrap();
            out.push_str(&std::mem::take(&mut state.ready));
            assert!(state.strings.pending.len() < SECRET_BYTES + BLOCK + 4);
        }
        state.finish().unwrap();
        out.push_str(&state.ready);
        out
    }
    #[test]
    fn json_strings_preserve_structure_and_decode_escapes() {
        let raw = r#"{"null":null,"values":[true,17,-2.5,"private\"secret","\uD83E\uDD80","null"],"nested":{}}"#;
        let out = stream(raw, true, &["private\"secret", "null"]);
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(parsed["null"].is_null());
        assert_eq!(parsed["values"][3], "[REDACTED]");
        assert_eq!(parsed["values"][4], "🦀");
        assert_eq!(parsed["values"][5], "[REDACTED]");
    }
    #[test]
    fn large_strings_and_overlapping_boundary_secrets_are_bounded() {
        let source = format!(
            "{}secret-🦀-value{}",
            "x".repeat(4090),
            "y".repeat(5 * 1024 * 1024)
        );
        let out = stream(&source, false, &["secret-🦀-value"]);
        assert_eq!(
            out,
            format!(
                "{}[REDACTED]{}",
                "x".repeat(4090),
                "y".repeat(5 * 1024 * 1024)
            )
        );
        let chain = "ab".repeat(20000);
        assert_eq!(stream(&chain, false, &["ababa", "babab"]), "[REDACTED]");
    }
    #[test]
    fn malformed_json_is_refused() {
        for source in [
            r#"{"x":}"#,
            "[1,]",
            r#""\uD800x""#,
            r#""\uDC00""#,
            "{}{}",
            "tru",
            r#""unfinished"#,
        ] {
            let mut state = Stream::new(true, &Redactor::default()).unwrap();
            let result = source
                .chars()
                .try_for_each(|c| state.push(c))
                .and_then(|_| state.finish());
            assert!(result.is_err(), "{source}");
        }
    }
}
