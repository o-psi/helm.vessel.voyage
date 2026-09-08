use std::path::PathBuf;

/// Parse only explicit paths, never prose, URLs to fetch, or shell expressions.
/// This is lexical: no filesystem or desktop access is performed.
pub(super) fn parse(text: &str) -> Option<Vec<PathBuf>> {
    if text.len() > super::TEXT || text.contains('\0') {
        return None;
    }
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut paths = Vec::new();
    // URI lists and Windows paths are line-oriented; backslashes are literal.
    if text.lines().all(|line| {
        let line = line.trim().trim_matches('"');
        line.starts_with("file:") || windows_drive(line) || line.starts_with(r"\\")
    }) {
        for line in text.lines() {
            paths.push(one(line.trim().trim_matches('"'))?);
        }
        return Some(paths);
    }
    let mut word = String::new();
    let mut quote = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if q == c => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (Some('\''), _) => word.push(c),
            (_, '\\') => {
                let next = chars.next()?;
                if !matches!(
                    next,
                    ' ' | '\t' | '\\' | '\'' | '"' | '(' | ')' | '[' | ']' | '&' | ';' | '$' | '`'
                ) {
                    return None;
                }
                word.push(next);
            }
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() {
                    paths.push(one(&word)?);
                    word.clear();
                }
            }
            (None, ';' | '|' | '&' | '<' | '>' | '`' | '$') => return None,
            _ => word.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    if !word.is_empty() {
        paths.push(one(&word)?);
    }
    if paths.is_empty() { None } else { Some(paths) }
}

fn windows_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/')
}
fn one(s: &str) -> Option<PathBuf> {
    if s.chars().any(char::is_control) || (s.contains("://") && !s.starts_with("file:")) {
        return None;
    }
    if s.starts_with("file:") {
        let url = reqwest::Url::parse(s).ok()?;
        if url.scheme() != "file"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        if url
            .host_str()
            .is_some_and(|h| !h.eq_ignore_ascii_case("localhost"))
        {
            return None;
        }
        let path = url.to_file_path().ok()?;
        let value = path.to_str()?;
        if value.chars().any(char::is_control) {
            return None;
        }
        #[cfg(not(windows))]
        if value.starts_with('/') && windows_drive(&value[1..]) {
            return one(&value[1..]);
        }
        return Some(path);
    }
    if s.starts_with(r"\\") {
        #[cfg(windows)]
        {
            return Some(PathBuf::from(s));
        }
        #[cfg(not(windows))]
        {
            return None;
        } // Never guess a remote UNC mount on Unix.
    }
    if windows_drive(s) {
        #[cfg(target_os = "linux")]
        {
            return Some(PathBuf::from(format!(
                "/mnt/{}/{}",
                s[..1].to_ascii_lowercase(),
                s[3..].replace('\\', "/")
            )));
        }
        #[cfg(not(target_os = "linux"))]
        {
            return Some(PathBuf::from(s));
        }
    }
    if let Some(tail) = s.strip_prefix("~/") {
        return dirs::home_dir().map(|home| home.join(tail));
    }
    if s.starts_with('/') || s.starts_with("./") || s.starts_with("../") {
        return Some(PathBuf::from(s));
    }
    // One relative raster filename/path is a candidate, not arbitrary prose.
    // The caller checks existence, real type and regular-file safety before attaching.
    if PathBuf::from(s)
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| {
            matches!(
                x.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
    {
        return Some(PathBuf::from(s));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_paths_only() {
        for text in [
            "hello world",
            "https://example.com/a.png",
            "see /tmp/a.png",
            "/tmp/a.png;whoami",
            "file://remote/a",
            "file:///tmp/a%00b",
            "'unfinished",
        ] {
            assert!(parse(text).is_none(), "{text}");
        }
        assert_eq!(
            parse("'/tmp/a b.png' /tmp/c\\ d.jpg").unwrap(),
            vec![PathBuf::from("/tmp/a b.png"), PathBuf::from("/tmp/c d.jpg")]
        );
        assert_eq!(
            parse("file:///tmp/a%20b.png\r\nfile:///tmp/c.png")
                .unwrap()
                .len(),
            2
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn windows_to_wsl() {
        assert_eq!(
            parse("C:\\Users\\A B\\x.png").unwrap(),
            vec![PathBuf::from("/mnt/c/Users/A B/x.png")]
        );
        assert_eq!(
            parse("file:///C:/Users/a.png").unwrap(),
            vec![PathBuf::from("/mnt/c/Users/a.png")]
        );
    }
}

#[cfg(test)]
mod raster_path_tests {
    use super::*;
    #[test]
    fn relative_rasters_are_candidates_but_protocols_and_controls_are_not() {
        assert_eq!(
            parse("photo.png").unwrap(),
            vec![PathBuf::from("photo.png")]
        );
        assert_eq!(
            parse("assets/photo.jpeg").unwrap(),
            vec![PathBuf::from("assets/photo.jpeg")]
        );
        for text in [
            "https://example.com/a.png",
            "ftp://example.com/a.png",
            "/tmp/a\u{1b}.png",
            "file:///tmp/a%1bb.png",
            "explain photo.png",
        ] {
            assert!(parse(text).is_none(), "{text:?}");
        }
    }
}
