use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Step {
    Role,
    Roles,
    Vessel,
    Address,
    Reach,
    Internet,
    Tunnel,
    HostAddress,
    Admin,
    VesselBoot,
    Provider,
    Api,
    Endpoint,
    Model,
    Name,
    Folder,
    Sharing,
    Approval,
    WorkerBoot,
    Controller,
    Review,
    Done,
}

impl Step {
    pub fn title(self) -> &'static str {
        match self {
            Self::Role => "What will this machine do?",
            Self::Roles => "Choose this machine's roles",
            Self::Vessel => "Do you already have a Vessel?",
            Self::Address => "Where is your Vessel?",
            Self::Reach => "Who needs to reach Vessel?",
            Self::Internet => "How should machines reach Vessel?",
            Self::Tunnel => "Choose a tunnel approach",
            Self::HostAddress => "Which address should Helm use?",
            Self::Admin => "Name the first Vessel administrator",
            Self::VesselBoot => "When should Vessel run?",
            Self::Provider => "How should Helm access a model?",
            Self::Api => "Choose an API provider",
            Self::Endpoint => "Where is your model server?",
            Self::Model => "Which model would you use?",
            Self::Name => "Name this Helm",
            Self::Folder => "Which folder may remote work use?",
            Self::Sharing => "What may Vessel access?",
            Self::Approval => "Who should approve remote actions?",
            Self::WorkerBoot => "When should this Helm be available?",
            Self::Controller => "How will you control other Helms?",
            Self::Review => "Review your setup",
            Self::Done => "Simulation complete",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Role | Self::Roles => "Helm runs work. Vessel connects and manages machines.",
            Self::Vessel => "Controller login and worker enrollment are separate steps.",
            Self::Address | Self::HostAddress => {
                "Example address only. No connection or identity check will run."
            }
            Self::Reach => "Direct connectivity adds no tunnel software or service account.",
            Self::Internet => {
                "Direct access needs a reachable address. Some routers and ISPs prevent it."
            }
            Self::Tunnel => {
                "Quick: no signup, temporary URL, no SSE. Named: Cloudflare account and domain. Relay: a proposal, not an available service."
            }
            Self::Admin => {
                "Use an example name. Authentication setup is simulated; do not enter a password."
            }
            Self::VesselBoot | Self::WorkerBoot => {
                "Background startup would need an explicit service identity. No service is created."
            }
            Self::Provider => {
                "Only machines running work need model access. No login or API key is collected."
            }
            Self::Api => "API usage has separate billing. Credentials would stay on this Helm.",
            Self::Endpoint => "No server scan or network request. Use a URL without credentials.",
            Self::Model => {
                "Enter an example model ID. A real setup would discover your available models."
            }
            Self::Name => "A friendly example name for the fleet. No identity is enrolled.",
            Self::Folder => {
                "One example absolute path. Additional folders could be added after setup. No files are inspected."
            }
            Self::Sharing => "Enrollment never grants access to your existing private sessions.",
            Self::Approval => {
                "No answer means blocked or denied. Choosing a remote approver would require local opt-in."
            }
            Self::Controller => {
                "Control-only mode needs no local model. Delegating through a local agent also needs model setup."
            }
            Self::Review => {
                "All actions below are a preview. Back edits the previous answer; Change answers returns to roles."
            }
            Self::Done => {
                "Nothing was installed, configured, authenticated, connected, or verified. Answers are kept only in memory."
            }
        }
    }

    pub fn options(self) -> &'static [&'static str] {
        match self {
            Self::Role => &[
                "Work locally with Helm",
                "Manage other machines",
                "Let others manage this Helm",
                "Host Vessel",
                "Combine roles",
            ],
            Self::Roles => &[
                "Local work",
                "Control other Helms",
                "Accept remote work",
                "Host Vessel",
            ],
            Self::Vessel => &[
                "Connect to an existing Vessel",
                "Set up Vessel here",
                "Set up Vessel on another machine",
                "Connect later",
            ],
            Self::Reach => &[
                "Only this machine",
                "My local network",
                "Over the internet",
                "Configure later",
            ],
            Self::Internet => &[
                "Existing address / reverse proxy",
                "Public IP / router forwarding",
                "Without router changes",
            ],
            Self::Tunnel => &[
                "Quick Tunnel - free, no signup, temporary",
                "Named Cloudflare tunnel - account needed",
                "My own tunnel / reverse proxy",
                "Voyage relay - unavailable; defer",
                "Configure later",
            ],
            Self::VesselBoot => &["Start manually", "Start automatically at boot"],
            Self::Provider => &[
                "Configure later",
                "Sign in with ChatGPT",
                "Use an API provider",
                "Use a local model server",
                "Keep my existing configuration",
            ],
            Self::Api => &["OpenAI", "Anthropic", "Custom compatible endpoint"],
            Self::Sharing => &[
                "New Vessel sessions only",
                "Choose existing sessions later",
                "Configure sharing later",
            ],
            Self::Approval => &[
                "Ask me on this machine",
                "Authorized remote operator",
                "Configure unattended permissions later",
            ],
            Self::WorkerBoot => &[
                "While I explicitly run Helm",
                "Automatically in the background",
            ],
            Self::Controller => &[
                "Remote-control interface only",
                "Local agent plus remote control",
            ],
            Self::Review => &["Preview setup actions", "Change answers", "Cancel"],
            _ => &[],
        }
    }

    pub fn default_text(self) -> &'static str {
        match self {
            Self::Address | Self::HostAddress => "https://vessel.example.com",
            Self::Admin => "admin",
            Self::Endpoint => "http://127.0.0.1:11434",
            Self::Name => "my-helm",
            Self::Folder => "/path/to/workspace",
            _ => "",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Answer {
    Choice(usize),
    Roles([bool; 4]),
    Text(String),
}

#[derive(Default)]
pub struct Wizard {
    pub answers: BTreeMap<Step, Answer>,
    pub cursor: usize,
    pub input: String,
    pub roles: [bool; 4],
    pub error: Option<String>,
    pub scroll: u16,
    pub current: Option<Step>,
}

impl Wizard {
    pub fn new() -> Self {
        let mut w = Self::default();
        w.enter(Step::Role);
        w
    }
    pub fn step(&self) -> Step {
        self.current.unwrap_or(Step::Role)
    }
    pub fn choice(&self, s: Step) -> usize {
        match self.answers.get(&s) {
            Some(Answer::Choice(i)) => *i,
            _ => 0,
        }
    }
    fn text(&self, s: Step) -> &str {
        match self.answers.get(&s) {
            Some(Answer::Text(t)) => t,
            _ => s.default_text(),
        }
    }
    pub fn effective_roles(&self) -> [bool; 4] {
        match self.choice(Step::Role) {
            0 => [true, false, false, false],
            1 => [false, true, false, false],
            2 => [false, false, true, false],
            3 => [false, false, false, true],
            _ => match self.answers.get(&Step::Roles) {
                Some(Answer::Roles(r)) => *r,
                _ => [true, false, false, false],
            },
        }
    }
    pub fn route(&self) -> Vec<Step> {
        use Step::*;
        let [local, control, worker, host] = self.effective_roles();
        let mut r = vec![Role];
        if self.choice(Role) == 4 {
            r.push(Roles);
        }
        if (control || worker) && !host {
            r.push(Vessel);
            if self.choice(Vessel) == 0 {
                r.push(Address);
            }
        }
        let hosting = host || ((control || worker) && self.choice(Vessel) == 1);
        if hosting {
            r.push(Reach);
            if self.choice(Reach) == 2 {
                r.push(Internet);
                if self.choice(Internet) == 2 {
                    r.push(Tunnel);
                }
                if self.choice(Internet) != 2 || matches!(self.choice(Tunnel), 1 | 2) {
                    r.push(HostAddress);
                }
            } else if self.choice(Reach) == 1 {
                r.push(HostAddress);
            }
            r.extend([Admin, VesselBoot]);
        }
        if control {
            r.push(Controller);
        }
        if local || worker || (control && self.choice(Controller) == 1) {
            r.push(Provider);
            if self.choice(Provider) == 2 {
                r.push(Api);
            }
            if self.choice(Provider) == 3 || (self.choice(Provider) == 2 && self.choice(Api) == 2) {
                r.push(Endpoint);
            }
            if matches!(self.choice(Provider), 1..=3) {
                r.push(Model);
            }
        }
        if worker {
            r.extend([Name, Folder, Sharing, Approval, WorkerBoot]);
        }
        r.push(Review);
        r
    }
    pub fn enter(&mut self, s: Step) {
        self.current = Some(s);
        self.cursor = self.choice(s);
        self.input = self.text(s).to_owned();
        self.roles = match self.answers.get(&Step::Roles) {
            Some(Answer::Roles(r)) => *r,
            _ => [true, false, false, false],
        };
        self.error = None;
        self.scroll = 0;
    }
    fn save(&mut self) {
        let s = self.step();
        let a = if s == Step::Roles {
            Answer::Roles(self.roles)
        } else if !s.options().is_empty() {
            Answer::Choice(self.cursor)
        } else {
            Answer::Text(self.input.clone())
        };
        self.answers.insert(s, a);
    }
    pub fn back(&mut self) {
        let s = self.step();
        if s == Step::Done {
            self.enter(Step::Review);
            return;
        }
        self.save();
        let r = self.route();
        if let Some(i) = r.iter().position(|v| *v == s) {
            self.enter(r[i.saturating_sub(1)]);
        }
    }
    /// Returns true only when the user explicitly exits; no action executor exists.
    pub fn next(&mut self) -> bool {
        let s = self.step();
        if s == Step::Done {
            return true;
        }
        if s == Step::Review {
            match self.cursor {
                0 => self.enter(Step::Done),
                1 => self.enter(Step::Role),
                _ => return true,
            }
            return false;
        }
        if s == Step::Roles && !self.roles.iter().any(|v| *v) {
            self.error = Some("Select at least one role with Space.".into());
            return false;
        }
        if s.options().is_empty()
            && let Err(e) = validate(s, &self.input)
        {
            self.error = Some(e.into());
            return false;
        }
        self.save();
        let r = self.route();
        let i = r
            .iter()
            .position(|v| *v == s)
            .expect("active question belongs to route");
        self.enter(r[i + 1]);
        false
    }
    pub fn summary(&self) -> Vec<String> {
        let roles = self.effective_roles();
        let mut rows = vec![format!(
            "Roles: {}",
            Step::Roles
                .options()
                .iter()
                .zip(roles)
                .filter(|(_, on)| *on)
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        )];
        for s in self
            .route()
            .into_iter()
            .filter(|s| !matches!(s, Step::Role | Step::Roles | Step::Review))
        {
            let value = if s.options().is_empty() {
                self.text(s)
            } else {
                s.options()[self.choice(s)]
            };
            rows.push(format!("{}  {}", s.title(), value));
        }
        rows
    }
    pub fn actions(&self) -> Vec<String> {
        let [local, control, worker, host] = self.effective_roles();
        let hosting = host || ((control || worker) && self.choice(Step::Vessel) == 1);
        let mut a = Vec::new();
        if local || control || worker {
            a.push("Would install Helm.".into());
        }
        if hosting {
            a.push("Would install Vessel and create the first administrator.".into());
            let reach = self.choice(Step::Reach);
            let connectivity = match reach {
                0 => "Would bind Vessel to loopback.",
                1 => "Would configure authenticated LAN access.",
                2 if self.choice(Step::Internet) != 2 => {
                    "Would configure the chosen public address and authenticated TLS access."
                }
                2 => match self.choice(Step::Tunnel) {
                    0 => {
                        "Would try a temporary Cloudflare tunnel. Streaming compatibility remains unverified; no stable address promised."
                    }
                    1 => {
                        "Would guide Cloudflare account/domain setup and install cloudflared for a named tunnel."
                    }
                    2 => "Would configure your existing tunnel address.",
                    3 => "DEFERRED: Voyage relay does not exist; choose another connection path.",
                    _ => "DEFERRED: Vessel internet connectivity.",
                },
                _ => "DEFERRED: Vessel connectivity.",
            };
            a.push(connectivity.into());
            if self.choice(Step::VesselBoot) == 1 {
                a.push(
                    "Would confirm service account ownership and configure Vessel startup.".into(),
                );
            }
        }
        if (control || worker) && !host {
            match self.choice(Step::Vessel) {
                0 => a.push("Would verify Vessel identity before login or enrollment.".into()),
                2 => a.push("DEFERRED: run this installer on the Vessel host, then return with its address.".into()),
                3 => a.push("DEFERRED: connect this machine to Vessel.".into()),
                _ => {},
            }
        }
        if self.route().contains(&Step::Provider) {
            a.push(match self.choice(Step::Provider) {
                0 => "DEFERRED: model provider setup.",
                1 => "Would sign in with ChatGPT on this Helm; no API billing fallback.",
                2 => "Would request an API credential securely on this Helm; separate API billing applies.",
                3 => "Would check your local model server and selected model.",
                _ => "Would inspect and preserve your existing Helm configuration.",
            }.into());
        }
        let deferred = (!hosting && (control || worker) && self.choice(Step::Vessel) >= 2)
            || (hosting
                && (self.choice(Step::Reach) == 3
                    || (self.choice(Step::Reach) == 2
                        && self.choice(Step::Internet) == 2
                        && self.choice(Step::Tunnel) >= 3)));
        if worker {
            if deferred {
                a.push("DEFERRED: worker enrollment until Vessel is reachable.".into());
            } else {
                a.push("Would enroll this Helm using a separate invitation, keeping provider credentials local.".into());
            }
            a.push("Would apply the selected folder, sharing and approval policy; no implicit private-session access.".into());
            if self.choice(Step::Sharing) != 0 {
                a.push(
                    "DEFERRED: session-sharing selection; existing sessions remain private.".into(),
                );
            }
            if self.choice(Step::Approval) == 2 {
                a.push(
                    "DEFERRED: unattended grants; approval-required work stays blocked or denied."
                        .into(),
                );
            }
            if self.choice(Step::WorkerBoot) == 1 {
                a.push("Would confirm the worker service account and its credentials before enabling background startup.".into());
            }
        }
        if control {
            if deferred {
                a.push(
                    "DEFERRED: operator login and fleet listing until Vessel is reachable.".into(),
                );
            } else {
                a.push("Would authenticate the operator and list enrolled Helms, or offer an invitation if none exist.".into());
            }
        }
        a.push("NOT RUN: downloads of Helm/Vessel, authentication, enrollment, service changes and connectivity checks.".into());
        a
    }
}

pub fn validate(s: Step, text: &str) -> Result<(), &'static str> {
    if text.trim().is_empty() {
        return Err("Enter a value, or go Back to choose deferred setup.");
    }
    if text.len() > 512 || text.chars().any(char::is_control) {
        return Err("Use at most 512 bytes without control characters.");
    }
    if matches!(s, Step::Address | Step::HostAddress | Step::Endpoint) {
        let u = url::Url::parse(text).map_err(|_| "Enter a complete http:// or https:// URL.")?;
        if !matches!(u.scheme(), "http" | "https")
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
            || text.chars().any(char::is_whitespace)
        {
            return Err("Use an HTTP(S) address without credentials, query, fragment or spaces.");
        }
        if s != Step::Endpoint
            && u.scheme() == "http"
            && !matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        {
            return Err("Use HTTPS for Vessel, except for explicit loopback development.");
        }
    }
    if s == Step::Folder
        && !(text.starts_with('/')
            || (text.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && text.as_bytes().get(1) == Some(&b':')
                && text
                    .as_bytes()
                    .get(2)
                    .is_some_and(|c| matches!(c, b'/' | b'\\'))))
    {
        return Err("Use an absolute example folder, such as /srv/work or C:\\work.");
    }
    Ok(())
}
