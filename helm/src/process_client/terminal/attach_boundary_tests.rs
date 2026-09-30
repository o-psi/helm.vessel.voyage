//! Source-prepared private terminal protocol journeys on a child-owned PTY.
//! No human terminal, Vessel, provider or actual terminal program is attached.
use super::*;
use crate::process_client::loopback_tests::Peer;
use serde_json::json;
const MODE: &str = "HELM_TERMINAL_ATTACH_FIXTURE_MODE";
const ROOT: &str = "HELM_TERMINAL_ATTACH_FIXTURE_ROOT";
#[test]
fn attached_terminal_child() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut peer=Peer::open().await;let client=peer.client.clone();let session=Uuid::new_v4();let incarnation=Uuid::new_v4();let run=Uuid::new_v4();let terminal_id=Uuid::new_v4();let stage=mode.clone();let directory=root.clone();
        let server=tokio::spawn(async move {
            let(id,command)=peer.command().await;assert!(matches!(command,VesselCommand::Inspect{session_id} if session_id==session));
            peer.reply(id,json!({"session_id":session,"incarnation":incarnation,"workspace":directory,"state":"live"})).await;
            let(id,command)=peer.command().await;assert!(matches!(command,VesselCommand::Voyage(voyage_protocol::vessel::VoyageRequest{command:voyage_protocol::vessel::VoyageCommand::Terminal{operation:TerminalAction::Attach,..},..})));
            peer.voyage_reply(id,session,incarnation,json!({"status":"attached"})).await;
            let(id,command)=peer.command().await;assert!(matches!(command,VesselCommand::Voyage(voyage_protocol::vessel::VoyageRequest{command:voyage_protocol::vessel::VoyageCommand::Terminal{operation:TerminalAction::Resize{..},..},..})));
            if stage=="wrong-owner" {peer.voyage_reply(id,session,Uuid::new_v4(),json!({"status":"resized"})).await;return;}
            peer.voyage_reply(id,session,incarnation,json!({"status":"resized"})).await;
            let(id,command)=peer.command().await;assert!(matches!(command,VesselCommand::Voyage(voyage_protocol::vessel::VoyageRequest{command:voyage_protocol::vessel::VoyageCommand::Terminal{operation:TerminalAction::Snapshot,..},..})));
            let state=if stage=="already-exited"{"exited"}else{"running"};
            peer.voyage_reply(id,session,incarnation,json!({"terminal_id":terminal_id,"run_id":run,"revision":1,"state":state,"title":"fixture\u{001b} private terminal","rows":["fixture row"],"cursor":null})).await;
            std::fs::write(directory.join("ready"),b"ready").unwrap();
            if stage=="already-exited" {return;}
            loop {
                let(id,command)=peer.command().await;let VesselCommand::Voyage(voyage_protocol::vessel::VoyageRequest{command:voyage_protocol::vessel::VoyageCommand::Terminal{operation,..},..})=command else{panic!("terminal only")};
                assert!(matches!(operation,TerminalAction::Snapshot),"fixture sends no private program input");
                let state=if stage=="observed-exit"{"exited"}else{"running"};
                peer.voyage_reply(id,session,incarnation,json!({"terminal_id":terminal_id,"run_id":run,"revision":2,"state":state,"rows":["fixture row"]})).await;
                if stage=="observed-exit" {return;}
            }
        });
        let result=tokio::time::timeout(std::time::Duration::from_secs(4),attach_observed(&client,session,incarnation,run,terminal_id)).await.unwrap();
        if mode=="detach" {result.unwrap();}else{assert!(result.is_err());}
        server.abort();let _=server.await;std::fs::write(root.join("completed"),b"completed").unwrap();
    });
}
fn owned_pty(mode: &str) {
    use std::{
        io::{Read, Write},
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir().unwrap();
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let fd = pair.master.as_raw_fd().unwrap();
    fn attributes(fd: i32) -> libc::termios {
        let mut attr = std::mem::MaybeUninit::uninit();
        assert_eq!(unsafe { libc::tcgetattr(fd, attr.as_mut_ptr()) }, 0);
        unsafe { attr.assume_init() }
    }
    let saved = attributes(fd);
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let mut command = portable_pty::CommandBuilder::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "process_client::terminal::attach_boundary_tests::attached_terminal_child",
        "--nocapture",
    ]);
    command.env(MODE, mode);
    command.env(ROOT, root.path());
    command.env("TERM", "xterm-256color");
    command.env("HELM_COLOR", "never");
    struct Child(Box<dyn portable_pty::Child + Send + Sync>);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(pair.slave.spawn_command(command).unwrap());
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let deadline = Instant::now() + Duration::from_secs(7);
    let mut captured = Vec::new();
    let mut sent = false;
    loop {
        let mut bytes = [0; 4096];
        while let Ok(n @ 1..) = reader.read(&mut bytes) {
            captured.extend_from_slice(&bytes[..n]);
            assert!(captured.len() < 65536);
        }
        if mode == "detach" && !sent && root.path().join("ready").exists() {
            writer.write_all(b"\x1d").unwrap();
            writer.flush().unwrap();
            sent = true;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(
                status.success(),
                "isolated terminal fixture failed ({mode})"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "isolated terminal fixture deadline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(root.path().join("completed")).unwrap(),
        b"completed"
    );
    let restored = attributes(fd);
    assert_eq!(
        (
            saved.c_iflag,
            saved.c_oflag,
            saved.c_cflag,
            saved.c_lflag,
            saved.c_cc
        ),
        (
            restored.c_iflag,
            restored.c_oflag,
            restored.c_cflag,
            restored.c_lflag,
            restored.c_cc
        )
    );
    if mode == "detach" {
        assert!(sent);
    }
    // Unknown exit/owner outcomes return a cleanup refusal to an observed Helm
    // interface rather than letting partially decoded input enter the composer.
}
#[test]
fn explicit_detach_restores_owned_terminal_without_sending_program_input() {
    owned_pty("detach");
}
#[test]
fn changed_owner_after_attach_restores_owned_terminal_and_refuses_frontend_resume() {
    owned_pty("wrong-owner");
}
#[test]
fn initial_exited_program_restores_owned_terminal_and_refuses_implicit_composer_input() {
    owned_pty("already-exited");
}
#[test]
fn observed_program_exit_restores_owned_terminal_without_treating_it_as_explicit_detach() {
    owned_pty("observed-exit");
}
