//! 图片上传 API（契约「图片上传」条款）：
//! - POST /api/admin/uploads（Bearer；multipart 字段 file）→ 200 UploadResult
//!   magic bytes 判型（png/jpeg/gif/webp，svg 一律拒绝）、大小上限 [uploads] max_size_mb、
//!   sha256 前 16 位 hex 命名（不含用户输入，天然防穿越）、内容哈希去重
//! - GET /api/uploads/*path → 公开静态读取（Cache-Control immutable，防目录穿越，同 themes assets）

use axum::extract::multipart::{Field, MultipartRejection};
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::error::{ApiError, ApiResult};
use crate::models::UploadResult;
use crate::state::AppState;
use crate::themes;

use super::helpers::check_auth;

/// magic bytes 判定出的真实图片类型（规范扩展名 + Content-Type）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageKind {
    /// 落盘用的规范扩展名（由真实类型决定，与上传文件名无关）
    pub fn ext(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
            ImageKind::Gif => "gif",
            ImageKind::Webp => "webp",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ImageKind::Png => "image/png",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Gif => "image/gif",
            ImageKind::Webp => "image/webp",
        }
    }
}

/// 按文件头 magic bytes 判定真实图片类型；不信任扩展名与 Content-Type。
/// svg 不在允许列表（XSS 风险），与其他非图片内容一样返回 None。
pub fn detect_image_kind(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some(ImageKind::Png);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(ImageKind::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(ImageKind::Gif);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes[8..12] == *b"WEBP" {
        return Some(ImageKind::Webp);
    }
    None
}

fn invalid_file_type() -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_file_type",
        "仅支持 PNG / JPEG / GIF / WebP 图片（按文件内容判定；SVG 不允许）",
    )
}

fn file_too_large(max: usize) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "file_too_large",
        format!("文件超出大小上限（{} MB）", max / (1024 * 1024)),
    )
}

/// 流式读取字段内容；累计超过 max 立即中止 → 422 file_too_large。
/// 请求体总限（DefaultBodyLimit = max + 1MB multipart 余量）先触发时同样映射为 file_too_large。
async fn read_limited(mut field: Field<'_>, max: usize) -> ApiResult<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        match field.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len() + chunk.len() > max {
                    return Err(file_too_large(max));
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(buf),
            Err(e) => {
                let msg = e.to_string();
                // 请求体总限溢出（文件远超上限时先于逐块计数触发）也归为 file_too_large
                if msg.contains("length limit") || msg.contains("too large") {
                    return Err(file_too_large(max));
                }
                return Err(ApiError::validation(format!("读取上传内容失败: {e}")));
            }
        }
    }
}

/// sha256 前 16 位 hex（文件名，不含任何用户输入）
fn sha256_hex16(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let full: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    full[..16].to_string()
}

/// 内容哈希去重：在 uploads/*/*/ 下查找 <hash16>.<ext> 已存在的文件，
/// 返回其相对 URL 路径（正斜杠，如 "2026/10/ab12….png"）；多个月份命中取字典序最小（最早）。
fn find_existing_by_hash(uploads: &std::path::Path, hash16: &str) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    let years = std::fs::read_dir(uploads).ok()?;
    for year in years.flatten() {
        let year_path = year.path();
        if !year_path.is_dir() {
            continue;
        }
        let Some(year_name) = year_path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(months) = std::fs::read_dir(&year_path) else {
            continue;
        };
        for month in months.flatten() {
            let month_path = month.path();
            if !month_path.is_dir() {
                continue;
            }
            let Some(month_name) = month_path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(files) = std::fs::read_dir(&month_path) else {
                continue;
            };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                // 文件名形如 <hash16>.<ext>
                if name.len() > hash16.len() + 1
                    && name.starts_with(hash16)
                    && name.as_bytes()[hash16.len()] == b'.'
                    && f.path().is_file()
                {
                    found.push(format!("{year_name}/{month_name}/{name}"));
                }
            }
        }
    }
    found.sort();
    found.into_iter().next()
}

/// POST /api/admin/uploads（multipart file=图片）→ 200 UploadResult
pub async fn admin_upload_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<UploadResult>> {
    check_auth(&state, &headers).await?;
    let max = state.uploads_max_size_bytes();

    let mut mp = multipart.map_err(|_| {
        ApiError::validation("需要 multipart/form-data，字段 file = 图片文件")
    })?;
    let mut uploaded: Option<(String, Vec<u8>)> = None;
    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| ApiError::validation(format!("multipart 解析失败: {e}")))?
    {
        if field.name() == Some("file") {
            let filename = field.file_name().unwrap_or_default().to_string();
            let bytes = read_limited(field, max).await?;
            uploaded = Some((filename, bytes));
            break;
        }
    }
    let (filename, data) =
        uploaded.ok_or_else(|| ApiError::validation("缺少 file 字段"))?;

    // 真实类型按 magic bytes 判定；扩展名/Content-Type 一律不信任
    let kind = detect_image_kind(&data).ok_or_else(invalid_file_type)?;
    let hash16 = sha256_hex16(&data);
    let stored_name = format!("{hash16}.{}", kind.ext());
    // filename 仅回显：原始文件名为空时用生成的文件名兜底
    let display_name = if filename.trim().is_empty() {
        stored_name.clone()
    } else {
        filename
    };

    // 内容哈希去重：同内容已存在（任意年月目录）直接返回已有 URL，不重复落盘
    if let Some(existing) = find_existing_by_hash(state.uploads_dir(), &hash16) {
        return Ok(Json(UploadResult {
            url: format!("/api/uploads/{existing}"),
            size: data.len() as u64,
            filename: display_name,
        }));
    }

    let rel_dir = chrono::Utc::now().format("%Y/%m").to_string();
    let dir = state.uploads_dir().join(&rel_dir);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(&stored_name), &data)?;

    Ok(Json(UploadResult {
        url: format!("/api/uploads/{rel_dir}/{stored_name}"),
        size: data.len() as u64,
        filename: display_name,
    }))
}

/// GET /api/uploads/*path → 静态图片（公开；不存在/路径非法一律 404，防目录穿越）
pub async fn serve_upload(
    State(state): State<AppState>,
    Path(rel): Path<String>,
) -> Response {
    if rel.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // 词法清洗（同 themes assets）：拒绝绝对路径、`..`、反斜杠与盘符（%2e%2e 等解码后同样被拦截）
    let rel = rel.replace('\\', "/");
    let mut full = PathBuf::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains(':') {
            return StatusCode::NOT_FOUND.into_response();
        }
        full.push(seg);
    }
    if full.as_os_str().is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let root = state.uploads_dir().to_path_buf();
    let target = root.join(&full);
    // 物理校验：canonicalize 后必须仍在 uploads 根目录内（防符号链接等残余手段）
    let (Ok(root_canon), Ok(target_canon)) = (root.canonicalize(), target.canonicalize()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !target_canon.starts_with(root_canon) || !target_canon.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(&target_canon) {
        Ok(body) => (
            [
                (header::CONTENT_TYPE, themes::mime_for(&rel)),
                (
                    header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable",
                ),
            ],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_magic_bytes() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];
        assert_eq!(detect_image_kind(&png), Some(ImageKind::Png));
        assert_eq!(ImageKind::Png.ext(), "png");

        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 0];
        assert_eq!(detect_image_kind(&jpeg), Some(ImageKind::Jpeg));
        assert_eq!(ImageKind::Jpeg.ext(), "jpg");

        assert_eq!(detect_image_kind(b"GIF87a...."), Some(ImageKind::Gif));
        assert_eq!(detect_image_kind(b"GIF89a...."), Some(ImageKind::Gif));

        let mut webp = Vec::new();
        webp.extend_from_slice(b"RIFF");
        webp.extend_from_slice(&[0x10, 0, 0, 0]);
        webp.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(detect_image_kind(&webp), Some(ImageKind::Webp));
        // RIFF 但非 WEBP（如 wav）不放行
        let mut wav = webp.clone();
        wav[8..12].copy_from_slice(b"WAVE");
        assert_eq!(detect_image_kind(&wav), None);
    }

    #[test]
    fn rejects_non_images_and_svg() {
        assert_eq!(detect_image_kind(b""), None);
        assert_eq!(detect_image_kind(b"plain text renamed to .png"), None);
        assert_eq!(
            detect_image_kind(b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>"),
            None
        );
        assert_eq!(detect_image_kind(b"<?xml version='1.0'?><svg/>"), None);
        // 只有前 7 个字节的伪 PNG 头不放行
        assert_eq!(detect_image_kind(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a]), None);
    }

    #[test]
    fn hash16_is_lowercase_hex() {
        let h = sha256_hex16(b"hello");
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // 同内容同哈希（去重基础）
        assert_eq!(h, sha256_hex16(b"hello"));
        assert_ne!(h, sha256_hex16(b"hello!"));
    }
}
