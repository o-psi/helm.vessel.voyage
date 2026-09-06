//! Preserve the parsed CLI contract without reparsing global Helm arguments.
use crate::github::{
    admin,
    operator::{Args, Command, ViewArgs},
};
use anyhow::Result;
use clap::ValueEnum;

pub(super) fn words(args: Args) -> Result<Vec<String>> {
    let mut out = Vec::new();
    match args.command {
        Command::Admin { command } => {
            out.push("admin".into());
            match command {
                admin::Command::List { offset } => {
                    out.extend(["list".into(), "--offset".into(), offset.to_string()])
                }
                admin::Command::Inspect { id } => out.extend(["inspect".into(), id.to_string()]),
                admin::Command::Cancel { id, digest } => {
                    out.extend(["cancel".into(), id.to_string(), digest])
                }
                admin::Command::Dispose { id, digest, note } => {
                    out.extend(["dispose".into(), id.to_string(), digest, note])
                }
                admin::Command::Forget { id, digest } => {
                    out.extend(["forget".into(), id.to_string(), digest])
                }
                admin::Command::Audit => out.push("audit".into()),
                admin::Command::ClearAudit { digest } => out.extend(["clear-audit".into(), digest]),
            }
        }
        Command::Auth => out.push("auth".into()),
        Command::Remotes => out.push("remotes".into()),
        Command::References => out.push("references".into()),
        Command::Logs { url, job } => out.extend(["logs".into(), url, job.to_string()]),
        Command::Continue { request } => out.extend(["continue".into(), request]),
        Command::Reference { url } => out.extend(["reference".into(), url]),
        Command::Unreference { url } => out.extend(["unreference".into(), url]),
        Command::View(args) => {
            out.push("view".into());
            view(&mut out, args);
        }
        Command::Feedback { view: args, id } => {
            out.push("feedback".into());
            view(&mut out, args);
            if let Some(id) = id {
                out.extend(["--id".into(), id.to_string()]);
            }
        }
        Command::Prepare(args) => {
            out.push("prepare".into());
            if let Some(url) = args.url {
                out.push(url);
            }
            for (flag, value) in [
                ("--body", args.body),
                ("--event", args.event),
                ("--commit", args.commit),
            ] {
                if let Some(value) = value {
                    out.extend([flag.into(), value]);
                }
            }
            for (flag, value) in [
                ("--body-file", args.body_file),
                ("--draft-file", args.draft_file),
            ] {
                if let Some(value) = value {
                    out.extend([
                        flag.into(),
                        value
                            .canonicalize()?
                            .to_str()
                            .ok_or_else(|| anyhow::anyhow!("GitHub body path is not UTF-8"))?
                            .into(),
                    ]);
                }
            }
        }
        Command::Publish { id, digest } => out.extend(["publish".into(), id.to_string(), digest]),
        Command::Inspect { id } => out.extend(["inspect".into(), id.to_string()]),
        Command::List { offset } => {
            out.extend(["list".into(), "--offset".into(), offset.to_string()])
        }
        Command::Cancel { id, digest } => out.extend(["cancel".into(), id.to_string(), digest]),
        Command::Forget { id, digest } => out.extend(["forget".into(), id.to_string(), digest]),
        Command::Reconcile {
            id,
            digest,
            remote_id,
        } => out.extend([
            "reconcile".into(),
            id.to_string(),
            digest,
            remote_id.to_string(),
        ]),
        Command::Dispose { id, digest, note } => {
            out.extend(["dispose".into(), id.to_string(), digest, note])
        }
    }
    Ok(out)
}
fn view(out: &mut Vec<String>, args: ViewArgs) {
    out.push(args.url);
    out.extend([
        "--section".into(),
        args.section
            .to_possible_value()
            .expect("known section")
            .get_name()
            .into(),
        "--page".into(),
        args.page.to_string(),
    ]);
    for (flag, value) in [
        ("--after", args.after),
        ("--thread", args.thread),
        ("--head", args.head),
        ("--resource", args.resource.map(|v| v.to_string())),
    ] {
        if let Some(value) = value {
            out.extend([flag.into(), value]);
        }
    }
}
