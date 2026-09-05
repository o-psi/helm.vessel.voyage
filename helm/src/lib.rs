pub mod agent;
pub mod attachment;
pub mod completion;
pub mod config;
pub mod context;
mod file_publication;
pub mod local_provider;
pub mod markdown;
pub mod model;
pub mod onboarding;
pub mod policy;
pub mod policy_profile;
pub mod provider;
pub mod session;
pub mod subagent;
pub mod supervision;
pub mod terminal;
pub mod titles;
pub mod todo;
pub mod tools;
pub mod tui;
pub mod workflow;

pub use agent::{Agent, AgentEvent, AgentOutcome, EventSink};
pub use config::{Config, ProviderKind};
pub use model::{Message, Role};

mod workspace_instructions;
