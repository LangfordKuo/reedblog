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
pub const BUILTIN_DEFAULT_THEME_TOML: &str = r##"name = "默认主题"
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

# 主题设置项（扩展契约「主题设置项」条款）：后台按声明渲染设置面板，
# 值按主题 slug 存 theme_settings 表。layout 为前端内置消费的系统级设置。
[[settings]]
key = "layout"
label = "页面布局"
type = "select"
group = "布局"
default = "topbar-two-column"
options = [
  { value = "topbar-two-column", label = "顶栏导航 + 双列" },
  { value = "topbar-minimal-three-column", label = "极简顶栏 + 三列" },
]

[[settings]]
key = "wide_layout"
label = "宽幅正文"
type = "switch"
group = "布局"
default = false

[[settings]]
key = "accent_color"
label = "强调色"
type = "color"
group = "配色"
default = "#0f172a"
"##;

/// 设置项 type 枚举（扩展契约「主题设置项」条款）
pub const SETTING_TYPES: [&str; 6] = ["text", "textarea", "color", "select", "switch", "number"];

/// select 选项（下发时统一归一化为 {value, label}；theme.toml 中也接受纯字符串）
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThemeSettingOption {
    pub value: String,
    pub label: String,
}

/// theme.toml `[[settings]]` 单项声明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSettingDecl {
    pub key: String,
    /// 面板显示名；缺省（空）时归一化为 key
    #[serde(default)]
    pub label: String,
    /// text | textarea | color | select | switch | number（TOML 字段名为 type）
    #[serde(rename = "type")]
    pub kind: String,
    /// 面板分组标题；缺省/空串不分组
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 默认值（类型须与 kind 匹配，validate 强制）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// select 专用选项（字符串或 {value,label} 均可，反序列化时归一化）
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_options"
    )]
    pub options: Option<Vec<ThemeSettingOption>>,
}

/// options 兼容两种 TOML 写法：`"value"` 与 `{ value = "...", label = "..." }`
fn deserialize_options<'de, D>(d: D) -> Result<Option<Vec<ThemeSettingOption>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawOption {
        Str(String),
        Obj {
            value: String,
            #[serde(default)]
            label: Option<String>,
        },
    }
    let raw: Option<Vec<RawOption>> = Option::deserialize(d)?;
    Ok(raw.map(|items| {
        items
            .into_iter()
            .map(|r| match r {
                RawOption::Str(s) => ThemeSettingOption {
                    label: s.clone(),
                    value: s,
                },
                RawOption::Obj { value, label } => ThemeSettingOption {
                    label: label.unwrap_or_else(|| value.clone()),
                    value,
                },
            })
            .collect()
    }))
}

/// 设置项 key 合法性：^[a-z0-9][a-z0-9_-]{0,63}$（CSS 变量/data 属性片段，_ 下发时换 -）
pub fn valid_setting_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

/// color 类型值校验：#RGB / #RRGGBB / #RRGGBBAA（大小写不敏感）
pub fn valid_hex_color(v: &str) -> bool {
    let b = v.as_bytes();
    if b.is_empty() || b[0] != b'#' || !matches!(b.len(), 4 | 7 | 9) {
        return false;
    }
    b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

/// theme.toml `[[widgets]]` 单条声明（扩展契约「主题组件」条款）：
/// 主题自带的 HTML 片段型自定义组件，片段固定在 assets/widgets/<key>.html
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeWidgetDecl {
    /// 组件唯一标识：^[a-z0-9][a-z0-9_-]{0,63}$，主题内唯一，不得与内置组件 key 冲突
    pub key: String,
    /// 显示名；缺省（空）时归一化为 key
    #[serde(default)]
    pub label: String,
    /// 默认是否启用（缺省 false）
    #[serde(default)]
    pub default_enabled: bool,
    /// 默认位置（sidebar|left|right|footer，缺省 sidebar）
    #[serde(default = "default_widget_position")]
    pub default_position: String,
    /// 默认排序（缺省 100，排在内置组件默认序之后）
    #[serde(default = "default_widget_sort")]
    pub default_sort: i64,
    /// 参数 schema（复用 [[settings]] 声明形状与校验；片段内 {{param}} 令牌替换）
    #[serde(default)]
    pub params: Vec<ThemeSettingDecl>,
}

fn default_widget_position() -> String {
    "sidebar".to_string()
}

fn default_widget_sort() -> i64 {
    100
}

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
    /// 主题设置项声明（可选；未声明的主题面板显示「该主题无自定义设置项」）
    #[serde(default)]
    pub settings: Vec<ThemeSettingDecl>,
    /// 主题声明组件（可选；未声明的主题只有内置组件可配）
    #[serde(default)]
    pub widgets: Vec<ThemeWidgetDecl>,
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
        self.validate_settings()?;
        self.validate_widgets()
    }

    /// 校验 [[settings]] 声明（契约「主题设置项」：key/type/options/default 规则）；
    /// 上传 zip 与后端解析共用，违规 → 422 invalid_manifest
    pub fn validate_settings(&self) -> Result<(), String> {
        validate_param_decls(&self.settings, "settings")
    }

    /// 校验 [[widgets]] 声明（扩展契约「主题组件」）：key 规则同 settings.key、
    /// 主题内唯一、不得与内置组件 key 冲突；default_position 须在规范枚举内；
    /// params 复用 [[settings]] 的声明校验。违规 → 422 invalid_manifest
    pub fn validate_widgets(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for w in &self.widgets {
            if !valid_setting_key(&w.key) {
                return Err(format!(
                    "widgets.key '{}' 非法（须匹配 ^[a-z0-9][a-z0-9_-]{{0,63}}$）",
                    w.key
                ));
            }
            if !seen.insert(&w.key) {
                return Err(format!("widgets.key '{}' 重复声明", w.key));
            }
            if crate::widgets::builtin_def(&w.key).is_some() {
                return Err(format!("widgets.key '{}' 与内置组件 key 冲突", w.key));
            }
            if !crate::widgets::valid_position(&w.default_position) {
                return Err(format!(
                    "widgets[{}].default_position '{}' 非法（须为 {} 之一）",
                    w.key,
                    w.default_position,
                    crate::widgets::POSITIONS.join(" / ")
                ));
            }
            if w.label.chars().count() > 64 {
                return Err(format!("widgets[{}].label 过长（上限 64 字符）", w.key));
            }
            validate_param_decls(&w.params, &format!("widgets[{}].params", w.key))?;
        }
        Ok(())
    }

    /// 下发用归一化声明：label 缺省填 key、group 空串视为无
    pub fn normalized_settings(&self) -> Vec<ThemeSettingDecl> {
        normalize_param_decls(&self.settings)
    }

    /// 下发用归一化组件声明：label 缺省填 key；**宽容过滤**非法声明
    /// （读路径用：磁盘上手工改坏的声明静默跳过，不阻断其余组件；
    /// 上传路径的严格校验由 validate_widgets 负责）
    pub fn normalized_widgets(&self) -> Vec<ThemeWidgetDecl> {
        let mut seen = std::collections::HashSet::new();
        self.widgets
            .iter()
            .filter(|w| {
                valid_setting_key(&w.key)
                    && seen.insert(w.key.clone())
                    && crate::widgets::builtin_def(&w.key).is_none()
                    && crate::widgets::valid_position(&w.default_position)
                    && validate_param_decls(&w.params, "params").is_ok()
            })
            .map(|w| ThemeWidgetDecl {
                key: w.key.clone(),
                label: if w.label.trim().is_empty() {
                    w.key.clone()
                } else {
                    w.label.trim().to_string()
                },
                default_enabled: w.default_enabled,
                default_position: w.default_position.clone(),
                default_sort: w.default_sort,
                params: normalize_param_decls(&w.params),
            })
            .collect()
    }
}

/// 参数声明数组的通用校验（[[settings]] 与 [[widgets]].params 共用；
/// ctx 为错误消息前缀，如 "settings" / "widgets[notice].params"）
fn validate_param_decls(decls: &[ThemeSettingDecl], ctx: &str) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for s in decls {
        if !valid_setting_key(&s.key) {
            return Err(format!(
                "{ctx}.key '{}' 非法（须匹配 ^[a-z0-9][a-z0-9_-]{{0,63}}$）",
                s.key
            ));
        }
        if !seen.insert(&s.key) {
            return Err(format!("{ctx}.key '{}' 重复声明", s.key));
        }
        if !SETTING_TYPES.contains(&s.kind.as_str()) {
            return Err(format!(
                "{ctx}[{}].type '{}' 未知（须为 {} 之一）",
                s.key,
                s.kind,
                SETTING_TYPES.join(" / ")
            ));
        }
        if s.group.as_deref().map(|g| g.chars().count()).unwrap_or(0) > 64 {
            return Err(format!("{ctx}[{}].group 过长（上限 64 字符）", s.key));
        }
        let is_select = s.kind == "select";
        if is_select {
            let opts = s
                .options
                .as_ref()
                .filter(|o| !o.is_empty())
                .ok_or_else(|| format!("{ctx}[{}]: type=select 必须声明非空 options", s.key))?;
            if opts.iter().any(|o| o.value.trim().is_empty()) {
                return Err(format!("{ctx}[{}].options 含空 value", s.key));
            }
            let mut uniq = std::collections::HashSet::new();
            for o in opts {
                if !uniq.insert(&o.value) {
                    return Err(format!(
                        "{ctx}[{}].options 含重复 value '{}'",
                        s.key, o.value
                    ));
                }
            }
        } else if s.options.is_some() {
            return Err(format!("{ctx}[{}]: 仅 type=select 允许声明 options", s.key));
        }
        if let Some(d) = &s.default {
            validate_default(ctx, &s.key, &s.kind, s.options.as_deref(), d)?;
        }
    }
    Ok(())
}

/// 参数声明数组的归一化（label 缺省填 key、group 空串视为无）
fn normalize_param_decls(decls: &[ThemeSettingDecl]) -> Vec<ThemeSettingDecl> {
    decls
        .iter()
        .map(|s| ThemeSettingDecl {
            key: s.key.clone(),
            label: if s.label.trim().is_empty() {
                s.key.clone()
            } else {
                s.label.trim().to_string()
            },
            kind: s.kind.clone(),
            group: s
                .group
                .as_deref()
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(str::to_string),
            default: s.default.clone(),
            options: s.options.clone(),
        })
        .collect()
}

/// default 值与 type 的匹配校验（契约「主题设置项」声明校验条款）
fn validate_default(
    ctx: &str,
    key: &str,
    kind: &str,
    options: Option<&[ThemeSettingOption]>,
    d: &serde_json::Value,
) -> Result<(), String> {
    use serde_json::Value as V;
    let err = |msg: String| Err(format!("{ctx}[{key}].default {msg}"));
    match kind {
        "switch" => match d {
            V::Bool(_) => Ok(()),
            _ => err("必须是 bool（type=switch）".into()),
        },
        "number" => match d {
            V::Number(_) => Ok(()),
            _ => err("必须是数字（type=number）".into()),
        },
        "select" => match d {
            V::String(s) if options.is_some_and(|o| o.iter().any(|x| &x.value == s)) => Ok(()),
            V::String(s) => err(format!("'{s}' 不在 options 内（type=select）")),
            _ => err("必须是字符串（type=select）".into()),
        },
        "color" => match d {
            V::String(s) if valid_hex_color(s) => Ok(()),
            V::String(s) => err(format!("'{s}' 不是合法 hex 颜色（#RGB/#RRGGBB/#RRGGBBAA）")),
            _ => err("必须是字符串（type=color）".into()),
        },
        // text / textarea
        _ => match d {
            V::String(s) => {
                let max = if kind == "textarea" { 5000 } else { 500 };
                if s.chars().count() > max {
                    return err(format!("过长（type={kind} 上限 {max} 字符）"));
                }
                Ok(())
            }
            _ => err(format!("必须是字符串（type={kind}）")),
        },
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

/// 读取 manifest；仅 default 主题在磁盘缺失/损坏时回退后端内置常量
/// （themes/active 与 themes/:slug/settings 共用：保证任何情况下 default 可解析）
pub fn load_manifest_or_builtin(themes_dir: &Path, slug: &str) -> Option<ThemeManifest> {
    if let Some(m) = load_manifest(themes_dir, slug) {
        if m.slug == slug {
            return Some(m);
        }
    }
    if slug == BUILTIN_THEME_SLUG {
        return Some(builtin_default_manifest());
    }
    None
}

/// 内置 default 主题常量解析（TOML 由单元测试保证合法）
pub fn builtin_default_manifest() -> ThemeManifest {
    toml::from_str(BUILTIN_DEFAULT_THEME_TOML).expect("内置 default 主题 TOML 必须合法")
}

/// 解析激活主题 manifest（兜底顺序：active slug → default 磁盘 → 内置常量），
/// 返回实际生效的 (slug, manifest)；themes/active 与 settings-panel 管理端点共用
pub fn resolve_active_manifest(themes_dir: &Path, active_slug: &str) -> (String, ThemeManifest) {
    for slug in [active_slug, BUILTIN_THEME_SLUG] {
        if let Some(m) = load_manifest_or_builtin(themes_dir, slug) {
            return (slug.to_string(), m);
        }
    }
    (BUILTIN_THEME_SLUG.to_string(), builtin_default_manifest())
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
    let (slug, m) = resolve_active_manifest(themes_dir, active_slug);
    manifest_to_active_json(m, themes_dir, &slug)
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
