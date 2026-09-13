//! Authored checkpoint diagnostics; underlying errors and paths stay private.
#[derive(Clone, Copy)]
pub(super) enum Operation {
    RuntimeState,
    ProviderAttempt,
    WorkingContext,
    CanonicalHistory,
    Acceptance,
    ToolPreview,
    PartialText,
}
impl Operation {
    fn label(self) -> &'static str {
        match self {
            Self::RuntimeState => "runtime state",
            Self::ProviderAttempt => "provider attempt",
            Self::WorkingContext => "working context",
            Self::CanonicalHistory => "canonical history",
            Self::Acceptance => "acceptance",
            Self::ToolPreview => "tool preview",
            Self::PartialText => "partial text",
        }
    }
    pub(super) fn requires_authority(self) -> bool {
        matches!(
            self,
            Self::WorkingContext | Self::CanonicalHistory | Self::Acceptance
        )
    }
}
const OPERATIONS: &[Operation] = &[
    Operation::RuntimeState,
    Operation::ProviderAttempt,
    Operation::WorkingContext,
    Operation::CanonicalHistory,
    Operation::Acceptance,
    Operation::ToolPreview,
    Operation::PartialText,
];
const CATEGORIES: &[&str] = &[
    "database busy",
    "storage error",
    "state or authority validation failed",
];

pub(super) fn reason(operation: Operation, error: &anyhow::Error) -> String {
    let category = match error.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(code, _))
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            CATEGORIES[0]
        }
        Some(_) => CATEGORIES[1],
        None if error.downcast_ref::<std::io::Error>().is_some() => CATEGORIES[1],
        None => CATEGORIES[2],
    };
    let operation = operation.label();
    format!("Checkpoint failed during {operation}: {category}.")
}

pub(super) fn is_public_reason(reason: &str) -> bool {
    OPERATIONS.iter().any(|operation| {
        let operation = operation.label();
        CATEGORIES
            .iter()
            .any(|category| reason == format!("Checkpoint failed during {operation}: {category}."))
    })
}
