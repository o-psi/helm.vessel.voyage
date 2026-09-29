//! Kernel-observed child identity while the fixed Voyage waits for launch input.
use anyhow::{Context, Result, ensure};
use std::{fs, io::Read, path::Path};
use voyage_protocol::execution_identity::{ConfiguredExecutionIdentity, ObservedExecution};

pub(super) fn observe(
    pid: u32,
    incarnation: uuid::Uuid,
    identity: &ConfiguredExecutionIdentity,
    binary: &Path,
) -> Result<ObservedExecution> {
    let read = |name: &str, limit: u64| -> Result<String> {
        let mut text = String::new();
        fs::File::open(format!("/proc/{pid}/{name}"))?
            .take(limit + 1)
            .read_to_string(&mut text)?;
        ensure!(
            text.len() as u64 <= limit,
            "child identity observation exceeds limit"
        );
        Ok(text)
    };
    let status = read("status", 16384)?;
    let field = |name: &str| -> Result<&str> {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .context("child identity field missing")
    };
    for (name, expected) in [("Uid:", identity.uid), ("Gid:", identity.gid)] {
        let ids = field(name)?
            .split_whitespace()
            .map(str::parse::<u32>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(
            ids == vec![expected; 4],
            "child execution identity mismatch"
        );
    }
    let mut groups = field("Groups:")?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut expected = identity.supplementary_groups.clone();
    groups.sort_unstable();
    expected.sort_unstable();
    ensure!(groups == expected, "child supplementary groups mismatch");
    let capability = |name| -> Result<u64> { Ok(u64::from_str_radix(field(name)?.trim(), 16)?) };
    let effective = capability("CapEff:")?;
    let permitted = capability("CapPrm:")?;
    let inheritable = capability("CapInh:")?;
    let ambient = capability("CapAmb:")?;
    let no_new_privileges = field("NoNewPrivs:")?.trim() == "1";
    ensure!(
        identity.uid == 0
            || (effective == 0
                && permitted == 0
                && inheritable == 0
                && ambient == 0
                && no_new_privileges),
        "ordinary child retained execution privilege"
    );
    let stat = read("stat", 4096)?;
    let fields = stat
        .rsplit_once(") ")
        .context("invalid child process identity")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    ensure!(
        fields
            .get(1)
            .context("child parent missing")?
            .parse::<u32>()?
            == std::process::id(),
        "process is not an owned child"
    );
    let process_start_ticks = fields
        .get(19)
        .context("child start time missing")?
        .parse()?;
    let namespace = fs::read_link(format!("/proc/{pid}/ns/user"))?;
    ensure!(
        namespace == fs::read_link("/proc/self/ns/user")?,
        "unexpected execution user namespace"
    );
    super::launch::protected_binary(binary)?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut file = fs::File::open(binary)?.take(512 * 1024 * 1024 + 1);
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        ensure!(count <= 512 * 1024 * 1024, "execution binary exceeds limit");
        hasher.update(&buffer[..read]);
    }
    Ok(ObservedExecution {
        identity: identity.identity.clone(),
        incarnation,
        uid: identity.uid,
        gid: identity.gid,
        supplementary_groups: groups,
        effective_capabilities: effective,
        permitted_capabilities: permitted,
        inheritable_capabilities: inheritable,
        ambient_capabilities: ambient,
        no_new_privileges,
        user_namespace: namespace.to_string_lossy().into_owned(),
        process_start_ticks,
        release_digest: format!("{:x}", hasher.finalize()),
    })
}
