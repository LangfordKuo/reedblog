//! argon2 密码哈希 + JWT HS256 签发/校验

use crate::error::{ApiError, ApiResult};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::http::HeaderMap;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// JWT 有效期 7 天（契约约定）
const TOKEN_DAYS: i64 = 7;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: i64,
}

/// argon2 哈希密码（随机 16 字节盐）
pub fn hash_password(plain: &str) -> ApiResult<String> {
    let mut salt_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| ApiError::internal(format!("生成盐失败: {e}")))?;
    let hash = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| ApiError::internal(format!("密码哈希失败: {e}")))?
        .to_string();
    Ok(hash)
}

/// 校验密码；哈希串非法或密码不匹配都返回 false
pub fn verify_password(password_hash: &str, plain: &str) -> bool {
    match PasswordHash::new(password_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// 生成随机 JWT secret（32 字节 hex，64 字符）
pub fn generate_jwt_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 签发 token，返回 (token, expires_at RFC3339)
pub fn issue_token(secret: &str, username: &str) -> ApiResult<(String, String)> {
    let expires_at = Utc::now() + Duration::days(TOKEN_DAYS);
    let claims = Claims {
        sub: username.to_string(),
        exp: expires_at.timestamp(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ApiError::internal(format!("签发 token 失败: {e}")))?;
    Ok((
        token,
        expires_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    ))
}

/// 校验 token；无效/过期一律 401 unauthorized
pub fn verify_token(secret: &str, token: &str) -> ApiResult<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .map(|data| data.claims)
    .map_err(|_| ApiError::unauthorized())
}

/// 从 Authorization: Bearer <token> 头提取 token
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value.strip_prefix("Bearer ").map(|s| s.trim())
}

/// 管理接口鉴权：校验 Bearer token，失败即 401
pub fn require_auth(state_secret: Option<String>, headers: &HeaderMap) -> ApiResult<Claims> {
    let secret = state_secret.ok_or_else(ApiError::unauthorized)?;
    let token = bearer_token(headers).ok_or_else(ApiError::unauthorized)?;
    verify_token(&secret, token)
}
