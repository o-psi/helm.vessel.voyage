//! Real runtime entry points with isolated state, no owner start, and no provider.
//! Preserve LLVM's profile destination so these subprocess routes are measurable.
use std::{fs,process::{Command,Stdio,Output},time::{Duration,Instant}};
use uuid::Uuid;
struct Cli(tempfile::TempDir);
impl Cli {
    fn new()->Self{Self(tempfile::tempdir().unwrap())}
    fn run(&self,args:&[&str])->Output{
        let stdout=tempfile::tempfile().unwrap();let stderr=tempfile::tempfile().unwrap();
        let mut command=Command::new(env!("CARGO_BIN_EXE_voyage"));
        command.env_clear().env("HOME",self.0.path()).env("PATH","/usr/bin:/bin").env("TERM","dumb").current_dir(self.0.path()).stdin(Stdio::null()).stdout(stdout.try_clone().unwrap()).stderr(stderr.try_clone().unwrap());
        for(key,name)in[("XDG_CONFIG_HOME","config"),("XDG_DATA_HOME","data"),("XDG_STATE_HOME","state"),("XDG_CACHE_HOME","cache"),("XDG_RUNTIME_DIR","runtime")]{let path=self.0.path().join(name);fs::create_dir_all(&path).unwrap();command.env(key,path);}
        if let Some(destination)=std::env::var_os("LLVM_PROFILE_FILE"){command.env("LLVM_PROFILE_FILE",destination);}
        let mut child=command.args(args).spawn().unwrap();let deadline=Instant::now()+Duration::from_secs(10);
        let status=loop{if let Some(status)=child.try_wait().unwrap(){break status;}if Instant::now()>=deadline{let _=child.kill();let _=child.wait();panic!("offline runtime route exceeded its bound: {args:?}");}std::thread::sleep(Duration::from_millis(10));};
        use std::io::{Read,Seek};
        let read=|mut file:fs::File|{assert!(file.metadata().unwrap().len()<=2*1024*1024);file.rewind().unwrap();let mut bytes=Vec::new();file.take(2*1024*1024+1).read_to_end(&mut bytes).unwrap();bytes};
        Output{status,stdout:read(stdout),stderr:read(stderr)}
    }
    fn ok(&self,args:&[&str])->String{let output=self.run(args);assert!(output.status.success(),"{args:?}: {}",String::from_utf8_lossy(&output.stderr));String::from_utf8(output.stdout).unwrap()}
    fn fails(&self,args:&[&str],message:&str){let output=self.run(args);assert!(!output.status.success(),"unexpected success {args:?}");assert!(String::from_utf8_lossy(&output.stderr).contains(message),"{args:?}: {}",String::from_utf8_lossy(&output.stderr));assert!(output.stdout.is_empty());}
}

#[test]
fn all_shell_documents_reach_actual_runtime_entry_without_creating_an_owner(){
    let cli=Cli::new();for shell in["bash","zsh","fish","powershell","elvish"]{let text=cli.ok(&["completions",shell]);assert!(text.contains("voyage"));assert!(text.contains("observe-catalogue"));}
    let man=cli.ok(&["manpage"]).replace("\\-", "-");assert!(man.contains("validate-start"));assert!(man.contains("host-resources"));
    assert!(cli.ok(&["--version"]).starts_with("voyage "));let help=cli.ok(&["--help"]);assert!(help.contains("Independent voyage session runtime"));assert!(!help.contains("identity-helper"));assert!(!help.contains("transition-helper"));
    assert!(!cli.0.path().join("journal").exists());assert!(!cli.0.path().join("runtime.sock").exists());
}

#[test]
fn local_validation_does_not_launch_or_require_a_provider(){
    let cli=Cli::new();assert!(cli.ok(&["validate-start","--workspace",cli.0.path().to_str().unwrap()]).is_empty());
    assert!(!cli.0.path().join("journal").exists());assert!(!cli.0.path().join("runtime.sock").exists());
    let output=cli.run(&["validate-start","--workspace",cli.0.path().join("missing-workspace").to_str().unwrap()]);assert!(!output.status.success());assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn validation_refuses_a_public_launch_file_before_configuration_resolution(){
    use std::os::unix::fs::PermissionsExt;
    let cli=Cli::new();let config=cli.0.path().join("public-launch.json");fs::write(&config,b"CLI_PRIVATE_CONFIG_CANARY").unwrap();fs::set_permissions(&config,fs::Permissions::from_mode(0o644)).unwrap();
    cli.fails(&["validate-start","--workspace",cli.0.path().to_str().unwrap(),"--config",config.to_str().unwrap()],"private and owned");
    assert!(!cli.0.path().join("journal").exists());
}

#[cfg(unix)]
#[test]
fn oversized_socket_location_refuses_before_registration_or_journal(){
    let cli=Cli::new();let directory=cli.0.path().join("r".repeat(120));let session=Uuid::new_v4().to_string();let incarnation=Uuid::new_v4().to_string();
    cli.fails(&["serve","--directory",directory.to_str().unwrap(),"--session",&session,"--incarnation",&incarnation,"--workspace",cli.0.path().to_str().unwrap()],"socket path exceeds");
    assert!(!directory.join("journal").exists());assert!(!directory.join("runtime.sock").exists());
}

#[test]
fn resource_attestations_require_exact_confirmation_before_touching_host_store(){
    let cli=Cli::new();let id=Uuid::new_v4().to_string();let wrong=Uuid::new_v4().to_string();
    cli.fails(&["host-resources","attest",&id,"--confirm",&wrong,"--reason","CLI_PRIVATE_REASON"],"exact reservation");
    cli.fails(&["host-resources","browser-capacity",&id,"--confirm",&wrong,"--session-dir",cli.0.path().to_str().unwrap(),"--observed-no-descendants","--reason","CLI_PRIVATE_REASON"],"exact session");
    assert_eq!(fs::read_dir(cli.0.path().join("state")).unwrap().count(),0);
}

#[test]
fn catalogue_cli_never_initializes_an_absent_journal(){
    let cli=Cli::new();let directory=cli.0.path().join("never-initialized");let session=Uuid::new_v4().to_string();
    let output=cli.run(&["observe-catalogue","--directory",directory.to_str().unwrap(),"--session",&session]);assert!(!output.status.success());assert!(output.stdout.is_empty());assert!(!directory.exists());
}

#[test]
fn import_plan_refuses_unsupported_marker_without_mutating_source(){
    let cli=Cli::new();let session=Uuid::new_v4().to_string();let path=cli.0.path().join(format!("{session}.json"));let original=br#"{"format":"helm.session-transfer","version":99}"#;fs::write(&path,original).unwrap();
    #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();}
    cli.fails(&["import-plan","--source-directory",cli.0.path().to_str().unwrap(),"--session",&session],"unsupported transfer marker version");assert_eq!(fs::read(path).unwrap(),original);assert!(!cli.0.path().join("journal").exists());
}
