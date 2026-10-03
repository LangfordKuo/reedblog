use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// 统一错误类型，对应契约：{"error": {"code": "<snake_case>", "message": "..."}}
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    /// 附加响应头（如 429 的 `Retry-After`）；契约「反滥用」条款
    pub headers: Vec<(&'static str, String)>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            message: message.into(),
            headers: Vec::new(),
        }
    }

    /// 追加响应头（构建期链式调用；值非法时忽略）
    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    /// 评论/留言限流（契约「反滥用」）：429 + `Retry-After`。
    /// 文案不透露具体规则（阈值/剩余额度细节只体现在 Retry-After 秒数上）
    pub fn too_many_requests(retry_after_secs: u64) -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            "提交过于频繁，请稍后再试",
        )
        .with_header("Retry-After", retry_after_secs.to_string())
    }

    /// 后台登录失败退避（契约「反滥用」）：429 + `Retry-After`
    pub fn too_many_attempts(retry_after_secs: u64) -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_attempts",
            "尝试次数过多，请稍后再试",
        )
        .with_header("Retry-After", retry_after_secs.to_string())
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "资源不存在")
    }

    pub fn not_installed() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_installed",
            "站点尚未安装，请先完成安装向导",
        )
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            message,
        )
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "未授权或登录已过期",
        )
    }

    pub fn invalid_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "用户名或密码错误",
        )
    }

    pub fn conflict(code: &str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        });
        let mut resp = (self.status, Json(body)).into_response();
        for (name, value) in &self.headers {
            if let (Ok(name), Ok(value)) =
                (name.parse::<HeaderName>(), HeaderValue::from_str(value))
            {
                resp.headers_mut().insert(name, value);
            }
        }
        resp
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        eprintln!("[reedblog] sqlx 错误: {e}");
        ApiError::internal("数据库错误")
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        eprintln!("[reedblog] IO 错误: {e}");
        ApiError::internal("文件读写错误")
    }
}

/// JSON 请求体解析失败统一映射为 422 validation_error（契约要求的错误形状）
impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        ApiError::validation(format!("请求体格式错误或缺少必填字段: {r}"))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// 请求体提取结果别名：handler 中用 `let Json(body) = body.map_err(ApiError::from)?;`
pub type ValidJson<T> = Result<Json<T>, JsonRejection>;
