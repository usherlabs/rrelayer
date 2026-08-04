use axum::http::HeaderMap;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::yaml::{JwtHs256Config, SignatureScheme};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum SignatureVerificationError {
    #[error("Missing signature header `{0}`")]
    MissingSignatureHeader(String),

    #[error("Header `{0}` contains non-ASCII characters")]
    NonAsciiHeader(String),

    #[error(
        "Secret env variable `{0}` is not set in the process environment; \
         cannot verify request signature"
    )]
    SecretEnvMissing(String),

    #[error("Invalid or expired JWT in signature header")]
    InvalidJwt,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AppsmithJwtClaims {
    exp: usize,
    #[serde(rename = "userEmail", default)]
    #[allow(dead_code)]
    user_email: Option<String>,
}

fn lookup_header<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> Result<Option<&'a str>, SignatureVerificationError> {
    match headers.get(name) {
        None => Ok(None),
        Some(v) => v
            .to_str()
            .map(Some)
            .map_err(|_| SignatureVerificationError::NonAsciiHeader(name.to_string())),
    }
}

/// Verify an incoming request against the configured signature scheme.
///
/// Dispatches by `scheme`; each variant carries the data its own verifier
/// needs, so adding a scheme is a self-contained change here.
pub fn verify_request_signature(
    scheme: &SignatureScheme,
    headers: &HeaderMap,
) -> Result<(), SignatureVerificationError> {
    match scheme {
        SignatureScheme::JwtHs256(cfg) => verify_jwt_hs256(cfg, headers),
    }
}

fn verify_jwt_hs256(
    cfg: &JwtHs256Config,
    headers: &HeaderMap,
) -> Result<(), SignatureVerificationError> {
    let header_name = cfg.signature_header();
    let token = lookup_header(headers, header_name)?
        .ok_or_else(|| SignatureVerificationError::MissingSignatureHeader(header_name.to_string()))?
        .trim();

    let secret = std::env::var(&cfg.secret_env)
        .map_err(|_| SignatureVerificationError::SecretEnvMissing(cfg.secret_env.clone()))?;

    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    decode::<AppsmithJwtClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation)
        .map(|_| ())
        .map_err(|_| SignatureVerificationError::InvalidJwt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_SECRET: &str = "super-secret-do-not-use";
    const TEST_HEADER: &str = "x-appsmith-signature";

    /// Build a scheme that points at a unique env var per test (passed in by
    /// the caller) to avoid cross-test interference when tests run in
    /// parallel.
    fn make_scheme(secret_env: &str) -> SignatureScheme {
        SignatureScheme::JwtHs256(JwtHs256Config {
            secret_env: secret_env.to_string(),
            signature_header: Some(TEST_HEADER.to_string()),
        })
    }

    fn put_secret(name: &str) {
        // SAFETY: each test uses a unique env var name to avoid interference
        // between concurrent tests.
        unsafe { std::env::set_var(name, TEST_SECRET) };
    }

    fn now_secs() -> usize {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as usize
    }

    fn token_with_exp(secret: &str, exp: usize) -> String {
        let claims = AppsmithJwtClaims { exp, user_email: Some("ops@usher.so".to_string()) };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
    }

    #[test]
    fn verifies_a_valid_jwt_signature() {
        let env = "RRELAYER_TEST_HMAC_SECRET_VALID";
        put_secret(env);
        let scheme = make_scheme(env);
        let token = token_with_exp(TEST_SECRET, now_secs() + 300);

        let mut headers = HeaderMap::new();
        headers.insert(TEST_HEADER, HeaderValue::from_str(&token).unwrap());

        verify_request_signature(&scheme, &headers).expect("valid token should verify");
    }

    #[test]
    fn rejects_when_secret_env_missing() {
        let scheme = make_scheme("RRELAYER_TEST_HMAC_SECRET_UNSET_DO_NOT_DEFINE");
        let mut headers = HeaderMap::new();
        headers.insert(TEST_HEADER, HeaderValue::from_static("not-a-jwt"));
        let err = verify_request_signature(&scheme, &headers).unwrap_err();
        assert!(matches!(err, SignatureVerificationError::SecretEnvMissing(_)));
    }

    #[test]
    fn rejects_when_signature_header_missing() {
        let env = "RRELAYER_TEST_HMAC_SECRET_NO_SIG";
        put_secret(env);
        let scheme = make_scheme(env);
        let headers = HeaderMap::new();
        let err = verify_request_signature(&scheme, &headers).unwrap_err();
        assert!(matches!(err, SignatureVerificationError::MissingSignatureHeader(_)));
    }

    #[test]
    fn rejects_invalid_jwt() {
        let env = "RRELAYER_TEST_HMAC_SECRET_BAD_TOKEN";
        put_secret(env);
        let scheme = make_scheme(env);
        let mut headers = HeaderMap::new();
        headers.insert(TEST_HEADER, HeaderValue::from_static("not.a.jwt"));
        let err = verify_request_signature(&scheme, &headers).unwrap_err();
        assert!(matches!(err, SignatureVerificationError::InvalidJwt));
    }

    #[test]
    fn rejects_expired_jwt() {
        let env = "RRELAYER_TEST_HMAC_SECRET_EXPIRED";
        put_secret(env);
        let scheme = make_scheme(env);
        let token = token_with_exp(TEST_SECRET, now_secs() - 1);
        let mut headers = HeaderMap::new();
        headers.insert(TEST_HEADER, HeaderValue::from_str(&token).unwrap());
        let err = verify_request_signature(&scheme, &headers).unwrap_err();
        assert!(matches!(err, SignatureVerificationError::InvalidJwt));
    }

    /// Lock in the on-disk YAML shape for `request_verification`. If this
    /// test fails, every operator's policy file needs to be re-indented — so
    /// any change here must be a deliberate breaking change.
    #[test]
    fn signature_scheme_yaml_uses_adjacent_tagging() {
        let yaml = "\
scheme: jwt_hs256
params:
  secret_env: APPSMITH_SIGNATURE_KEY
  signature_header: x-appsmith-signature
";
        let parsed: SignatureScheme = serde_yaml::from_str(yaml).expect("deserialize");
        match &parsed {
            SignatureScheme::JwtHs256(cfg) => {
                assert_eq!(cfg.secret_env, "APPSMITH_SIGNATURE_KEY");
                assert_eq!(cfg.signature_header.as_deref(), Some("x-appsmith-signature"));
            }
        }

        let dumped = serde_yaml::to_string(&parsed).expect("serialize");
        assert!(dumped.contains("scheme: jwt_hs256"), "missing tag: {dumped}");
        assert!(dumped.contains("params:"), "missing params block: {dumped}");
        assert!(
            dumped.contains("secret_env: APPSMITH_SIGNATURE_KEY"),
            "missing secret_env under params: {dumped}"
        );
    }

    #[test]
    fn rejects_jwt_signed_with_wrong_secret() {
        let env = "RRELAYER_TEST_HMAC_SECRET_WRONG";
        put_secret(env);
        let scheme = make_scheme(env);
        let token = token_with_exp("another-secret", now_secs() + 300);
        let mut headers = HeaderMap::new();
        headers.insert(TEST_HEADER, HeaderValue::from_str(&token).unwrap());
        let err = verify_request_signature(&scheme, &headers).unwrap_err();
        assert!(matches!(err, SignatureVerificationError::InvalidJwt));
    }
}
