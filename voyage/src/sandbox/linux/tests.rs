use super::*;
// Interpret the small classic-BPF instruction set emitted by filter; no syscall
// filters are installed in the test process or on the user's system.
fn decision(program: &[libc::sock_filter], arch: u32, nr: u32, args: [u64; 6]) -> u32 {
    let mut data = [0u8; 64];
    data[0..4].copy_from_slice(&nr.to_ne_bytes());
    data[4..8].copy_from_slice(&arch.to_ne_bytes());
    for (i, arg) in args.iter().enumerate() {
        data[16 + i * 8..24 + i * 8].copy_from_slice(&arg.to_ne_bytes());
    }
    let (mut pc, mut a) = (0usize, 0u32);
    for _ in 0..program.len() * 2 {
        let i = &program[pc];
        match i.code {
            0x20 => {
                a = u32::from_ne_bytes(data[i.k as usize..i.k as usize + 4].try_into().unwrap());
                pc += 1;
            }
            0x15 => pc += 1 + if a == i.k { i.jt } else { i.jf } as usize,
            0x45 => pc += 1 + if a & i.k != 0 { i.jt } else { i.jf } as usize,
            0x06 => return i.k,
            _ => panic!("unexpected BPF opcode {}", i.code),
        }
    }
    panic!("unterminated filter")
}
#[test]
fn syscall_filter_denies_escape_foreign_abi_and_terminal_injection() {
    for extension in [false, true] {
        for network in [Network::Denied, Network::Host] {
            let p = filter(network, extension);
            assert_eq!(decision(&p, 0, libc::SYS_read as u32, [0; 6]), 0x80000000);
            assert_eq!(decision(&p, 0xc000003e, 0x40000000, [0; 6]), 0x80000000);
            for nr in [
                libc::SYS_ptrace,
                libc::SYS_mount,
                libc::SYS_umount2,
                libc::SYS_pivot_root,
            ] {
                assert_eq!(
                    decision(&p, 0xc000003e, nr as u32, [0; 6]) & 0xffff0000,
                    0x50000
                );
            }
            assert_eq!(
                decision(&p, 0xc000003e, libc::SYS_read as u32, [0; 6]),
                0x7fff0000
            );
            let mut args = [0; 6];
            #[allow(clippy::unnecessary_cast)] // ioctl constant width is target-dependent.
            {
                args[1] = libc::TIOCSTI as u64;
            }
            assert_eq!(
                decision(&p, 0xc000003e, libc::SYS_ioctl as u32, args) & 0xffff0000,
                0x50000
            );
            for family in [
                libc::AF_UNIX,
                libc::AF_INET,
                libc::AF_INET6,
                libc::AF_NETLINK,
            ] {
                args = [0; 6];
                args[0] = family as u64;
                let permitted =
                    network == Network::Host && matches!(family, libc::AF_INET | libc::AF_INET6);
                assert_eq!(
                    decision(&p, 0xc000003e, libc::SYS_socket as u32, args) == 0x7fff0000,
                    permitted
                );
            }
        }
    }
}
#[test]
fn roots_are_pinned_normalized_and_reject_reserved_or_symlinked_paths() {
    let root = tempfile::tempdir().unwrap();
    let child = root.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let file = child.join("file");
    std::fs::write(&file, b"fixture").unwrap();
    let pinned = roots(
        &[file.clone(), root.path().into()],
        std::slice::from_ref(&child),
    )
    .unwrap();
    assert_eq!(pinned[0].path, root.path());
    assert!(pinned.iter().any(|r| r.path == child && r.writable));
    assert!(pin(std::path::Path::new("relative")).is_err());
    assert!(pin(&root.path().join("child/../child")).is_err());
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&child, &link).unwrap();
    assert!(pin(&link).is_err());
    for path in ["/", "/tmp", "/usr/local", "/proc", "/home"] {
        assert!(roots(&[path.into()], &[]).is_err());
    }
    assert!(roots(&vec![file; 129], &[]).is_err());
}

#[test]
fn sandbox_launch_assembly_pins_mounts_and_scrubs_adapter_environment_without_spawning() {
    let root = tempfile::tempdir().unwrap();
    for network in [Network::Denied, Network::Host] {
        let sandbox = Sandbox::new(
            &Settings {
                mode: Mode::Required,
                network,
                ..Default::default()
            },
            &[root.path().into()],
            &[root.path().into()],
        )
        .unwrap();
        for readonly in [false, true] {
            let mut command = Command::new("/bin/echo");
            command
                .arg("fixture")
                .env_clear()
                .env("FIXTURE_VALUE", "not-a-secret");
            sandbox
                .apply_read_only(&mut command, root.path(), readonly)
                .unwrap();
            assert_eq!(command.get_program(), "/bin/echo");
            assert!(command.get_envs().next().is_none());
        }
    }
    let image = seal_extension(b"synthetic executable bytes").unwrap();
    let sandbox = Sandbox::new(
        &Settings {
            mode: Mode::Required,
            ..Default::default()
        },
        &[],
        &[],
    )
    .unwrap();
    let mut command = Command::new("/extension");
    command.env_clear();
    sandbox.apply_extension(&mut command, &image).unwrap();
    let mut invalid = Command::new("/extension");
    invalid.arg("not-permitted");
    assert!(sandbox.apply_extension(&mut invalid, &image).is_err());
}
#[test]
fn launch_arguments_are_bounded_and_sealed_extension_cannot_be_mutated() {
    let root = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(
        &Settings {
            mode: Mode::Required,
            ..Default::default()
        },
        &[root.path().into()],
        &[],
    )
    .unwrap();
    let mut command = Command::new("/bin/true");
    command.env("TOO_LARGE", "x".repeat(1024 * 1024));
    assert!(sandbox.apply(&mut command, root.path()).is_err());
    let image = seal_extension(b"fixture").unwrap();
    let mut copy = image.file.try_clone().unwrap();
    assert!(copy.write_all(b"change").is_err());
}
