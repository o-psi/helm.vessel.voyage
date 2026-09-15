use super::*;
#[tokio::test]
async fn empty_resource_cleanup_is_idempotent_and_fences_extension_admission() {
    for resources in [ManagedResources::default(), ManagedResources::local()] {
        assert!(!resources.has_owned_work().unwrap());
        resources
            .own_extensions(Arc::new(crate::extensions::runtime::Manager::default()))
            .unwrap();
        let retained = resources.close().unwrap();
        assert!(retained.closed);
        assert_eq!(retained.extensions.len(), 1);
        assert!(
            resources
                .own_extensions(Arc::new(crate::extensions::runtime::Manager::default()))
                .is_err()
        );
        resources.shutdown_observed(false).await.unwrap();
        resources.shutdown_observed(true).await.unwrap();
        assert!(!resources.has_owned_work().unwrap());
    }
}
