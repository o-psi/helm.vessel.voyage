use super::layout::Layout;
use anyhow::Result;
pub(super) fn inspect() -> Result<Vec<String>> {
    let layout = Layout::get()?;
    let journal = layout.journal()?;
    let mut lines = Vec::new();
    for (label, id) in [
        ("Current", journal.current.as_ref()),
        ("Previous", journal.previous.as_ref()),
    ] {
        if let Some(id) = id {
            let manifest = layout.verify(id)?;
            lines.push(format!("{label}: {} ({id})", manifest.version));
            lines.push(format!(
                "  {} — hashes and executable permissions verified",
                layout.release(id).join("bin").display()
            ));
        } else {
            lines.push(format!("{label}: not installed"));
        }
    }
    lines.push(format!(
        "Pending transaction: {}",
        journal.pending.as_deref().unwrap_or("none")
    ));
    let pointer = layout.pointer()?;
    anyhow::ensure!(
        pointer == journal.current || pointer == journal.pending,
        "Current pointer does not match installation journal"
    );
    lines.push(format!(
        "Published pointer: {}",
        pointer.as_deref().unwrap_or("none")
    ));
    lines.push(format!("Command directory: {}", layout.bin.display()));
    Ok(lines)
}
