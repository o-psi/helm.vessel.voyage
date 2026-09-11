//! Resolve a named credential at each native HTTP dispatch, not adapter creation.
//! No Debug/serialization: these values must never become public diagnostics.
use super::ProviderError;
use voyage_protocol::accounts::AccountBinding;

pub(super) enum ApiCredential {
    Legacy(String),
    Account {
        binding: AccountBinding,
        authority: Option<std::sync::Arc<dyn crate::policy::ExecutionAuthority>>,
    },
}
impl ApiCredential {
    pub(super) fn resolve(&self) -> Result<String, ProviderError> {
        match self {
            Self::Legacy(value) => Ok(value.clone()),
            Self::Account { binding, authority } => {
                super::check_provider_authority(authority)?;
                crate::accounts::Registry::default_host()
                    .and_then(|registry| registry.resolve_api_key(binding))
                    // Storage errors must not disclose credential or host paths.
                    .map_err(|_| {
                        ProviderError::Authentication(
                            "selected account unavailable or changed; review account selection"
                                .into(),
                        )
                    })
            }
        }
    }
}
