use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::HeaderMap;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use mes_application::IdentityRepository;
use serde::Deserialize;
use tokio::sync::RwLock;
use uuid::Uuid;

const DEV_ISSUER: &str = "urn:agentic-mes:dev";
const MAX_TOKEN_BYTES: usize = 16 * 1024;
const JWKS_CACHE_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    DevHeaders,
    Oidc,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedPrincipal {
    pub user_id: Uuid,
    pub external_identity_id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub tenant_id: Uuid,
    pub permitted_factory_ids: Vec<Uuid>,
    pub roles: Vec<String>,
    pub capabilities: Vec<String>,
    pub auth_method: AuthMethod,
}

impl AuthenticatedPrincipal {
    #[must_use]
    pub fn factory_id(&self) -> Uuid {
        self.permitted_factory_ids[0]
    }
}

#[derive(Clone, Copy, Debug)]
pub enum AuthError {
    Unauthenticated,
    Forbidden,
    Unavailable,
}

#[derive(Clone)]
pub struct IdentityService {
    provider: AuthenticationProvider,
    repository: Arc<dyn IdentityRepository>,
}

#[derive(Clone)]
enum AuthenticationProvider {
    DevHeaders,
    Oidc(Arc<OidcProvider>),
}

#[derive(Debug)]
struct VerifiedSubject {
    issuer: String,
    subject: String,
    tenant_id: Uuid,
    factory_id: Uuid,
    method: AuthMethod,
}

impl IdentityService {
    pub fn development(repository: Arc<dyn IdentityRepository>) -> Self {
        Self {
            provider: AuthenticationProvider::DevHeaders,
            repository,
        }
    }

    /// Builds an OIDC-backed identity service and primes its trusted key cache.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid configuration, failed discovery, or an
    /// unavailable/invalid initial JWKS document.
    pub async fn oidc(
        repository: Arc<dyn IdentityRepository>,
        issuer: String,
        audience: String,
        jwks_override: Option<String>,
    ) -> Result<Self, String> {
        let provider = OidcProvider::new(issuer, audience, jwks_override).await?;
        Ok(Self {
            provider: AuthenticationProvider::Oidc(Arc::new(provider)),
            repository,
        })
    }

    #[must_use]
    pub fn provider_name(&self) -> &'static str {
        match self.provider {
            AuthenticationProvider::DevHeaders => "dev_headers",
            AuthenticationProvider::Oidc(_) => "oidc",
        }
    }

    pub async fn is_ready(&self) -> bool {
        match &self.provider {
            AuthenticationProvider::DevHeaders => true,
            AuthenticationProvider::Oidc(provider) => {
                provider.cache.read().await.as_ref().is_some_and(|cache| {
                    cache.expires_at > Instant::now() && !cache.keys.keys.is_empty()
                })
            }
        }
    }

    /// Verifies the selected credential and resolves local authorization data.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed authentication, authorization, or provider error.
    pub async fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> Result<AuthenticatedPrincipal, AuthError> {
        let subject = match &self.provider {
            AuthenticationProvider::DevHeaders => dev_subject(headers)?,
            AuthenticationProvider::Oidc(provider) => provider.verify(headers).await?,
        };
        let access = self
            .repository
            .resolve_identity(
                &subject.issuer,
                &subject.subject,
                subject.tenant_id,
                subject.factory_id,
            )
            .await
            .map_err(|_| AuthError::Unavailable)?
            .ok_or(AuthError::Forbidden)?;
        let capabilities = capabilities(&access.roles);
        Ok(AuthenticatedPrincipal {
            user_id: access.user_id,
            external_identity_id: access.external_identity_id,
            issuer: subject.issuer,
            subject: subject.subject,
            tenant_id: access.tenant_id,
            permitted_factory_ids: access.permitted_factory_ids,
            roles: access.roles,
            capabilities,
            auth_method: subject.method,
        })
    }
}

fn dev_subject(headers: &HeaderMap) -> Result<VerifiedSubject, AuthError> {
    let tenant_id = header_uuid(headers, "x-dev-tenant-id")?;
    let factory_id = header_uuid(headers, "x-dev-factory-id")?;
    let subject = headers
        .get("x-dev-user-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("dev-user")
        .trim();
    if subject.is_empty() || subject.len() > 128 {
        return Err(AuthError::Unauthenticated);
    }
    Ok(VerifiedSubject {
        issuer: DEV_ISSUER.to_owned(),
        subject: subject.to_owned(),
        tenant_id,
        factory_id,
        method: AuthMethod::DevHeaders,
    })
}

fn header_uuid(headers: &HeaderMap, name: &'static str) -> Result<Uuid, AuthError> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(AuthError::Unauthenticated)
}

fn capabilities(roles: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut add = |value: &str| {
        if !result.iter().any(|v| v == value) {
            result.push(value.to_owned());
        }
    };
    for role in roles {
        match role.as_str() {
            "viewer" => add("read:mes"),
            "production_analyst" => {
                add("read:mes");
                add("run:agent");
                add("read:agent_runs:self");
            }
            "supervisor" => {
                add("read:mes");
                add("run:agent");
                add("read:agent_runs:self");
                add("read:agent_runs:tenant");
            }
            "admin" => {
                add("read:mes");
                add("run:agent");
                add("read:agent_runs:self");
                add("read:agent_runs:tenant");
                add("manage:identity");
            }
            _ => {}
        }
    }
    result
}

struct CachedKeys {
    keys: JwkSet,
    expires_at: Instant,
}

struct OidcProvider {
    issuer: String,
    audience: String,
    jwks_url: String,
    client: reqwest::Client,
    cache: RwLock<Option<CachedKeys>>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    exp: usize,
    nbf: Option<usize>,
    iat: Option<i64>,
}

impl OidcProvider {
    async fn new(
        issuer: String,
        audience: String,
        override_url: Option<String>,
    ) -> Result<Self, String> {
        if issuer.trim().is_empty() || audience.trim().is_empty() {
            return Err("OIDC issuer and audience must be non-empty".to_owned());
        }
        if !issuer.starts_with("https://") {
            return Err("OIDC_ISSUER must use HTTPS".to_owned());
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| "cannot build OIDC HTTP client".to_owned())?;
        let jwks_url = if let Some(url) = override_url {
            url
        } else {
            let url = format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            );
            let discovery: Discovery = client
                .get(url)
                .send()
                .await
                .map_err(|_| "OIDC discovery failed".to_owned())?
                .error_for_status()
                .map_err(|_| "OIDC discovery failed".to_owned())?
                .json()
                .await
                .map_err(|_| "invalid OIDC discovery document".to_owned())?;
            if discovery.issuer != issuer {
                return Err("OIDC discovery issuer mismatch".to_owned());
            }
            discovery.jwks_uri
        };
        if !jwks_url.starts_with("https://") {
            return Err("OIDC JWKS URL must use HTTPS".to_owned());
        }
        let provider = Self {
            issuer,
            audience,
            jwks_url,
            client,
            cache: RwLock::new(None),
        };
        provider
            .refresh_keys()
            .await
            .map_err(|_| "initial OIDC JWKS retrieval failed".to_owned())?;
        Ok(provider)
    }

    async fn verify(&self, headers: &HeaderMap) -> Result<VerifiedSubject, AuthError> {
        let mut authorization_values = headers.get_all("authorization").iter();
        let raw = authorization_values
            .next()
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthError::Unauthenticated)?;
        if authorization_values.next().is_some() {
            return Err(AuthError::Unauthenticated);
        }
        if raw.contains(',') || !raw.starts_with("Bearer ") {
            return Err(AuthError::Unauthenticated);
        }
        let token = &raw[7..];
        if token.is_empty()
            || token.len() > MAX_TOKEN_BYTES
            || token.chars().any(char::is_whitespace)
        {
            return Err(AuthError::Unauthenticated);
        }
        let header = decode_header(token).map_err(|_| AuthError::Unauthenticated)?;
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::Unauthenticated);
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|v| !v.is_empty())
            .ok_or(AuthError::Unauthenticated)?;
        let mut jwk = self.key(kid).await;
        if jwk.is_none() {
            self.refresh_keys().await?;
            jwk = self.key(kid).await;
        }
        let jwk = jwk.ok_or(AuthError::Unauthenticated)?;
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| AuthError::Unauthenticated)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 60;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| AuthError::Unauthenticated)?
            .claims;
        if claims.iss != self.issuer || claims.sub.is_empty() || claims.sub.len() > 255 {
            return Err(AuthError::Unauthenticated);
        }
        let now = chrono::Utc::now().timestamp();
        if claims.iat.is_some_and(|iat| iat > now + 60) {
            return Err(AuthError::Unauthenticated);
        }
        let _ = (claims.exp, claims.nbf);
        Ok(VerifiedSubject {
            issuer: claims.iss,
            subject: claims.sub,
            tenant_id: header_uuid(headers, "x-tenant-id")?,
            factory_id: header_uuid(headers, "x-factory-id")?,
            method: AuthMethod::Oidc,
        })
    }

    async fn key(&self, kid: &str) -> Option<jsonwebtoken::jwk::Jwk> {
        let cache = self.cache.read().await;
        cache
            .as_ref()
            .filter(|c| c.expires_at > Instant::now())
            .and_then(|c| c.keys.find(kid).cloned())
    }

    async fn refresh_keys(&self) -> Result<(), AuthError> {
        let keys: JwkSet = self
            .client
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|_| AuthError::Unavailable)?
            .error_for_status()
            .map_err(|_| AuthError::Unavailable)?
            .json()
            .await
            .map_err(|_| AuthError::Unavailable)?;
        if keys.keys.is_empty() {
            return Err(AuthError::Unavailable);
        }
        *self.cache.write().await = Some(CachedKeys {
            keys,
            expires_at: Instant::now() + JWKS_CACHE_TTL,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::http::{HeaderMap, HeaderValue};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use jsonwebtoken::{EncodingKey, Header, encode, jwk::JwkSet};
    use mes_application::{ApplicationError, IdentityAccess, IdentityRepository};
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
    use serde::Serialize;
    use uuid::Uuid;

    use super::{
        AuthError, AuthMethod, AuthenticationProvider, CachedKeys, IdentityService, JWKS_CACHE_TTL,
        OidcProvider, capabilities,
    };

    struct FixtureRepository {
        tenant_id: Uuid,
        factory_id: Uuid,
    }

    #[async_trait]
    impl IdentityRepository for FixtureRepository {
        async fn resolve_identity(
            &self,
            issuer: &str,
            subject: &str,
            tenant_id: Uuid,
            factory_id: Uuid,
        ) -> Result<Option<IdentityAccess>, ApplicationError> {
            if issuer != "https://issuer.example/"
                || subject != "known-user"
                || tenant_id != self.tenant_id
                || factory_id != self.factory_id
            {
                return Ok(None);
            }
            Ok(Some(IdentityAccess {
                user_id: Uuid::new_v4(),
                external_identity_id: Uuid::new_v4(),
                tenant_id,
                permitted_factory_ids: vec![factory_id],
                roles: vec!["production_analyst".to_owned()],
            }))
        }
    }

    #[derive(Serialize)]
    struct TestClaims<'a> {
        iss: &'a str,
        sub: &'a str,
        aud: &'a str,
        exp: i64,
        iat: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        nbf: Option<i64>,
    }

    fn signing_material(kid: &str) -> (EncodingKey, JwkSet) {
        let private = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)
            .expect("test RSA key generation must succeed");
        let public = private.to_public_key();
        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "kid": kid,
                "use": "sig",
                "alg": "RS256",
                "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
                "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be())
            }]
        }))
        .expect("test JWKS must deserialize");
        let der = private
            .to_pkcs1_der()
            .expect("test private key must encode");
        let encoding = EncodingKey::from_rsa_der(der.as_bytes());
        (encoding, jwks)
    }

    fn fixture() -> (IdentityService, EncodingKey, Arc<OidcProvider>, Uuid, Uuid) {
        let (encoding, jwks) = signing_material("test-key");
        let provider = Arc::new(OidcProvider {
            issuer: "https://issuer.example/".to_owned(),
            audience: "agentic-mes".to_owned(),
            jwks_url: "http://127.0.0.1:1/jwks".to_owned(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(100))
                .build()
                .expect("test client must build"),
            cache: tokio::sync::RwLock::new(Some(CachedKeys {
                keys: jwks,
                expires_at: std::time::Instant::now() + JWKS_CACHE_TTL,
            })),
        });
        let tenant_id = Uuid::new_v4();
        let factory_id = Uuid::new_v4();
        let repository = Arc::new(FixtureRepository {
            tenant_id,
            factory_id,
        });
        (
            IdentityService {
                provider: AuthenticationProvider::Oidc(provider.clone()),
                repository,
            },
            encoding,
            provider,
            tenant_id,
            factory_id,
        )
    }

    fn headers(token: &str, tenant_id: Uuid, factory_id: Uuid) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).expect("header must be valid"),
        );
        headers.insert(
            "x-tenant-id",
            HeaderValue::from_str(&tenant_id.to_string()).expect("UUID header must be valid"),
        );
        headers.insert(
            "x-factory-id",
            HeaderValue::from_str(&factory_id.to_string()).expect("UUID header must be valid"),
        );
        headers
    }

    #[allow(clippy::too_many_arguments)]
    fn token_with(
        key: &EncodingKey,
        kid: Option<&str>,
        issuer: &str,
        audience: &str,
        subject: &str,
        exp: i64,
        iat: i64,
        nbf: Option<i64>,
    ) -> String {
        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = kid.map(ToOwned::to_owned);
        encode(
            &header,
            &TestClaims {
                iss: issuer,
                sub: subject,
                aud: audience,
                exp,
                iat,
                nbf,
            },
            key,
        )
        .expect("test token must encode")
    }

    fn token(key: &EncodingKey, audience: &str, subject: &str) -> String {
        let now = chrono::Utc::now().timestamp();
        token_with(
            key,
            Some("test-key"),
            "https://issuer.example/",
            audience,
            subject,
            now + 300,
            now,
            None,
        )
    }

    #[tokio::test]
    async fn oidc_token_builds_locally_authorized_principal() {
        let (service, key, _provider, tenant_id, factory_id) = fixture();
        let principal = service
            .authenticate(&headers(
                &token(&key, "agentic-mes", "known-user"),
                tenant_id,
                factory_id,
            ))
            .await
            .expect("valid token and membership must authenticate");
        assert_eq!(principal.auth_method, AuthMethod::Oidc);
        assert_eq!(principal.permitted_factory_ids, vec![factory_id]);
        assert!(principal.capabilities.iter().any(|v| v == "run:agent"));
    }

    #[tokio::test]
    async fn oidc_rejects_wrong_audience_and_unknown_local_subject() {
        let (service, key, _provider, tenant_id, factory_id) = fixture();
        let wrong_audience = service
            .authenticate(&headers(
                &token(&key, "other-service", "known-user"),
                tenant_id,
                factory_id,
            ))
            .await;
        assert!(matches!(wrong_audience, Err(AuthError::Unauthenticated)));

        let unknown = service
            .authenticate(&headers(
                &token(&key, "agentic-mes", "unknown-user"),
                tenant_id,
                factory_id,
            ))
            .await;
        assert!(matches!(unknown, Err(AuthError::Forbidden)));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn oidc_rejects_time_issuer_signature_algorithm_and_kid_attacks() {
        let (service, key, _provider, tenant_id, factory_id) = fixture();
        let now = chrono::Utc::now().timestamp();
        assert!(matches!(
            service
                .authenticate(&headers("not-a-jwt", tenant_id, factory_id))
                .await,
            Err(AuthError::Unauthenticated)
        ));
        for attacked in [
            token_with(
                &key,
                Some("test-key"),
                "https://issuer.example/",
                "agentic-mes",
                "known-user",
                now - 120,
                now - 300,
                None,
            ),
            token_with(
                &key,
                Some("test-key"),
                "https://issuer.example/",
                "agentic-mes",
                "known-user",
                now + 300,
                now,
                Some(now + 120),
            ),
            token_with(
                &key,
                Some("test-key"),
                "https://issuer.example/",
                "agentic-mes",
                "known-user",
                now + 300,
                now + 120,
                None,
            ),
            token_with(
                &key,
                Some("test-key"),
                "https://wrong-issuer.example/",
                "agentic-mes",
                "known-user",
                now + 300,
                now,
                None,
            ),
            token_with(
                &key,
                None,
                "https://issuer.example/",
                "agentic-mes",
                "known-user",
                now + 300,
                now,
                None,
            ),
        ] {
            assert!(matches!(
                service
                    .authenticate(&headers(&attacked, tenant_id, factory_id))
                    .await,
                Err(AuthError::Unauthenticated)
            ));
        }

        let valid = token(&key, "agentic-mes", "known-user");
        let mut parts: Vec<String> = valid.split('.').map(ToOwned::to_owned).collect();
        let replacement = if parts[2].starts_with('A') { "B" } else { "A" };
        parts[2].replace_range(0..1, replacement);
        let modified = parts.join(".");
        assert!(matches!(
            service
                .authenticate(&headers(&modified, tenant_id, factory_id))
                .await,
            Err(AuthError::Unauthenticated)
        ));
        let failure = service
            .authenticate(&headers(&modified, tenant_id, factory_id))
            .await;
        assert!(!format!("{failure:?}").contains(&modified));

        let mut symmetric_header = Header::new(jsonwebtoken::Algorithm::HS256);
        symmetric_header.kid = Some("test-key".to_owned());
        let symmetric = encode(
            &symmetric_header,
            &TestClaims {
                iss: "https://issuer.example/",
                sub: "known-user",
                aud: "agentic-mes",
                exp: now + 300,
                iat: now,
                nbf: None,
            },
            &EncodingKey::from_secret(b"test-only-secret"),
        )
        .expect("symmetric attack token must encode");
        assert!(matches!(
            service
                .authenticate(&headers(&symmetric, tenant_id, factory_id))
                .await,
            Err(AuthError::Unauthenticated)
        ));

        let none_header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","kid":"test-key"}"#);
        let none_payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&TestClaims {
                iss: "https://issuer.example/",
                sub: "known-user",
                aud: "agentic-mes",
                exp: now + 300,
                iat: now,
                nbf: None,
            })
            .expect("none payload must encode"),
        );
        let none_token = format!("{none_header}.{none_payload}.");
        assert!(matches!(
            service
                .authenticate(&headers(&none_token, tenant_id, factory_id))
                .await,
            Err(AuthError::Unauthenticated)
        ));

        let valid = token(&key, "agentic-mes", "known-user");
        let mut duplicated = headers(&valid, tenant_id, factory_id);
        duplicated.append(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {valid}"))
                .expect("duplicate authorization header must build"),
        );
        assert!(matches!(
            service.authenticate(&duplicated).await,
            Err(AuthError::Unauthenticated)
        ));
    }

    #[tokio::test]
    async fn oidc_unknown_kid_unavailable_jwks_and_dev_fallback_fail_closed() {
        let (service, key, provider, tenant_id, factory_id) = fixture();
        let now = chrono::Utc::now().timestamp();
        let unknown_kid = token_with(
            &key,
            Some("unknown-key"),
            "https://issuer.example/",
            "agentic-mes",
            "known-user",
            now + 300,
            now,
            None,
        );
        assert!(matches!(
            service
                .authenticate(&headers(&unknown_kid, tenant_id, factory_id))
                .await,
            Err(AuthError::Unavailable)
        ));

        provider
            .cache
            .write()
            .await
            .as_mut()
            .expect("cache exists")
            .expires_at = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .expect("one second before now must be representable");
        assert!(matches!(
            service
                .authenticate(&headers(
                    &token(&key, "agentic-mes", "known-user"),
                    tenant_id,
                    factory_id
                ))
                .await,
            Err(AuthError::Unavailable)
        ));

        let mut dev_only = HeaderMap::new();
        dev_only.insert(
            "x-dev-tenant-id",
            HeaderValue::from_str(&tenant_id.to_string()).expect("UUID header"),
        );
        dev_only.insert(
            "x-dev-factory-id",
            HeaderValue::from_str(&factory_id.to_string()).expect("UUID header"),
        );
        dev_only.insert("x-dev-user-id", HeaderValue::from_static("dev-user"));
        assert!(matches!(
            service.authenticate(&dev_only).await,
            Err(AuthError::Unauthenticated)
        ));
    }

    #[tokio::test]
    async fn oidc_key_rotation_accepts_new_key_and_rejects_old_key() {
        let (service, old_key, provider, tenant_id, factory_id) = fixture();
        let (new_key, new_jwks) = signing_material("rotated-key");
        *provider.cache.write().await = Some(CachedKeys {
            keys: new_jwks,
            expires_at: std::time::Instant::now() + JWKS_CACHE_TTL,
        });
        let now = chrono::Utc::now().timestamp();
        let rotated = token_with(
            &new_key,
            Some("rotated-key"),
            "https://issuer.example/",
            "agentic-mes",
            "known-user",
            now + 300,
            now,
            None,
        );
        assert!(
            service
                .authenticate(&headers(&rotated, tenant_id, factory_id))
                .await
                .is_ok()
        );

        let old = token(&old_key, "agentic-mes", "known-user");
        assert!(matches!(
            service
                .authenticate(&headers(&old, tenant_id, factory_id))
                .await,
            Err(AuthError::Unavailable)
        ));
    }

    #[test]
    fn frozen_roles_expand_only_to_their_documented_capabilities() {
        assert_eq!(capabilities(&["viewer".to_owned()]), vec!["read:mes"]);
        assert_eq!(
            capabilities(&["production_analyst".to_owned()]),
            vec!["read:mes", "run:agent", "read:agent_runs:self"]
        );
        assert!(
            capabilities(&["supervisor".to_owned()])
                .iter()
                .any(|value| value == "read:agent_runs:tenant")
        );
        let admin = capabilities(&["admin".to_owned()]);
        assert!(admin.iter().any(|value| value == "manage:identity"));
        assert!(
            !admin
                .iter()
                .any(|value| value == "read:evaluation_datasets")
        );
    }
}
