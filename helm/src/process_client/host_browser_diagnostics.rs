//! Fixed viewer observations. No website data, transport error text or receipts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Startup,
    SocketChanged,
    DispatchUnknown,
    DispatchRefused,
    ControlExchange,
    ServerEnded,
    ServerRetirement,
    PrivateCleanup,
}
impl Failure {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Startup => "viewer_startup_unavailable",
            Self::SocketChanged => "viewer_socket_changed",
            Self::DispatchUnknown => "viewer_dispatch_unknown",
            Self::DispatchRefused => "viewer_dispatch_refused",
            Self::ControlExchange => "viewer_control_exchange_failed",
            Self::ServerEnded => "viewer_server_ended",
            Self::ServerRetirement => "viewer_server_retirement_failed",
            Self::PrivateCleanup => "viewer_private_cleanup_failed",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Diagnostic {
    pub failure: Option<Failure>,
    pub local_cleanup_observed: bool,
    pub detach_reply_observed: Option<bool>,
}
impl Diagnostic {
    pub(super) fn fail(&mut self, failure: Failure) {
        self.failure.get_or_insert(failure);
    }
    pub(super) fn summary(&self) -> String {
        format!(
            "Viewer diagnostic: {}; local cleanup {}; detach reply {}. Operation outcomes require exact receipts; unknown effects are not replayed; host browser is not implicitly closed",
            self.failure.map_or("none", Failure::code),
            if self.local_cleanup_observed {
                "observed"
            } else {
                "not observed"
            },
            match self.detach_reply_observed {
                Some(true) => "observed",
                Some(false) => "not observed",
                None => "not attempted",
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_codes_and_independent_cleanup_do_not_claim_receipts() {
        for failure in [
            Failure::Startup,
            Failure::SocketChanged,
            Failure::DispatchUnknown,
            Failure::DispatchRefused,
            Failure::ControlExchange,
            Failure::ServerEnded,
            Failure::ServerRetirement,
            Failure::PrivateCleanup,
        ] {
            let diagnostic = Diagnostic {
                failure: Some(failure),
                local_cleanup_observed: true,
                detach_reply_observed: Some(false),
            };
            let summary = diagnostic.summary();
            assert!(summary.contains(failure.code()));
            assert!(summary.contains("local cleanup observed"));
            assert!(summary.contains("detach reply not observed"));
            assert!(summary.contains("require exact receipts"));
        }
        assert!(Diagnostic::default().summary().contains("not attempted"));
        let mut first = Diagnostic::default();
        first.fail(Failure::DispatchUnknown);
        first.fail(Failure::SocketChanged);
        assert_eq!(first.failure, Some(Failure::DispatchUnknown));
    }
}
