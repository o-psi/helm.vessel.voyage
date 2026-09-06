//! Local GitHub context and exact attended publication.
pub mod context;
pub mod operator;
pub mod admin;
pub mod approval;
pub mod tool;

/// Supported diagnostic projections for the explicitly delegated credential.
/// This is redaction, not a claim to detect arbitrary encodings or exfiltration.
pub fn credential_redactions(config: &crate::Config) -> Vec<String> {
    let token = config.env.get("HELM_GITHUB_TOKEN").cloned().or_else(|| {
        config.inherit_env.iter().any(|name|name == "HELM_GITHUB_TOKEN").then(||std::env::var("HELM_GITHUB_TOKEN").ok()).flatten()
    });
    token.map(|token|credential_forms(&token)).unwrap_or_default()
}
pub(crate) fn credential_forms(token: &str) -> Vec<String> {
    use base64::Engine;
    if token.len() < 4 { return Vec::new(); }
    vec![token.to_owned(),base64::engine::general_purpose::STANDARD.encode(token),base64::engine::general_purpose::STANDARD_NO_PAD.encode(token),base64::engine::general_purpose::URL_SAFE.encode(token),base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token),hex::encode(token),hex::encode_upper(token)]
}
pub mod publication;
pub mod repository;
pub mod service;
pub mod store;
mod transport;

#[cfg(test)]
mod credential_tests {
    #[test]
    fn explicit_configured_github_credentials_redact_supported_forms() {
        let mut config = crate::Config::default();
        let token = "fixture-github-secret-9876";
        config.env.insert("HELM_GITHUB_TOKEN".into(),token.into());
        let forms = super::credential_redactions(&config);
        let redactor = crate::tools::Redactor::new(forms.clone());
        for form in forms {
            assert_eq!(redactor.redact(format!("before {form} after")),"before [REDACTED] after");
            assert!(redactor.contains_secret(&form));
        }
        let mut undelegated = crate::Config::default();
        undelegated.env.insert("GH_TOKEN".into(),token.into());
        assert!(super::credential_redactions(&undelegated).is_empty());
    }
}
