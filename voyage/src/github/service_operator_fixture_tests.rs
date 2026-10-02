//! Owned loopback construction retains production authority and owner validation.
use super::*;

impl Service {
    pub(in crate::github) fn for_operator_test(
        context: ToolContext,
        session: Option<Uuid>,
        client: Client,
        directory: PathBuf,
    ) -> Result<Self> {
        let mut service = Self::new(context, session)?;
        service.client = client.with_policy(service.context.policy.clone());
        service.directory = directory;
        Ok(service)
    }
}
