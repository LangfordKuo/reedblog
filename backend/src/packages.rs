//! zip 包解压/校验与 slug、semver、文件系统时间戳等通用工具（插件、主题安装共用）。

use rand::RngCore;
use std::cmp::Ordering;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::error::ApiError;

/// 上传 zip 解压后的总体积上限（防 zip 炸弹）
const MAX_UNCOMPRESSED: u64 = 64 * 1024 * 1024;
/// 上传 zip 内的条目数上限
const MAX_ENTRIES: usize = 4096;

fn invalid_package(msg: impl Into<String>) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_package",
        msg,
    )
}

/// slug 合法性：^[a-z0-9-]+$（另加 128 长度上限，避免文件系统问题）
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 128
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// 解析 semver 核心三元组（忽略 -prerelease / +build 后缀）
fn semver_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// semver 合法性（宽松：核心必须是 X.Y.Z 数字；后缀不校验）
pub fn valid_semver(v: &str) -> bool {
    semver_core(v).is_some()
}

/// semver 比较（仅核心三元组；解析失败按 0.0.0 处理）
pub fn compare_semver(a: &str, b: &str) -> Ordering {
    let fa = semver_core(a).unwrap_or((0, 0, 0));
    let fb = semver_core(b).unwrap_or((0, 0, 0));
    fa.cmp(&fb)
}

/// 目录/文件的 (installed_at, updated_at)：取文件系统 created/modified 时间，
/// 失败回退当前时间。RFC3339 UTC 秒精度，与全库时间戳格式一致。
pub fn fs_timestamps(path: &Path) -> (String, String) {
    let fallback = crate::state::now_rfc3339();
    let to_rfc = |t: std::io::Result<std::time::SystemTime>| -> Option<String> {
        t.ok().map(|st| {
            chrono::DateTime::<chrono::Utc>::from(st)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
    };
    match std::fs::metadata(path) {
        Ok(meta) => (
            to_rfc(meta.created()).unwrap_or_else(|| fallback.clone()),
            to_rfc(meta.modified()).unwrap_or(fallback),
        ),
        Err(_) => (fallback.clone(), fallback),
    }
}

/// 随机十六进制后缀（临时 staging 目录名用）
fn rand_hex() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 在 parent 下创建一次性 staging 目录（.tmp-<hex>），解压校验用；调用方负责删除
pub fn make_staging_dir(parent: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".tmp-{}", rand_hex()));
    std::fs::create_dir_all(&staging)?;
    Ok(staging)
}

/// 校验并解压「根层单目录」的 zip 到 staging，返回根目录名。
/// - zip 非法/结构不符/路径不安全（绝对路径、`..`、盘符、符号链接）→ 422 invalid_package
/// - 体积与条目数设上限，防 zip 炸弹
pub fn extract_single_root_zip(data: &[u8], staging: &Path) -> Result<String, ApiError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| invalid_package(format!("zip 解析失败: {e}")))?;
    if archive.len() == 0 {
        return Err(invalid_package("zip 内容为空"));
    }
    if archive.len() > MAX_ENTRIES {
        return Err(invalid_package("zip 条目数超出上限"));
    }

    let mut root: Option<String> = None;
    let mut total_size: u64 = 0;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| invalid_package(format!("zip 条目读取失败: {e}")))?;
        total_size += file.size();
        if total_size > MAX_UNCOMPRESSED {
            return Err(invalid_package("zip 解压后体积超出上限"));
        }

        // 条目名归一化（zip 规范为正斜杠；部分打包工具写入反斜杠）+ 安全清洗
        let raw = file.name().replace('\\', "/");
        let mut segments: Vec<&str> = Vec::new();
        for seg in raw.split('/') {
            if seg.is_empty() || seg == "." {
                continue;
            }
            if seg == ".." || seg.contains(':') {
                return Err(invalid_package(format!("zip 含非法路径: {raw}")));
            }
            segments.push(seg);
        }
        if segments.is_empty() {
            return Err(invalid_package(format!("zip 含非法路径: {raw}")));
        }

        // 根层必须是单个目录：所有条目共享同一顶层目录名，且顶层条目只能是目录
        match &root {
            None => root = Some(segments[0].to_string()),
            Some(r) if r == segments[0] => {}
            _ => return Err(invalid_package("zip 根层必须是单个目录")),
        }
        if segments.len() == 1 && !file.is_dir() {
            return Err(invalid_package("zip 根层必须是单个目录（发现根层文件）"));
        }

        // 符号链接条目直接跳过（不解压，防指向包外）
        if let Some(mode) = file.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                continue;
            }
        }

        let out = segments
            .iter()
            .fold(staging.to_path_buf(), |p, s| p.join(s));
        if file.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| {
                ApiError::internal(format!("创建目录失败 {}: {e}", out.display()))
            })?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    ApiError::internal(format!("创建目录失败 {}: {e}", parent.display()))
                })?;
            }
            let mut of = std::fs::File::create(&out).map_err(|e| {
                ApiError::internal(format!("写入文件失败 {}: {e}", out.display()))
            })?;
            std::io::copy(&mut file, &mut of).map_err(|e| {
                ApiError::internal(format!("解压文件失败 {}: {e}", out.display()))
            })?;
        }
    }

    let root = root.ok_or_else(|| invalid_package("zip 内容为空"))?;
    if !staging.join(&root).is_dir() {
        return Err(invalid_package("zip 根层必须是单个目录"));
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_validation() {
        assert!(valid_slug("demo-plugin"));
        assert!(valid_slug("a1"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("Demo"));
        assert!(!valid_slug("de_mo"));
        assert!(!valid_slug("../etc"));
        assert!(!valid_slug(&"a".repeat(129)));
    }

    #[test]
    fn semver_validation_and_compare() {
        assert!(valid_semver("1.0.0"));
        assert!(valid_semver("0.1.0-beta.1"));
        assert!(valid_semver("1.2.3+build"));
        assert!(!valid_semver("1.0"));
        assert!(!valid_semver("v1.0.0"));
        assert_eq!(compare_semver("1.0.0", "0.9.9"), Ordering::Greater);
        assert_eq!(compare_semver("0.1.0", "0.1.0"), Ordering::Equal);
        assert_eq!(compare_semver("0.1.0", "0.2.0"), Ordering::Less);
    }
}
