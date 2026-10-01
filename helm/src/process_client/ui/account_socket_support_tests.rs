//! Scripted human account/profile host over the existing private loopback socket.
//! This is metadata and response simulation, never a provider or an executor.
use super::socket_support_tests::{Server, voyage};
use super::{
    App,
    account_test_support::Fixture,
    accounts::app_tests,
    state::{Target, View},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use voyage_protocol::{
    accounts::*,
    execution_profiles::{ExecutionProfile, ProfileCatalogue},
    vessel::{VesselCommand, VoyageCommand},
};

pub(super) const CODE: &str = "LOOPBACK-HUMAN-CODE";
pub(super) struct Host {
    pub host: Uuid,
    pub account: AccountBinding,
    pub profile: ExecutionProfile,
    pub default: Option<AccountBinding>,
    pub default_revision: u64,
    pub profile_revision: u64,
    pub profile_default: Option<Uuid>,
    pub profiles: Vec<ExecutionProfile>,
    pub can_manage: bool,
    pub capabilities: bool,
    pub nil_host: bool,
    pub refuse_accounts: bool,
    pub refuse_defaults: bool,
    pub refuse_usage: bool,
    pub wrong_usage: bool,
    pub refuse_default_change: bool,
    pub wrong_default_ack: bool,
    pub refuse_profiles: bool,
    pub refuse_profile_change: bool,
    pub malformed_profile_ack: bool,
    pub refuse_private: bool,
    pub refuse_cancel: bool,
    pub enrollment: Option<VesselCommand>,
    pub enrollment_state: EnrollmentState,
    pub calls: Vec<VesselCommand>,
}
impl Host {
    fn new() -> Self {
        let account = AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 2,
            connection_revision: 3,
            transport: Transport::OpenaiResponses,
        };
        let profile = ExecutionProfile {
            id: Uuid::new_v4(),
            name: "Host profile".into(),
            account: account.clone(),
            model: "profile-model".into(),
            reasoning_effort: Some("high".into()),
            service_tier: None,
        };
        Self {
            host: Uuid::new_v4(),
            account,
            profile: profile.clone(),
            default: None,
            default_revision: 7,
            profile_revision: 9,
            profile_default: None,
            profiles: vec![profile],
            can_manage: true,
            capabilities: true,
            nil_host: false,
            refuse_accounts: false,
            refuse_defaults: false,
            refuse_usage: false,
            wrong_usage: false,
            refuse_default_change: false,
            wrong_default_ack: false,
            refuse_profiles: false,
            refuse_profile_change: false,
            malformed_profile_ack: false,
            refuse_private: false,
            refuse_cancel: false,
            enrollment: None,
            enrollment_state: EnrollmentState::Pending,
            calls: vec![],
        }
    }
    pub fn catalogue(&self) -> Value {
        json!({"accounts":[AccountDescriptor { id:self.account.account_id, connection_id:self.account.connection_id, alias:"loopback".into(), label:"Loopback account".into(), metadata_revision:1, identity_generation:self.account.identity_generation, credential_revision:1, capability_revision:4, availability:CredentialAvailability::Available, state:AccountState::Ready }],
            "connections":[ConnectionDescriptor { id:self.account.connection_id, revision:self.account.connection_revision, label:"Loopback API".into(),endpoint:"https://fixture.invalid/v1".into(),transports:vec![self.account.transport] },
                ConnectionDescriptor {id:Uuid::from_u128(353),revision:1,label:"ChatGPT".into(),endpoint:"https://chatgpt.com/backend-api/codex".into(),transports:vec![Transport::ChatgptOauth]}],
            "default_account":self.default,"default_revision":self.default_revision,"can_set_default":self.can_manage})
    }
    pub fn profile_catalogue(&self) -> ProfileCatalogue {
        ProfileCatalogue {
            revision: self.profile_revision,
            profiles: self.profiles.clone(),
            default_profile_id: self.profile_default,
            can_manage: self.can_manage,
        }
    }
    fn status(&self, enrollment_id: Uuid, private: bool) -> Value {
        let status = EnrollmentStatus {
            enrollment_id,
            state: self.enrollment_state,
            account_id: None,
            expires_at: u64::MAX,
            effects_may_have_occurred: true,
        };
        if private {
            json!(PrivateEnrollmentStatus {
                status,
                user_code: Some(CODE.into()),
                verification_uri: Some("https://auth.openai.com/codex/device".into()),
                failure: None
            })
        } else {
            json!(status)
        }
    }
    fn reply(&mut self, command: &VesselCommand) -> Result<Value, String> {
        self.calls.push(command.clone());
        Ok(match command {
            VesselCommand::Capabilities => {
                json!({"vessel_id":if self.nil_host {Uuid::nil()} else {self.host},"features":if self.capabilities {vec!["provider_accounts","private_account_enrollment","execution_profiles"]} else {vec![]},"rights":["account_use","account_enroll"]})
            }
            VesselCommand::Accounts { .. } => {
                if self.refuse_accounts {
                    return Err(format!("private remote diagnostic {CODE}"));
                }
                self.catalogue()
            }
            VesselCommand::AccountDefaults { .. } => {
                if self.refuse_defaults {
                    return Err("defaults denied".into());
                }
                json!({"code":"default_account_required","account":null})
            }
            VesselCommand::AccountUsage { account, .. } => {
                if self.refuse_usage {
                    return Err(format!("remote usage diagnostic {CODE}"));
                }
                let mut account = account.clone();
                if self.wrong_usage {
                    account.identity_generation += 1;
                }
                json!(AccountUsageObservation {
                    account,
                    capability_revision: 4,
                    snapshot: None,
                    refresh_status: AccountUsageRefreshStatus::Unsupported,
                    attempted_at: Some(1)
                })
            }
            VesselCommand::AccountSetDefault {
                account,
                expected_revision,
                ..
            } => {
                if self.refuse_default_change {
                    return Err("uncertain synthetic default response".into());
                }
                assert_eq!(*expected_revision, self.default_revision);
                self.default_revision += 1;
                self.default = Some(account.clone());
                let mut actual = account.clone();
                if self.wrong_default_ack {
                    actual.identity_generation += 1;
                }
                json!({"default_account":actual,"default_revision":self.default_revision})
            }
            VesselCommand::EnrollAccount { enrollment_id, .. } => {
                assert!(self.enrollment.is_none());
                self.enrollment = Some(command.clone());
                self.status(*enrollment_id, false)
            }
            VesselCommand::ResolveAccountEnrollment {
                command_id,
                enrollment_id,
                workspace,
                connection_id,
                alias,
                label,
            } => {
                let Some(VesselCommand::EnrollAccount {
                    command_id: old_id,
                    enrollment_id: old_enrollment,
                    workspace: old_workspace,
                    connection_id: old_connection,
                    alias: old_alias,
                    label: old_label,
                }) = &self.enrollment
                else {
                    panic!("resolve without retained enrollment");
                };
                assert_eq!(
                    (
                        command_id,
                        enrollment_id,
                        workspace,
                        connection_id,
                        alias,
                        label
                    ),
                    (
                        old_id,
                        old_enrollment,
                        old_workspace,
                        old_connection,
                        old_alias,
                        old_label
                    )
                );
                self.status(*enrollment_id, false)
            }
            VesselCommand::PrivateAccountEnrollment { enrollment_id, .. } => {
                if self.refuse_private {
                    return Err(format!("remote code diagnostic {CODE}"));
                }
                self.status(*enrollment_id, true)
            }
            VesselCommand::CancelAccountEnrollment { enrollment_id, .. } => {
                if self.refuse_cancel {
                    return Err("cancellation reply uncertain".into());
                }
                self.enrollment_state = EnrollmentState::Cancelled;
                self.status(*enrollment_id, false)
            }
            VesselCommand::Profiles { .. } => {
                if self.refuse_profiles {
                    return Err("profile observation denied".into());
                }
                json!(self.profile_catalogue())
            }
            VesselCommand::SaveProfile {
                expected_revision,
                profile,
                make_default,
                ..
            } => {
                if self.refuse_profile_change {
                    return Err("profile authority withdrawn".into());
                }
                assert_eq!(*expected_revision, self.profile_revision);
                self.profile_revision += 1;
                self.profiles.retain(|p| p.id != profile.id);
                self.profiles.push(profile.clone());
                if *make_default {
                    self.profile_default = Some(profile.id);
                }
                if self.malformed_profile_ack {
                    json!({"revision":"invalid","profiles":[]})
                } else {
                    json!(self.profile_catalogue())
                }
            }
            VesselCommand::SetDefaultProfile {
                expected_revision,
                profile_id,
                ..
            } => {
                if self.refuse_profile_change {
                    return Err("profile authority withdrawn".into());
                }
                assert_eq!(*expected_revision, self.profile_revision);
                assert!(self.profiles.iter().any(|p| p.id == *profile_id));
                self.profile_revision += 1;
                self.profile_default = Some(*profile_id);
                json!(self.profile_catalogue())
            }
            VesselCommand::DeleteProfile {
                expected_revision,
                profile_id,
                ..
            } => {
                if self.refuse_profile_change {
                    return Err("profile authority withdrawn".into());
                }
                assert_eq!(*expected_revision, self.profile_revision);
                self.profile_revision += 1;
                self.profiles.retain(|p| p.id != *profile_id);
                if self.profile_default == Some(*profile_id) {
                    self.profile_default = None;
                }
                json!(self.profile_catalogue())
            }
            VesselCommand::Voyage(request) => voyage(
                command,
                match &request.command {
                    VoyageCommand::Controls { section, .. } if section == "models" => {
                        json!({"value":[]})
                    }
                    VoyageCommand::SetAccountInference { command_id, .. } => {
                        json!({"command_id":command_id,"status":"applied","revision":18})
                    }
                    VoyageCommand::Receipt { command_id } => {
                        json!({"command_id":command_id,"status":"unknown"})
                    }
                    VoyageCommand::Resolve { command_id, .. } => {
                        json!({"command_id":command_id,"status":"unknown"})
                    }
                    _ => json!({}),
                },
            ),
            _ => panic!("unexpected scripted account/profile operation"),
        })
    }
}

pub(super) struct Peer {
    pub app: App,
    pub target: Target,
    pub host: Arc<Mutex<Host>>,
    pub updates: tokio::sync::mpsc::Receiver<super::observe::Update>,
    _server: Server,
    pub fixture: Fixture,
}
impl Peer {
    pub async fn new() -> Self {
        let fixture = Fixture::new();
        let host = Arc::new(Mutex::new(Host::new()));
        let replies = host.clone();
        let server = Server::new(move |command| replies.lock().unwrap().reply(command)).await;
        let mut app = app_tests::app(fixture.0.path());
        app.clients = super::routes::Routes::new(vec![server.client.clone()]);
        let target = server.target;
        let binding = host.lock().unwrap().account.clone();
        let mut view=View::new(serde_json::from_value(json!({"session_id":target.session,"incarnation":server.incarnation,"workspace":fixture.0.path(),"state":"live","name":"Scripted voyage"})).unwrap());
        view.snapshot=Some(serde_json::from_value(json!({"session_id":target.session,"revision":17,"model":"original-model","messages":[],"inference":{"account":binding,"provider":"openai-responses","model":"original-model","reasoning_effort":null,"service_tier":null},"run":null})).unwrap());
        view.draft.insert_str("Unsent local prompt Δ");
        app.views.insert(target, view);
        app.selected = Some(target);
        let (sender, updates) = tokio::sync::mpsc::channel(32);
        app.sender = sender;
        Self {
            app,
            target,
            host,
            updates,
            _server: server,
            fixture,
        }
    }
    pub fn unchanged(&self) {
        assert_eq!(
            self.app.views[&self.target].draft.text,
            "Unsent local prompt Δ"
        );
        assert!(self.app.views[&self.target].pending.is_none());
        assert_eq!(
            self.app.views[&self.target]
                .snapshot
                .as_ref()
                .unwrap()
                .model,
            "original-model"
        );
        assert!(
            self.app.views[&self.target]
                .snapshot
                .as_ref()
                .unwrap()
                .messages
                .is_empty()
        );
        assert!(!self.app.status.contains(CODE));
    }
}
