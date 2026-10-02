//! Resolve a named credential at each native HTTP dispatch, not adapter creation.
//! No Debug/serialization: these values must never become public diagnostics.
use super::ProviderError;
use voyage_protocol::accounts::AccountBinding;

pub(super) enum ApiCredential {
    Legacy(String),
    XaiAccount {
        registry: crate::accounts::Registry,
        binding: AccountBinding,
        authority: Option<std::sync::Arc<dyn crate::policy::ExecutionAuthority>>,
        redactor: Option<std::sync::Arc<crate::tools::Redactor>>,
    },
    Account {
        binding: AccountBinding,
        authority: Option<std::sync::Arc<dyn crate::policy::ExecutionAuthority>>,
        redactor: Option<std::sync::Arc<crate::tools::Redactor>>,
    },
}
impl ApiCredential {
    pub(super) fn resolve(&self) -> Result<String, ProviderError> {
        match self {
            Self::Legacy(value) => Ok(value.clone()),
            Self::XaiAccount {
                registry,
                binding,
                authority,
                redactor,
            } => {
                super::check_provider_authority(authority)?;
                let tokens = registry.xai_load(binding).map_err(|_| {
                    ProviderError::Authentication(
                        "SuperGrok account unavailable or refresh pending".into(),
                    )
                })?;
                super::xai_oauth::validate_tokens(&tokens)?;
                if let Some(redactor) = redactor {
                    for value in [
                        &tokens.access_token,
                        &tokens.refresh_token,
                        &tokens.identity,
                    ]
                    .into_iter()
                    .chain(tokens.id_token.iter())
                    {
                        redactor.remember_credential(value).map_err(|_| {
                            ProviderError::Authentication(
                                "SuperGrok credential redaction unavailable".into(),
                            )
                        })?;
                    }
                }
                Ok(tokens.access_token)
            }
            Self::Account {
                binding,
                authority,
                redactor,
            } => {
                super::check_provider_authority(authority)?;
                crate::accounts::Registry::default_host()
                    .and_then(|registry| registry.resolve_api_key(binding))
                    .and_then(|key| {
                        if let Some(redactor) = redactor {
                            redactor.remember_credential(&key)?;
                        }
                        Ok(key)
                    })
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
