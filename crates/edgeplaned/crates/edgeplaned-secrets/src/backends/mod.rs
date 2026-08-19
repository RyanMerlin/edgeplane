pub mod env;
pub mod infisical;
pub mod literal;

pub use env::EnvBackend;
pub use infisical::InfisicalBackend;
pub use literal::LiteralBackend;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretsBackend;
    use crate::{ResolveCtx, SecretRef};

    #[tokio::test]
    async fn env_backend_resolves_present_var() {
        std::env::set_var("EP_SECRETS_T3_ENV", "val-123");
        let b = EnvBackend::new();
        let r = SecretRef::parse("secret://env/EP_SECRETS_T3_ENV").unwrap();
        let v = b.resolve(&r, &ResolveCtx::default()).await.unwrap();
        assert_eq!(v.expose(), "val-123");
        std::env::remove_var("EP_SECRETS_T3_ENV");
    }

    #[tokio::test]
    async fn env_backend_missing_is_not_found() {
        let b = EnvBackend::new();
        let r = SecretRef::parse("secret://env/EP_SECRETS_T3_ABSENT").unwrap();
        assert!(matches!(
            b.resolve(&r, &ResolveCtx::default()).await,
            Err(crate::BackendError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn literal_backend_resolves_from_map() {
        let mut m = std::collections::HashMap::new();
        m.insert("greeting".to_string(), "hi".to_string());
        let b = LiteralBackend::new(m);
        let r = SecretRef::parse("secret://literal/greeting").unwrap();
        assert_eq!(
            b.resolve(&r, &ResolveCtx::default())
                .await
                .unwrap()
                .expose(),
            "hi"
        );
    }
}
