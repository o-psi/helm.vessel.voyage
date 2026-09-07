use super::{
    Options, Report, files,
    layout::{Journal, Layout},
    links::{publish_links, validate},
    release::Manifest,
};
use anyhow::{Context, Result, ensure};
use std::{fs, os::unix::fs::symlink};
pub fn install(options: Options) -> Result<Report> {
    let l = Layout::get()?;
    let manifest = Manifest::inspect(&options.bin_dir)?;
    let id = manifest.id()?;
    if options.dry_run {
        let j = l.journal()?;
        validate(&l, &j, options.replace_existing)?;
        return l.report(&id, &manifest.version, &j);
    }
    files::private_directory(&l.root)?;
    let _lock = files::lock(&l.root.join("lock"))?;
    let mut j = l.journal()?;
    recover(&l, &mut j)?;
    validate(&l, &j, options.replace_existing)?;
    let report = l.report(&id, &manifest.version, &j)?;
    files::directory(&l.root.join("releases"))?;
    manifest.stage(&options.bin_dir, &l.release(&id))?;
    j.pending = Some(id.clone());
    l.save(&j)?;
    publish_links(&l, options.replace_existing)?;
    publish(&l, &mut j, &id)?;
    Ok(report)
}
pub fn rollback(dry_run: bool) -> Result<Report> {
    let l = Layout::get()?;
    if dry_run {
        let j = l.journal()?;
        let id = j
            .previous
            .as_deref()
            .context("No previous release available")?;
        let manifest = l.verify(id)?;
        validate(&l, &j, false)?;
        return l.report(id, &manifest.version, &j);
    }
    files::private_directory(&l.root)?;
    let _lock = files::lock(&l.root.join("lock"))?;
    let mut j = l.journal()?;
    recover(&l, &mut j)?;
    validate(&l, &j, false)?;
    let id = j
        .previous
        .clone()
        .context("No previous release available")?;
    let manifest = l.verify(&id)?;
    let report = l.report(&id, &manifest.version, &j)?;
    j.pending = Some(id.clone());
    l.save(&j)?;
    publish(&l, &mut j, &id)?;
    Ok(report)
}
fn publish(l: &Layout, j: &mut Journal, id: &str) -> Result<()> {
    l.verify(id)?;
    ensure!(
        l.pointer()? == j.current || l.pointer()?.as_deref() == Some(id),
        "Current pointer changed during transaction"
    );
    let temp = l.root.join("next");
    if temp.exists() || fs::symlink_metadata(&temp).is_ok() {
        ensure!(
            fs::read_link(&temp)? == l.release(id),
            "Unrecognized pending link"
        );
    } else {
        symlink(l.release(id), &temp)?;
    }
    let current = l.root.join("current");
    if fs::symlink_metadata(&current).is_ok() {
        let expected = j.current.as_ref().map(|v| l.release(v));
        files::exchange(&temp, &current)?;
        let displaced = fs::read_link(&temp)?;
        ensure!(
            Some(displaced) == expected || fs::read_link(&temp)? == l.release(id),
            "Concurrent current pointer preserved at next; installation paused"
        );
        fs::remove_file(&temp)?;
    } else {
        fs::hard_link(&temp, &current)?;
        fs::remove_file(&temp)?;
    }
    files::sync(&l.root.join("current"))?;
    if j.current.as_deref() != Some(id) {
        j.previous = j.current.take();
        j.current = Some(id.into());
    }
    j.pending = None;
    l.save(j)
}
fn recover(l: &Layout, j: &mut Journal) -> Result<()> {
    if let Some(id) = j.pending.clone() {
        l.verify(&id)?;
        if l.pointer()?.as_deref() == Some(&id) {
            let next = l.root.join("next");
            if fs::symlink_metadata(&next).is_ok() {
                ensure!(
                    j.current
                        .as_ref()
                        .is_some_and(|old| fs::read_link(&next).ok() == Some(l.release(old))),
                    "Unrecognized interrupted pointer retained at next"
                );
                fs::remove_file(&next)?;
                files::sync(&next)?;
            }
            if j.current.as_deref() != Some(&id) {
                j.previous = j.current.take();
                j.current = Some(id);
            }
            j.pending = None;
            l.save(j)?;
        } else {
            ensure!(
                l.pointer()? == j.current,
                "Unrecognized partial installation pointer"
            );
            j.pending = None;
            l.save(j)?;
        }
    }
    Ok(())
}
