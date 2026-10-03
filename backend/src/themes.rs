//! 主题系统（docs/extensibility-contract.md 第二部分）：
//! - 主题 = themes/<slug>/（theme.toml 必需 + theme.css / preview.png / assets/ 可选）
//! - 内置 default 主题：themes/default 不存在时自动生成（shadcn neutral 令牌），不可删/停用/覆盖
//! - active 权威来源 = config.toml [themes] active；主题不入 DB
//! - 令牌原样下发；css_url / preview_url 由后端静态托管

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

use crate::packages;

/// 内置主题 slug（builtin:true，不可删除、不可被上传覆盖）
pub const BUILTIN_THEME_SLUG: &str = "default";

/// 内置 default 主题 theme.toml（shadcn neutral 令牌，含暗色变体）
pub const BUILTIN_DEFAULT_THEME_TOML: &str = r#"name = "默认主题"
slug = "default"
version = "1.0.0"
description = "内置简洁浅色主题"
author = "reedblog"

[tokens]
background = "0 0% 100%"
foreground = "0 0% 3.9%"
card = "0 0% 100%"
card_foreground = "0 0% 3.9%"
popover = "0 0% 100%"
popover_foreground = "0 0% 3.9%"
primary = "0 0% 9%"
primary_foreground = "0 0% 98%"
secondary = "0 0% 96.1%"
secondary_foreground = "0 0% 9%"
muted = "0 0% 96.1%"
muted_foreground = "0 0% 45.1%"
accent = "0 0% 96.1%"
accent_foreground = "0 0% 9%"
destructive = "0 84.2% 60.2%"
destructive_foreground = "0 0% 98%"
border = "0 0% 89.8%"
input = "0 0% 89.8%"
ring = "0 0% 3.9%"
radius = "0.5rem"

[tokens_dark]
background = "0 0% 3.9%"
foreground = "0 0% 98%"
card = "0 0% 3.9%"
card_foreground = "0 0% 98%"
popover = "0 0% 3.9%"
popover_foreground = "0 0% 98%"
primary = "0 0% 98%"
primary_foreground = "0 0% 9%"
secondary = "0 0% 14.9%"
secondary_foreground = "0 0% 98%"
muted = "0 0% 14.9%"
muted_foreground = "0 0% 63.9%"
accent = "0 0% 14.9%"
accent_foreground = "0 0% 98%"
destructive = "0 62.8% 30.6%"
destructive_foreground = "0 0% 98%"
border = "0 0% 14.9%"
input = "0 0% 14.9%"
ring = "0 0% 83.1%"
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeManifest {
    pub name: String,
    pub slug: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// 设计令牌：key 为 CSS 变量名去 -- 前缀的下划线形式；原样下发（未知 key 保留，前端忽略）
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
    #[serde(default)]
    pub tokens_dark: Option<BTreeMap<String, String>>,
}

impl ThemeManifest {
    /// 校验 theme.toml（dir_name 为 zip/磁盘上的目录名）
    pub fn validate(&self, dir_name: &str) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("name 不能为空".into());
        }
        if !packages::valid_slug(&self.slug) {
            return Err(format!("slug '{}' 非法（须匹配 ^[a-z0-9-]+$）", self.slug));
        }
        if self.slug != dir_name {
            return Err(format!(
                "slug '{}' 必须与目录名 '{}' 一致",
                self.slug, dir_name
            ));
        }
        if !packages::valid_semver(&self.version) {
            return Err(format!("version '{}' 不是合法 semver", self.version));
        }
        Ok(())
    }
}

/// 契约 ThemeInfo 形状
#[derive(Debug, Clone, Serialize)]
pub struct ThemeInfo {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub active: bool,
    pub builtin: bool,
    pub has_css: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    pub installed_at: String,
    pub updated_at: String,
}

/// 首次运行/升级启动时补建内置 default 主题（幂等：已存在则不动，不可被覆盖）
pub fn ensure_default_theme(themes_dir: &Path) {
    let dir = themes_dir.join(BUILTIN_THEME_SLUG);
    if dir.join("theme.toml").is_file() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[reedblog] 创建默认主题目录失败: {e}");
        return;
    }
    if let Err(e) = std::fs::write(dir.join("theme.toml"), BUILTIN_DEFAULT_THEME_TOML) {
        eprintln!("[reedblog] 写入默认主题 theme.toml 失败: {e}");
    }
    // 不生成 theme.css（has_css=false）
}

/// 读取并解析某主题的 theme.toml；不存在/非法 → None
pub fn load_manifest(themes_dir: &Path, slug: &str) -> Option<ThemeManifest> {
    if !packages::valid_slug(slug) {
        return None;
    }
    let text = std::fs::read_to_string(theme_dir(themes_dir, slug).join("theme.toml")).ok()?;
    toml::from_str(&text).ok()
}

pub fn theme_dir(themes_dir: &Path, slug: &str) -> std::path::PathBuf {
    themes_dir.join(slug)
}

pub fn theme_exists(themes_dir: &Path, slug: &str) -> bool {
    packages::valid_slug(slug) && theme_dir(themes_dir, slug).join("theme.toml").is_file()
}

/// 组装单个 ThemeInfo（active_slug 为 config.toml 中的激活 slug）
pub fn theme_info(themes_dir: &Path, slug: &str, active_slug: &str) -> Option<ThemeInfo> {
    let m = load_manifest(themes_dir, slug)?;
    let dir = theme_dir(themes_dir, slug);
    let (installed_at, updated_at) = packages::fs_timestamps(&dir);
    Some(ThemeInfo {
        slug: slug.to_string(),
        name: m.name,
        version: m.version,
        description: m.description,
        author: m.author,
        active: slug == active_slug,
        builtin: slug == BUILTIN_THEME_SLUG,
        has_css: dir.join("theme.css").is_file(),
        preview_url: dir
            .join("preview.png")
            .is_file()
            .then(|| format!("/api/themes/{slug}/preview.png")),
        installed_at,
        updated_at,
    })
}

/// 扫描主题目录 → 全部 ThemeInfo（按 slug 字典序；theme.toml 非法的目录跳过）
pub fn list_themes(themes_dir: &Path, active_slug: &str) -> Vec<ThemeInfo> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(themes_dir) {
        for item in rd.flatten() {
            let path = item.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('.') || !packages::valid_slug(name) {
                continue;
            }
            if let Some(info) = theme_info(themes_dir, name, active_slug) {
                out.push(info);
            }
        }
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// GET /api/themes/active 响应：{slug, name, tokens, tokens_dark?, css_url|null, preview_url?}
/// 任何情况下都返回可用主题：激活主题 → default 目录 → 内置常量兜底
pub fn active_theme_response(themes_dir: &Path, active_slug: &str) -> Value {
    for slug in [active_slug, BUILTIN_THEME_SLUG] {
        if let Some(m) = load_manifest(themes_dir, slug) {
            if m.slug == slug {
                return manifest_to_active_json(m, themes_dir, slug);
            }
        }
    }
    // 磁盘上没有可用 default（如未安装且未生成）：用内置常量兜底
    let m: ThemeManifest =
        toml::from_str(BUILTIN_DEFAULT_THEME_TOML).expect("内置 default 主题 TOML 必须合法");
    manifest_to_active_json(m, themes_dir, BUILTIN_THEME_SLUG)
}

fn manifest_to_active_json(m: ThemeManifest, themes_dir: &Path, slug: &str) -> Value {
    let dir = theme_dir(themes_dir, slug);
    let css_url = if dir.join("theme.css").is_file() {
        json!(format!("/api/themes/{slug}/theme.css"))
    } else {
        Value::Null
    };
    let mut v = json!({
        "slug": slug,
        "name": m.name,
        "tokens": m.tokens,
        "css_url": css_url,
    });
    if let Some(dark) = m.tokens_dark {
        v["tokens_dark"] = json!(dark);
    }
    if dir.join("preview.png").is_file() {
        v["preview_url"] = json!(format!("/api/themes/{slug}/preview.png"));
    }
    v
}

/// 按扩展名猜 MIME（静态资源托管用）；未知 → application/octet-stream
pub fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        _ => "application/octet-stream",
    }
}
