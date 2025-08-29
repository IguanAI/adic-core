use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    RequestPartsExt,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// JWT claims structure
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,  // Subject (user identifier)
    pub exp: u64,     // Expiry time
    pub iat: u64,     // Issued at
    pub role: String, // User role (admin, readonly, etc.)
}

/// Auth configuration
#[derive(Clone)]
pub struct AuthConfig {
    pub enabled: bool,
    pub jwt_secret: String,
    #[allow(dead_code)]
    pub token_expiry: Duration, // Used in _create_token and validate_token
    pub admin_tokens: Vec<String>, // Pre-authorized admin tokens
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: std::env::var("ADIC_API_AUTH_ENABLED")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            jwt_secret: std::env::var("ADIC_JWT_SECRET").unwrap_or_else(|_| {
                // Generate a random secret if not provided
                use rand::Rng;
                let mut rng = rand::thread_rng();
                let secret: [u8; 32] = rng.gen();
                hex::encode(secret)
            }),
            token_expiry: Duration::from_secs(3600), // 1 hour
            admin_tokens: std::env::var("ADIC_ADMIN_TOKENS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl AuthConfig {
    /// Create a new JWT token
    pub fn _create_token(&self, user_id: &str, role: &str) -> Result<String, AuthError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AuthError::InternalError)?
            .as_secs();

        let claims = Claims {
            sub: user_id.to_string(),
            exp: now + self.token_expiry.as_secs(),
            iat: now,
            role: role.to_string(),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_ref()),
        )
        .map_err(|_| AuthError::TokenCreation)
    }

    /// Validate a JWT token
    pub fn validate_token(&self, token: &str) -> Result<Claims, AuthError> {
        // Check if it's a pre-authorized admin token
        if self.admin_tokens.contains(&token.to_string()) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| AuthError::InternalError)?
                .as_secs();

            return Ok(Claims {
                sub: "admin".to_string(),
                exp: now + 3600,
                iat: now,
                role: "admin".to_string(),
            });
        }

        // Otherwise validate as JWT
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_ref()),
            &Validation::default(),
        )
        .map(|data| data.claims)
        .map_err(|_| AuthError::InvalidToken)
    }
}

/// Authentication errors
#[derive(Debug)]
pub enum AuthError {
    InvalidToken,
    MissingToken,
    #[allow(dead_code)]
    TokenCreation,
    InternalError,
    #[allow(dead_code)]
    Unauthorized,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid authentication token"),
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "Missing authentication token"),
            AuthError::TokenCreation => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create token")
            }
            AuthError::InternalError => (StatusCode::INTERNAL_SERVER_ERROR, "Internal error"),
            AuthError::Unauthorized => (StatusCode::FORBIDDEN, "Unauthorized"),
        };

        (status, message).into_response()
    }
}

/// Extractor for authenticated requests
pub struct AuthUser {
    pub _user_id: String,
    pub _role: String,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Get auth config from extensions (should be injected by middleware)
        let auth_config = parts
            .extensions
            .get::<AuthConfig>()
            .ok_or(AuthError::InternalError)?
            .clone(); // Clone to avoid borrow issues

        // Skip auth if not enabled
        if !auth_config.enabled {
            return Ok(AuthUser {
                _user_id: "anonymous".to_string(),
                _role: "admin".to_string(), // Grant full access when auth disabled
            });
        }

        // Extract the token from the Authorization header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::MissingToken)?;

        // Validate the token
        let claims = auth_config.validate_token(bearer.token())?;

        Ok(AuthUser {
            _user_id: claims.sub,
            _role: claims.role,
        })
    }
}

/// Middleware to inject auth config into request extensions
pub async fn auth_middleware(
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    // Inject auth config
    req.extensions_mut().insert(AuthConfig::default());
    next.run(req).await
}

/// Rate limiting structure
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

pub struct RateLimiter {
    limits: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            limits: Arc::new(RwLock::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    pub async fn check_rate_limit(&self, key: &str) -> bool {
        let mut limits = self.limits.write().await;
        let now = Instant::now();

        let requests = limits.entry(key.to_string()).or_insert_with(Vec::new);

        // Remove old requests outside the window
        requests.retain(|&t| now.duration_since(t) < self.window);

        if requests.len() >= self.max_requests {
            false
        } else {
            requests.push(now);
            true
        }
    }
}

/// Rate limiting middleware
pub async fn rate_limit_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    // Get client IP or use a default key
    let key = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("default")
        .to_string();

    // Check rate limit (100 requests per minute by default)
    let limiter = RateLimiter::new(100, Duration::from_secs(60));

    if !limiter.check_rate_limit(&key).await {
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response();
    }

    next.run(req).await
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod auth_tests;
