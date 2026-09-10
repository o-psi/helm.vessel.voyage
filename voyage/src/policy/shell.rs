//! Bounded syntax inspection, not an OS sandbox or an interpreter for programs.
//! Parse the exact source that will execute; never repair or expand it here.
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser};

pub(super) struct Analysis {
    pub words: Vec<String>,
    pub simple: bool,
}
const MAX_BYTES: usize = 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 128;

pub(super) fn analyze(source: &str) -> Result<Analysis, String> {
    let mut analysis = Analysis {
        words: Vec::new(),
        simple: true,
    };
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut remaining = MAX_NODES;
    parse(source, &mut analysis, &mut remaining, 0, deadline)?;
    Ok(analysis)
}
fn parse(
    source: &str,
    result: &mut Analysis,
    remaining: &mut usize,
    depth: usize,
    deadline: Instant,
) -> Result<(), String> {
    if source.trim().is_empty() || source.len() > MAX_BYTES || depth > 8 {
        return Err("empty script or shell analysis byte/nesting limit exceeded".into());
    }
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .map_err(|_| "shell parser unavailable")?;
    let mut expired = |_: &tree_sitter::ParseState| Instant::now() >= deadline;
    let tree = parser
        .parse_with_options(
            &mut |offset, _| &source.as_bytes()[offset..],
            None,
            Some(ParseOptions::new().progress_callback(&mut expired)),
        )
        .ok_or("shell analysis time limit exceeded")?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("invalid or unsupported shell syntax (including incomplete heredoc)".into());
    }
    if root.named_child_count() != 1 || root.named_child(0).is_none_or(|n| n.kind() != "command") {
        result.simple = false;
    }
    visit(root, source, result, remaining, depth, 0, deadline)
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn literal(node: Node<'_>, source: &str) -> Option<String> {
    // Expansion is not quote removal. Only statically known shell words qualify.
    match node.kind() {
        "word" | "raw_string" | "string_content" | "number" => (),
        "command_name" | "concatenation" | "string" => {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|n| literal(n, source).is_none())
            {
                return None;
            }
        }
        _ => return None,
    }
    let raw = text(node, source);
    if node.kind() == "word" && raw.contains(['*', '?', '[', '~']) {
        return None;
    }
    let words = shell_words::split(raw).ok()?;
    (words.len() == 1).then(|| words[0].clone())
}
fn visit(
    node: Node<'_>,
    source: &str,
    result: &mut Analysis,
    remaining: &mut usize,
    parse_depth: usize,
    depth: usize,
    deadline: Instant,
) -> Result<(), String> {
    if depth > MAX_DEPTH || *remaining == 0 || Instant::now() >= deadline {
        return Err("shell analysis node/depth/time limit exceeded".into());
    }
    *remaining -= 1;
    if node.is_missing() || node.is_error() {
        return Err("incomplete shell syntax".into());
    }
    match node.kind() {
        "program"
        | "command"
        | "command_name"
        | "number"
        | "word"
        | "string"
        | "raw_string"
        | "string_content"
        | "concatenation"
        | "list"
        | "pipeline"
        | "subshell"
        | "compound_statement"
        | "redirected_statement"
        | "file_redirect"
        | "file_descriptor"
        | "heredoc_redirect"
        | "heredoc_body"
        | "heredoc_start"
        | "heredoc_end"
        | "heredoc_content"
        | "command_substitution"
        | "simple_expansion"
        | "expansion"
        | "variable_name"
        | "special_variable_name"
        | "variable_assignment"
        | "comment"
        | "if_statement"
        | "elif_clause"
        | "else_clause"
        | "for_statement"
        | "while_statement"
        | "do_group"
        | "negated_command"
        | "test_command"
        | "unary_expression"
        | "binary_expression"
        | "test_operator" => (),
        kind => {
            return Err(format!(
                "unsupported shell construct: {kind}; use a simpler explicit command"
            ));
        }
    }
    if node.kind() == "test_command" {
        result.simple = false;
        if !text(node, source).starts_with('[') || text(node, source).starts_with("[[") {
            return Err("unsupported non-POSIX test/arithmetic command".into());
        }
        result.words.push("[".into());
    }
    if !result.simple
        && matches!(
            node.kind(),
            "word" | "raw_string" | "string" | "concatenation"
        )
        && let Some(word) = literal(node, source)
    {
        // Also covers argv attached to heredoc redirects and test expressions.
        // Literal heredoc bodies are distinct nodes and are never scanned here.
        result.words.push(word);
    }
    if node.kind() == "expansion" {
        result.simple = false;
        let mut cursor = node.walk();
        if node
            .children_by_field_name("operator", &mut cursor)
            .any(|op| {
                !matches!(
                    text(op, source),
                    "-" | ":-" | "+" | ":+" | "=" | ":=" | "?" | ":?" | "#" | "##" | "%" | "%%"
                )
            })
        {
            return Err("unsupported parameter expansion operator".into());
        }
    }
    if node.kind() == "heredoc_redirect" {
        result.simple = false;
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        let start = children
            .iter()
            .find(|n| n.kind() == "heredoc_start")
            .ok_or("missing heredoc delimiter")?;
        let end = children
            .iter()
            .find(|n| n.kind() == "heredoc_end")
            .ok_or("unterminated heredoc")?;
        let delimiter = shell_words::split(text(*start, source))
            .map_err(|_| "unsupported heredoc delimiter")?;
        if delimiter.len() != 1 || text(*end, source) != delimiter[0] {
            return Err("unterminated or unsupported heredoc delimiter".into());
        }
        let mut cursor = node.walk();
        if node
            .children_by_field_name("argument", &mut cursor)
            .any(|arg| literal(arg, source).is_none())
        {
            return Err("dynamic arguments after a heredoc delimiter cannot be assessed safely; put explicit arguments before the redirect".into());
        }
        // Literal heredoc contents have no executable meaning; the grammar also
        // separates substitutions in unquoted bodies, which are visited below.
    }
    if node.kind() == "command" {
        let name = node
            .child_by_field_name("name")
            .ok_or("missing shell command name")?;
        let executable = literal(name, source)
            .ok_or("dynamic shell command name cannot be assessed; use a literal executable")?;
        let mut argv = vec![Some(executable.clone())];
        let mut cursor = node.walk();
        for argument in node.children_by_field_name("argument", &mut cursor) {
            argv.push(literal(argument, source));
        }
        result.words.extend(argv.iter().filter_map(Clone::clone));
        if argv.iter().any(Option::is_none) {
            result.simple = false;
        }
        let basename = Path::new(&executable)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if matches!(basename, "eval" | "." | "source") {
            return Err("eval/source cannot be assessed safely; use explicit commands".into());
        }
        // Forwarding dynamic arguments can move data into executable position.
        if matches!(
            basename,
            "env" | "sudo" | "command" | "exec" | "timeout" | "nice" | "xargs"
        ) && argv.iter().any(Option::is_none)
        {
            return Err("dynamic command forwarding cannot be assessed safely".into());
        }
        // Inspect literal nested shell -c scripts, including static forwarding.
        for (index, arg) in argv.iter().enumerate() {
            let Some(arg) = arg else { continue };
            let base = Path::new(arg)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !matches!(base, "sh" | "bash" | "dash" | "zsh" | "ksh") {
                continue;
            }
            if index != 0
                && !matches!(
                    basename,
                    "env" | "sudo" | "command" | "exec" | "timeout" | "nice" | "xargs"
                )
            {
                continue;
            }
            result.simple = false;
            let flag = argv.get(index + 1).and_then(Option::as_deref).unwrap_or("");
            if !flag.starts_with('-')
                || !flag[1..].contains('c')
                || !flag[1..].chars().all(|c| "celuxv".contains(c))
            {
                return Err("shell script/stdin execution cannot be assessed; supply a literal shell -c script or explicit commands".into());
            }
            let script = argv
                .get(index + 2)
                .and_then(Option::as_deref)
                .ok_or("dynamic shell -c script cannot be assessed")?;
            parse(script, result, remaining, parse_depth + 1, deadline)?;
        }
    }
    if matches!(
        node.kind(),
        "variable_assignment" | "file_redirect" | "simple_expansion" | "command_substitution"
    ) {
        result.simple = false;
    }
    if matches!(node.kind(), "pipeline" | "file_redirect" | "list") {
        let mut cursor = node.walk();
        if node
            .children(&mut cursor)
            .any(|n| !n.is_named() && matches!(text(n, source), "|&" | "&>" | "&>>" | ";&" | ";;&"))
        {
            return Err("unsupported non-POSIX shell operator".into());
        }
    }
    // Do not treat data words as command source. Their parent argv is already
    // checked; descend only to find actual executable substitutions.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(
            child,
            source,
            result,
            remaining,
            parse_depth,
            depth + 1,
            deadline,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn syntax_and_literal_payload() {
        for source in [
            "cat <<'EOF'\nwe're \" literal $(shutdown)\nEOF\n",
            "cat <<-EOF\n\thello\n\tEOF\nprintf done",
            "printf '%s' \"$(printf inner)\"",
            "true; printf after",
            "sh -lc 'printf nested'",
        ] {
            let result = analyze(source).unwrap_or_else(|e| panic!("{source:?}: {e}"));
            assert!(!result.simple);
            assert!(!result.words.iter().any(|s| s == "shutdown"));
        }
        assert!(analyze("pwd").unwrap().simple);
        assert!(analyze("[ -f file ] && printf ok").is_ok());
        assert!(analyze("printf '%s' \"${value:-default}\"").is_ok());
        for source in [
            "cat <<EOF\nmissing",
            "echo '",
            "$program arg",
            "sh <<'EOF'\nshutdown\nEOF\n",
            "eval 'printf hello'",
            "env $program",
            "env <<'EOF' $program\ndata\nEOF\n",
        ] {
            assert!(analyze(source).is_err(), "{source:?}");
        }
    }
    #[test]
    fn substitutions_and_post_heredoc_commands_are_inspected() {
        for source in [
            "cat <<EOF\n$(shutdown)\nEOF\n",
            "cat <<'EOF'\nliteral\nEOF\nshutdown",
            "sh -c 'shutdown'",
            "s\"hut\"down",
            "true|shutdown",
            "env <<'EOF' shutdown\ndata\nEOF\n",
            "echo \"${value:-$(shutdown)}\"",
        ] {
            assert!(
                analyze(source)
                    .unwrap()
                    .words
                    .iter()
                    .any(|s| s == "shutdown"),
                "{source:?}"
            );
        }
    }
}

#[cfg(test)]
mod policy_tests {
    use super::super::{Decision, Policy};
    use crate::{Config, config::AccessMode};
    #[test]
    fn retired_command_denials_do_not_override_access_modes() {
        let root = tempfile::tempdir().unwrap();
        for access in [
            AccessMode::Unrestricted,
            AccessMode::Approval,
            AccessMode::ReadOnly,
        ] {
            let config = Config {
                access: Some(access),
                legacy_deny_commands: vec!["printf".into()],
                ..Default::default()
            };
            let policy = Policy::new(&config, root.path().to_path_buf()).unwrap();
            for source in [
                "shutdown",
                "reboot",
                "mkfs",
                "printf shutdown",
                "printf denied",
                "/usr/bin/printf denied",
                "p\"rint\"f denied",
                "true; printf denied",
                "cat <<EOF\n$(printf denied)\nEOF\n",
                "cat <<'EOF'\ndata\nEOF\nprintf denied",
                "sh -c 'printf denied'",
                "echo $(printf denied)",
            ] {
                assert!(
                    match access {
                        AccessMode::Unrestricted =>
                            matches!(policy.command(source), Decision::Allow),
                        AccessMode::Approval =>
                            !matches!(policy.command(source), Decision::Deny(_)),
                        AccessMode::ReadOnly => matches!(policy.command(source), Decision::Deny(_)),
                    },
                    "{access:?} {source:?}"
                );
            }
            assert_eq!(
                policy.command("pwd"),
                match access {
                    AccessMode::ReadOnly =>
                        Decision::Deny("commands are disabled in read-only access mode".into()),
                    _ => Decision::Allow,
                }
            );
            let heredoc =
                policy.command("cat <<'EOF'\nwe're literal printf $(printf ignored)\nEOF\n");
            assert!(match access {
                AccessMode::Unrestricted => matches!(heredoc, Decision::Allow),
                AccessMode::Approval => matches!(heredoc, Decision::Ask(_)),
                AccessMode::ReadOnly => matches!(heredoc, Decision::Deny(_)),
            });
        }
    }
    #[test]
    fn bounded_and_incomplete_input_refuses() {
        assert!(super::analyze(&"a".repeat(super::MAX_BYTES + 1)).is_err());
        for source in [
            "cat <<EOF\nunterminated",
            "cat <<'EOF'\nunterminated",
            "echo $((1+2))",
            "echo <(printf unsupported)",
            "sh --rcfile harmless script",
            "true |& printf unsupported",
        ] {
            assert!(super::analyze(source).is_err(), "{source:?}");
        }
    }
}
