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
use crate::media;
use crate::models::UploadResult;
use crate::state::{require_pool, AppState};
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

/// 按图片头解析像素宽高（尽力而为，不引入 image 依赖：四种格式的头解析手写）。
/// 解析失败、尺寸非正 → None——上传/扫描侧一律存 NULL，不因此报错。
pub fn image_dimensions(kind: ImageKind, bytes: &[u8]) -> Option<(u32, u32)> {
    let dims = match kind {
        ImageKind::Png => png_dimensions(bytes),
        ImageKind::Jpeg => jpeg_dimensions(bytes),
        ImageKind::Gif => gif_dimensions(bytes),
        ImageKind::Webp => webp_dimensions(bytes),
    };
    dims.filter(|(w, h)| *w > 0 && *h > 0)
}

fn be_u16(b: &[u8]) -> u32 {
    u16::from_be_bytes([b[0], b[1]]) as u32
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn le_u16(b: &[u8]) -> u32 {
    u16::from_le_bytes([b[0], b[1]]) as u32
}

/// PNG：8 字节签名 + 4 字节长度 + "IHDR" 后紧跟宽高（大端 u32）
fn png_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 {
        return None;
    }
    Some((be_u32(&b[16..20]), be_u32(&b[20..24])))
}

/// GIF：6 字节签名/版本 + 逻辑屏宽高（小端 u16）
fn gif_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 10 {
        return None;
    }
    Some((le_u16(&b[6..8]), le_u16(&b[8..10])))
}

/// JPEG：扫描到 SOF0..SOF15（排除 DHT/JPG/DAC）段取高宽（大端 u16）。
/// 正常文件 SOF 必在 SOS（熵数据）之前；解析不到返回 None。
fn jpeg_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2usize;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        match marker {
            0xFF => {
                i += 1; // 填充字节
                continue;
            }
            0x00 | 0x01 | 0xD0..=0xD9 => {
                i += 2; // 无载荷标记（含 RSTn/SOI/EOI）
                continue;
            }
            _ => {}
        }
        let len = be_u16(&b[i + 2..i + 4]) as usize;
        if len < 2 {
            return None;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if i + 9 > b.len() {
                return None;
            }
            let height = be_u16(&b[i + 5..i + 7]);
            let width = be_u16(&b[i + 7..i + 9]);
            return Some((width, height));
        }
        i += 2 + len;
    }
    None
}

/// WebP：RIFF/WEBP 容器内按首个 chunk 变体解析（VP8X 扩展 / VP8L 无损 / VP8 有损）
fn webp_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 20 {
        return None;
    }
    let chunk = &b[12..16];
    let payload = &b[20..];
    if chunk == &b"VP8X"[..] {
        // 10 字节载荷：第 4..7 / 7..10 字节为 宽-1 / 高-1（24 位小端）
        if payload.len() < 10 {
            return None;
        }
        let w = 1 + (payload[4] as u32 | (payload[5] as u32) << 8 | (payload[6] as u32) << 16);
        let h = 1 + (payload[7] as u32 | (payload[8] as u32) << 8 | (payload[9] as u32) << 16);
        return Some((w, h));
    }
    if chunk == &b"VP8L"[..] {
        // 载荷首字节 0x2F + 4 字节位域：宽低 14 位，高次 14 位（各加 1）
        if payload.len() < 5 || payload[0] != 0x2F {
            return None;
        }
        let bits = payload[1] as u32
            | (payload[2] as u32) << 8
            | (payload[3] as u32) << 16
            | (payload[4] as u32) << 24;
        return Some((1 + (bits & 0x3FFF), 1 + ((bits >> 14) & 0x3FFF)));
    }
    if chunk == &b"VP8 "[..] {
        // 3 字节帧标签 + 3 字节同步码 0x9d 0x01 0x2a + 14 位宽/高（小端）
        if payload.len() < 10 || payload[3..6] != [0x9d, 0x01, 0x2a] {
            return None;
        }
        return Some((le_u16(&payload[6..8]) & 0x3FFF, le_u16(&payload[8..10]) & 0x3FFF));
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

/// 词法清洗相对路径（GET /api/uploads/*path 与媒体删除共用）：拒绝绝对路径、`..`、
/// 反斜杠与盘符（%2e%2e 等解码后同样被拦截）；空路径返回 None。
pub fn sanitize_upload_rel(rel: &str) -> Option<PathBuf> {
    let rel = rel.replace('\\', "/");
    let mut full = PathBuf::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains(':') {
            return None;
        }
        full.push(seg);
    }
    if full.as_os_str().is_empty() {
        None
    } else {
        Some(full)
    }
}

/// POST /api/admin/uploads（multipart file=图片）→ 200 UploadResult
pub async fn admin_upload_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<UploadResult>> {
    check_auth(&state, &headers).await?;
    let max = state.uploads_max_size_bytes();

    let mut mp = multipart
        .map_err(|_| ApiError::validation("需要 multipart/form-data，字段 file = 图片文件"))?;
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
    let (filename, data) = uploaded.ok_or_else(|| ApiError::validation("缺少 file 字段"))?;

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
    // 宽高尽力解析（失败存 NULL，不影响上传成功）
    let (width, height) = match image_dimensions(kind, &data) {
        Some((w, h)) => (Some(w as i64), Some(h as i64)),
        None => (None, None),
    };
    let (pool, db_type) = require_pool(&state).await?;

    // 内容哈希去重：同内容已存在（任意年月目录）直接复用已有 URL，不重复落盘
    let url = if let Some(existing) = find_existing_by_hash(state.uploads_dir(), &hash16) {
        format!("/api/uploads/{existing}")
    } else {
        let rel_dir = chrono::Utc::now().format("%Y/%m").to_string();
        let dir = state.uploads_dir().join(&rel_dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(&stored_name), &data)?;
        format!("/api/uploads/{rel_dir}/{stored_name}")
    };

    // media 记录 upsert（契约「媒体库」条款）：同 URL 只应有一行，重复上传返回既有 id
    let id = media::upsert_media_url(
        &pool,
        &db_type,
        &url,
        &display_name,
        data.len() as i64,
        kind.mime(),
        width,
        height,
    )
    .await?;

    Ok(Json(UploadResult {
        id,
        url,
        size: data.len() as u64,
        filename: display_name,
    }))
}

/// GET /api/uploads/*path → 静态图片（公开；不存在/路径非法一律 404，防目录穿越）
pub async fn serve_upload(State(state): State<AppState>, Path(rel): Path<String>) -> Response {
    // 词法清洗（同 themes assets）：拒绝绝对路径、`..`、反斜杠与盘符（%2e%2e 等解码后同样被拦截）
    let Some(full) = sanitize_upload_rel(&rel) else {
        return StatusCode::NOT_FOUND.into_response();
    };

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
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
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
            detect_image_kind(
                b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>"
            ),
            None
        );
        assert_eq!(detect_image_kind(b"<?xml version='1.0'?><svg/>"), None);
        // 只有前 7 个字节的伪 PNG 头不放行
        assert_eq!(
            detect_image_kind(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a]),
            None
        );
    }

    #[test]
    fn hash16_is_lowercase_hex() {
        let h = sha256_hex16(b"hello");
        assert_eq!(h.len(), 16);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // 同内容同哈希（去重基础）
        assert_eq!(h, sha256_hex16(b"hello"));
        assert_ne!(h, sha256_hex16(b"hello!"));
    }

    // ---------- 图片宽高解析（媒体库契约：解析失败 → NULL） ----------

    #[test]
    fn parses_png_dimensions() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&3u32.to_be_bytes());
        png.extend_from_slice(&2u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert_eq!(png_dimensions(&png), Some((3, 2)));
        assert_eq!(image_dimensions(ImageKind::Png, &png), Some((3, 2)));
        // 只有 magic 头 + 零填充的伪 PNG：尺寸为 0 → None（存 NULL）
        assert_eq!(image_dimensions(ImageKind::Png, &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(image_dimensions(ImageKind::Png, b"short"), None);
    }

    #[test]
    fn parses_gif_dimensions() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&10u16.to_le_bytes());
        gif.extend_from_slice(&20u16.to_le_bytes());
        assert_eq!(gif_dimensions(&gif), Some((10, 20)));
        assert_eq!(image_dimensions(ImageKind::Gif, b"GIF89a"), None);
    }

    #[test]
    fn parses_jpeg_dimensions() {
        let mut jpeg = vec![0xff, 0xd8]; // SOI
        jpeg.extend_from_slice(&[0xff, 0xe0, 0x00, 0x10]); // APP0，长度 16
        jpeg.extend_from_slice(&[0u8; 14]); // APP0 载荷
        jpeg.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]); // SOF0，长度 17，精度 8
        jpeg.extend_from_slice(&5u16.to_be_bytes()); // 高
        jpeg.extend_from_slice(&6u16.to_be_bytes()); // 宽
        jpeg.extend_from_slice(&[0u8; 8]);
        assert_eq!(image_dimensions(ImageKind::Jpeg, &jpeg), Some((6, 5)));
        // 截断在 SOF 内 → None
        assert_eq!(jpeg_dimensions(&jpeg[..12]), None);
    }

    #[test]
    fn parses_webp_dimensions() {
        // VP8X：24 位小端 宽-1/高-1
        let mut vp8x = b"RIFF".to_vec();
        vp8x.extend_from_slice(&0u32.to_le_bytes());
        vp8x.extend_from_slice(b"WEBP");
        vp8x.extend_from_slice(b"VP8X");
        vp8x.extend_from_slice(&10u32.to_le_bytes());
        vp8x.extend_from_slice(&[0, 0, 0, 0]);
        vp8x.extend_from_slice(&99u32.to_le_bytes()[..3]); // 宽-1 = 99
        vp8x.extend_from_slice(&49u32.to_le_bytes()[..3]); // 高-1 = 49
        assert_eq!(image_dimensions(ImageKind::Webp, &vp8x), Some((100, 50)));

        // VP8 有损：同步码 + 14 位宽高
        let mut vp8 = b"RIFF".to_vec();
        vp8.extend_from_slice(&0u32.to_le_bytes());
        vp8.extend_from_slice(b"WEBP");
        vp8.extend_from_slice(b"VP8 ");
        vp8.extend_from_slice(&10u32.to_le_bytes());
        vp8.extend_from_slice(&[0, 0, 0, 0x9d, 0x01, 0x2a]);
        vp8.extend_from_slice(&320u16.to_le_bytes());
        vp8.extend_from_slice(&240u16.to_le_bytes());
        assert_eq!(image_dimensions(ImageKind::Webp, &vp8), Some((320, 240)));
    }

    // ---------- 路径词法清洗（读取/删除共用，防穿越） ----------

    #[test]
    fn sanitize_rejects_traversal() {
        assert_eq!(
            sanitize_upload_rel("2026/10/ab.png"),
            Some(std::path::PathBuf::from("2026/10/ab.png"))
        );
        // 反斜杠按分隔符处理后再清洗；`..`、盘符、绝对路径一律拒绝
        assert_eq!(sanitize_upload_rel("..\\secret.txt"), None);
        assert_eq!(sanitize_upload_rel("2026/../../secret.txt"), None);
        assert_eq!(sanitize_upload_rel("C:/windows/win.ini"), None);
        assert_eq!(sanitize_upload_rel("/etc/passwd"), Some(std::path::PathBuf::from("etc/passwd")));
        // 空路径与纯 `.` 段 → None
        assert_eq!(sanitize_upload_rel(""), None);
        assert_eq!(sanitize_upload_rel("./."), None);
    }
}
