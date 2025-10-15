//! Ed25519 signature-based authentication for HTTP requests
//!
//! Provides server-side verification of Ed25519-signed requests using the
//! X-ADIC-PublicKey, X-ADIC-Timestamp, and X-ADIC-Signature headers.
//!
//! # Security
//!
//! - Full cryptographic verification using ed25519-dalek
//! - Timestamp-based replay protection (configurable window)
//! - Public key-based identity verification
//!
//! # Usage
//!
//! ```rust,ignore
//! use axum::{Router, routing::get};
//! use adic_node::auth_ed25519::{Ed25519Auth, Ed25519AuthConfig};
//!
//! async fn protected_endpoint(auth: Ed25519Auth) -> &'static str {
//!     // auth.public_key contains the verified public key
//!     "Authenticated!"
//! }
//!
//! let app = Router::new()
//!     .route("/protected", get(protected_endpoint))
//!     .layer(axum::Extension(Ed25519AuthConfig::default()));
//! ```

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use ed25519_dalek::{Signature as DalekSignature, Verifier, VerifyingKey};
use std::time::{SystemTime, UNIX_EPOCH};

/// Ed25519 authentication configuration
#[derive(Clone, Debug)]
pub struct Ed25519AuthConfig {
    /// Whether Ed25519 auth is enabled
    pub enabled: bool,
    /// Maximum age of timestamp in seconds (for replay protection)
    pub max_timestamp_age: i64,
    /// Allow timestamps slightly in the future (clock skew tolerance)
    pub allow_future_seconds: i64,
}

impl Default for Ed25519AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_timestamp_age: 300, // 5 minutes
            allow_future_seconds: 60, // 1 minute clock skew
        }
    }
}

impl Ed25519AuthConfig {
    /// Create configuration with custom timestamp window
    pub fn with_timestamp_window(max_age: i64, future_tolerance: i64) -> Self {
        Self {
            enabled: true,
            max_timestamp_age: max_age,
            allow_future_seconds: future_tolerance,
        }
    }

    /// Create configuration with auth disabled
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            max_timestamp_age: 300,
            allow_future_seconds: 60,
        }
    }
}

/// Ed25519 authentication errors
#[derive(Debug)]
pub enum Ed25519AuthError {
    MissingHeader(&'static str),
    InvalidHex(String),
    InvalidPublicKey(String),
    InvalidSignature,
    InvalidTimestamp(String),
    TimestampTooOld,
    TimestampInFuture,
    InternalError,
}

impl IntoResponse for Ed25519AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Ed25519AuthError::MissingHeader(header) => {
                (StatusCode::UNAUTHORIZED, format!("Missing header: {}", header))
            }
            Ed25519AuthError::InvalidHex(err) => {
                (StatusCode::BAD_REQUEST, format!("Invalid hex encoding: {}", err))
            }
            Ed25519AuthError::InvalidPublicKey(err) => {
                (StatusCode::BAD_REQUEST, format!("Invalid public key: {}", err))
            }
            Ed25519AuthError::InvalidSignature => {
                (StatusCode::UNAUTHORIZED, "Signature verification failed".to_string())
            }
            Ed25519AuthError::InvalidTimestamp(err) => {
                (StatusCode::BAD_REQUEST, format!("Invalid timestamp: {}", err))
            }
            Ed25519AuthError::TimestampTooOld => (
                StatusCode::UNAUTHORIZED,
                "Request timestamp too old (possible replay attack)".to_string(),
            ),
            Ed25519AuthError::TimestampInFuture => (
                StatusCode::UNAUTHORIZED,
                "Request timestamp in future (clock skew)".to_string(),
            ),
            Ed25519AuthError::InternalError => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal error".to_string())
            }
        };

        (status, message).into_response()
    }
}

/// Authenticated request with Ed25519 signature
///
/// Extract this in Axum handlers to enforce Ed25519 authentication.
/// The public key can be used to identify the caller.
pub struct Ed25519Auth {
    /// The verified public key of the requester
    pub public_key: [u8; 32],
    /// The timestamp from the request
    pub timestamp: i64,
}

impl Ed25519Auth {
    /// Get the public key as a hex string
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key)
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for Ed25519Auth
where
    S: Send + Sync,
{
    type Rejection = Ed25519AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Get config from extensions
        let config = parts
            .extensions
            .get::<Ed25519AuthConfig>()
            .cloned()
            .unwrap_or_default();

        // If disabled, skip verification (for development)
        if !config.enabled {
            return Ok(Ed25519Auth {
                public_key: [0u8; 32],
                timestamp: 0,
            });
        }

        // Extract required headers
        let public_key_hex = parts
            .headers
            .get("X-ADIC-PublicKey")
            .and_then(|h| h.to_str().ok())
            .ok_or(Ed25519AuthError::MissingHeader("X-ADIC-PublicKey"))?;

        let timestamp_str = parts
            .headers
            .get("X-ADIC-Timestamp")
            .and_then(|h| h.to_str().ok())
            .ok_or(Ed25519AuthError::MissingHeader("X-ADIC-Timestamp"))?;

        let signature_hex = parts
            .headers
            .get("X-ADIC-Signature")
            .and_then(|h| h.to_str().ok())
            .ok_or(Ed25519AuthError::MissingHeader("X-ADIC-Signature"))?;

        // Parse timestamp
        let timestamp: i64 = timestamp_str
            .parse()
            .map_err(|_| Ed25519AuthError::InvalidTimestamp("Failed to parse timestamp".to_string()))?;

        // Validate timestamp (replay protection)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Ed25519AuthError::InternalError)?
            .as_secs() as i64;

        let age = now - timestamp;

        if age > config.max_timestamp_age {
            return Err(Ed25519AuthError::TimestampTooOld);
        }

        if age < -config.allow_future_seconds {
            return Err(Ed25519AuthError::TimestampInFuture);
        }

        // Decode public key and signature
        let pubkey_bytes = hex::decode(public_key_hex)
            .map_err(|e| Ed25519AuthError::InvalidHex(e.to_string()))?;

        let sig_bytes = hex::decode(signature_hex)
            .map_err(|e| Ed25519AuthError::InvalidHex(e.to_string()))?;

        if pubkey_bytes.len() != 32 {
            return Err(Ed25519AuthError::InvalidPublicKey(format!(
                "Expected 32 bytes, got {}",
                pubkey_bytes.len()
            )));
        }

        if sig_bytes.len() != 64 {
            return Err(Ed25519AuthError::InvalidPublicKey(format!(
                "Expected 64-byte signature, got {}",
                sig_bytes.len()
            )));
        }

        // Create verifying key
        let mut pubkey_array = [0u8; 32];
        pubkey_array.copy_from_slice(&pubkey_bytes);

        let verifying_key = VerifyingKey::from_bytes(&pubkey_array)
            .map_err(|e| Ed25519AuthError::InvalidPublicKey(e.to_string()))?;

        // Create signature
        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(&sig_bytes);

        let signature = DalekSignature::from_bytes(&sig_array);

        // Reconstruct signed message: method || URI || body || timestamp
        let method = parts.method.as_str();
        let uri = parts.uri.to_string();

        // Note: Body is not available in FromRequestParts
        // For body verification, use a custom middleware
        // For now, we verify: method || uri || timestamp
        let mut message = Vec::new();
        message.extend_from_slice(method.as_bytes());
        message.extend_from_slice(uri.as_bytes());
        // Body would go here in full implementation
        message.extend_from_slice(&timestamp.to_le_bytes());

        // Verify signature
        verifying_key
            .verify(&message, &signature)
            .map_err(|_| Ed25519AuthError::InvalidSignature)?;

        tracing::debug!(
            public_key = public_key_hex,
            timestamp,
            method = method,
            uri = %uri,
            "✅ Ed25519 signature verified"
        );

        Ok(Ed25519Auth {
            public_key: pubkey_array,
            timestamp,
        })
    }
}

/// Middleware to inject Ed25519 auth config into requests
pub async fn ed25519_auth_middleware(
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    // Inject default config if not present
    if req.extensions().get::<Ed25519AuthConfig>().is_none() {
        req.extensions_mut().insert(Ed25519AuthConfig::default());
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_crypto::Keypair;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Extension, Router,
    };
    use tower::ServiceExt;

    async fn protected_handler(_auth: Ed25519Auth) -> &'static str {
        "Success"
    }

    async fn unprotected_handler() -> &'static str {
        "No auth required"
    }

    fn create_signed_request(
        keypair: &Keypair,
        method: &str,
        uri: &str,
        timestamp: i64,
    ) -> Request<Body> {
        // Build message: method || uri || timestamp
        let mut message = Vec::new();
        message.extend_from_slice(method.as_bytes());
        message.extend_from_slice(uri.as_bytes());
        message.extend_from_slice(&timestamp.to_le_bytes());

        let signature = keypair.sign(&message);

        Request::builder()
            .method(method)
            .uri(uri)
            .header("X-ADIC-PublicKey", hex::encode(keypair.public_key().as_bytes()))
            .header("X-ADIC-Timestamp", timestamp.to_string())
            .header("X-ADIC-Signature", hex::encode(signature.as_bytes()))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_ed25519_auth_valid_signature() {
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(Extension(Ed25519AuthConfig::default()));

        let keypair = Keypair::generate();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let request = create_signed_request(&keypair, "GET", "/protected", now);

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_ed25519_auth_invalid_signature() {
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(Extension(Ed25519AuthConfig::default()));

        let keypair = Keypair::generate();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create request with tampered signature
        let mut request = create_signed_request(&keypair, "GET", "/protected", now);
        *request.headers_mut().get_mut("X-ADIC-Signature").unwrap() =
            "0".repeat(128).parse().unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_ed25519_auth_missing_header() {
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(Extension(Ed25519AuthConfig::default()));

        let request = Request::builder()
            .method("GET")
            .uri("/protected")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_ed25519_auth_timestamp_too_old() {
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(Extension(Ed25519AuthConfig::default()));

        let keypair = Keypair::generate();
        let old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 400; // 400 seconds ago (> 300 second default window)

        let request = create_signed_request(&keypair, "GET", "/protected", old_timestamp);

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_ed25519_auth_timestamp_in_future() {
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(Extension(Ed25519AuthConfig::default()));

        let keypair = Keypair::generate();
        let future_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 120; // 2 minutes in future (> 60 second tolerance)

        let request = create_signed_request(&keypair, "GET", "/protected", future_timestamp);

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_ed25519_auth_disabled() {
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(Extension(Ed25519AuthConfig::disabled()));

        // No headers at all
        let request = Request::builder()
            .method("GET")
            .uri("/protected")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_ed25519_auth_wrong_public_key() {
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(Extension(Ed25519AuthConfig::default()));

        let keypair1 = Keypair::generate();
        let keypair2 = Keypair::generate();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Sign with keypair1 but use keypair2's public key
        let mut message = Vec::new();
        message.extend_from_slice(b"GET");
        message.extend_from_slice(b"/protected");
        message.extend_from_slice(&now.to_le_bytes());

        let signature = keypair1.sign(&message);

        let request = Request::builder()
            .method("GET")
            .uri("/protected")
            .header("X-ADIC-PublicKey", hex::encode(keypair2.public_key().as_bytes()))
            .header("X-ADIC-Timestamp", now.to_string())
            .header("X-ADIC-Signature", hex::encode(signature.as_bytes()))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
