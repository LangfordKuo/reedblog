//! 主题组件（api-contract.md「主题组件」+ 扩展契约「主题组件」条款）：
//! - 内置组件注册表（7 个，key 与前端 React 实现一一对应）+ 默认值
//!   （default 主题默认启用 recent-posts/tag-cloud/categories，位置 sidebar）；
//! - theme_widgets 表读写：只存覆盖行，SQLite/MySQL 共用一份 SQL（Any 驱动、
//!   `?` 占位符）；PUT 为全量替换（DELETE 全部行 + 逐条 INSERT）；
//! - PUT 入参校验：kind=builtin 未注册 key → 422 unknown_widget；position 越界 /
//!   key 重复或非法 / config 含未声明参数或超长 → 422 invalid_value；
//! - 生效合并：内置注册表与 theme.toml [[widgets]] 声明打底、已存行覆盖，
//!   参数默认值与已存 config 合并（复用 theme_settings 的类型转换管线）；
//! - custom 组件 HTML：主题声明组件读 assets/widgets/<key>.html（config.html 非空
//!   时为后台覆盖），公开输出前做 {{param}} 令牌替换（不转义，与插件注入同信任模型）；
//! - 删除主题时连带删除其行（与 theme_settings 同款生命周期）。

use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use sqlx::{AnyPool, Row};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{ApiError, ApiResult};
use crate::pages::row_bool;
use crate::state::now_rfc3339;
use crate::theme_settings;
use crate::themes::{valid_setting_key, ThemeSettingDecl, ThemeWidgetDecl};

/// position 规范枚举（存储与校验与布局无关；布局降级映射由前端执行，
/// 契约「主题组件」：双列 left/right 并入 sidebar，三列 sidebar 映射右栏）
pub const POSITIONS: [&str; 4] = ["sidebar", "left", "right", "footer"];

pub fn valid_position(p: &str) -> bool {
    POSITIONS.contains(&p)
}

/// 单主题组件数上限（PUT 校验）
const MAX_WIDGETS: usize = 100;
/// custom 组件 config.html 长度上限（字符）
const MAX_HTML_CHARS: usize = 65536;
/// custom 组件 config.title 长度上限（字符）
const MAX_TITLE_CHARS: usize = 200;

fn invalid_value(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_value", msg)
}

fn unknown_widget(key: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown_widget",
        format!("'{key}' 不是已注册的内置组件 key"),
    )
}

// ---------- 内置组件注册表 ----------

/// 内置组件定义（后端注册表：默认值 + 参数 schema；前端有对应 React 实现）
#[derive(Debug, Clone)]
pub struct BuiltinWidgetDef {
    pub key: &'static str,
    pub label: &'static str,
    pub default_enabled: bool,
    pub default_position: &'static str,
    pub default_sort: i64,
    pub params: Vec<ThemeSettingDecl>,
}

fn text_param(key: &str, label: &str, default: &str) -> ThemeSettingDecl {
    ThemeSettingDecl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "text".to_string(),
        group: None,
        default: Some(Value::String(default.to_string())),
        options: None,
    }
}

fn number_param(key: &str, label: &str, default: i64) -> ThemeSettingDecl {
    ThemeSettingDecl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "number".to_string(),
        group: None,
        default: Some(json!(default)),
        options: None,
    }
}

/// 全部内置组件（契约「主题组件」清单；默认启用集保证新装站点开箱有合理侧栏）
pub fn builtin_defs() -> Vec<BuiltinWidgetDef> {
    let title = |t: &str| text_param("title", "标题文字", t);
    let count = |n: i64| number_param("count", "显示条数", n);
    vec![
        BuiltinWidgetDef {
            key: "site-info",
            label: "站点信息",
            default_enabled: false,
            default_position: "sidebar",
            default_sort: 5,
            params: vec![title("站点信息")],
        },
        BuiltinWidgetDef {
            key: "recent-posts",
            label: "最新文章",
            default_enabled: true,
            default_position: "sidebar",
            default_sort: 10,
            params: vec![title("最新文章"), count(5)],
        },
        BuiltinWidgetDef {
            key: "hot-posts",
            label: "热门文章",
            default_enabled: false,
            default_position: "sidebar",
            default_sort: 20,
            params: vec![title("热门文章"), count(5)],
        },
        BuiltinWidgetDef {
            key: "tag-cloud",
            label: "标签云",
            default_enabled: true,
            default_position: "sidebar",
            default_sort: 30,
            params: vec![title("标签云"), count(20)],
        },
        BuiltinWidgetDef {
            key: "categories",
            label: "分类列表",
            default_enabled: true,
            default_position: "sidebar",
            default_sort: 40,
            params: vec![title("分类")],
        },
        BuiltinWidgetDef {
            key: "archive",
            label: "归档",
            default_enabled: false,
            default_position: "sidebar",
            default_sort: 50,
            params: vec![title("归档")],
        },
        BuiltinWidgetDef {
            key: "links",
            label: "友情链接",
            default_enabled: false,
            default_position: "sidebar",
            default_sort: 60,
            params: vec![title("友情链接"), count(10)],
        },
    ]
}

/// 按 key 查内置组件定义（themes.rs 声明校验与 PUT 校验共用）
pub fn builtin_def(key: &str) -> Option<BuiltinWidgetDef> {
    builtin_defs().into_iter().find(|d| d.key == key)
}

// ---------- theme_widgets 表 ----------

/// theme_widgets 表的已存行（config 为规范化字符串 map：参数值一律字符串存储，
/// 与 theme_settings 同款类型转换；custom 组件含 title/html）
#[derive(Debug, Clone)]
pub struct StoredWidget {
    pub key: String,
    pub enabled: bool,
    pub position: String,
    pub sort_order: i64,
    pub config: BTreeMap<String, String>,
}

/// 读取某主题的全部已存行（损坏的 config JSON 按空对象容错）
pub async fn load_rows(pool: &AnyPool, slug: &str) -> Result<Vec<StoredWidget>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT widget_key, enabled, position, sort_order, config \
         FROM theme_widgets WHERE theme_slug = ?",
    )
    .bind(slug)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut config = BTreeMap::new();
        let raw: String = r.try_get::<String, _>("config").unwrap_or_default();
        if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&raw) {
            for (k, v) in m {
                // 存储侧一律写字符串；对损坏行宽容：标量转回字符串
                let s = match v {
                    Value::String(s) => s,
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
                config.insert(k, s);
            }
        }
        out.push(StoredWidget {
            key: r.get::<String, _>("widget_key"),
            enabled: row_bool(r, "enabled"),
            position: r.try_get::<String, _>("position").unwrap_or_default(),
            sort_order: r.try_get::<i64, _>("sort_order").unwrap_or(0),
            config,
        });
    }
    Ok(out)
}

/// PUT 校验通过后待写入的行（config 已规范化为字符串 map）
#[derive(Debug, Clone)]
pub struct NewWidgetRow {
    pub key: String,
    pub enabled: bool,
    pub position: String,
    pub sort_order: i64,
    pub config: BTreeMap<String, String>,
}

/// 全量替换某主题的组件配置行（契约：PUT 全量替换语义；DELETE + 逐条 INSERT，
/// 双方言共用，不依赖事务隔离级别差异）
pub async fn replace_all(
    pool: &AnyPool,
    slug: &str,
    rows: &[NewWidgetRow],
) -> Result<(), sqlx::Error> {
    let now = now_rfc3339();
    sqlx::query("DELETE FROM theme_widgets WHERE theme_slug = ?")
        .bind(slug)
        .execute(pool)
        .await?;
    for r in rows {
        let config_json = serde_json::to_string(
            &r.config
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect::<Map<String, Value>>(),
        )
        .unwrap_or_else(|_| "{}".to_string());
        let enabled_flag: i64 = if r.enabled { 1 } else { 0 };
        sqlx::query(
            "INSERT INTO theme_widgets (theme_slug, widget_key, enabled, position, \
             sort_order, config, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(slug)
        .bind(&r.key)
        .bind(enabled_flag)
        .bind(&r.position)
        .bind(r.sort_order)
        .bind(&config_json)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// 删除主题的全部组件配置行（卸载主题时调用；契约：卸载即删除配置）
pub async fn delete_for_theme(pool: &AnyPool, slug: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM theme_widgets WHERE theme_slug = ?")
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------- PUT 入参校验 ----------

/// 校验 PUT body → 待写入行（契约「主题组件」校验条款；任何一条失败整体 422 不写库）
pub fn validate_put(theme_decls: &[ThemeWidgetDecl], body: &Value) -> ApiResult<Vec<NewWidgetRow>> {
    let arr = body
        .get("widgets")
        .and_then(|w| w.as_array())
        .ok_or_else(|| ApiError::validation("请求体缺少 widgets 数组"))?;
    if arr.len() > MAX_WIDGETS {
        return Err(invalid_value(format!("组件数量超出上限（{MAX_WIDGETS}）")));
    }

    let mut seen = BTreeMap::new();
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| ApiError::validation(format!("widgets[{i}] 必须是对象")))?;
        let key = obj
            .get("key")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid_value(format!("widgets[{i}] 缺少合法 key")))?;
        if seen.contains_key(key) {
            return Err(invalid_value(format!("组件 key '{key}' 重复")));
        }
        let kind = obj
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_value(format!("widgets[{i}] 缺少 kind")))?;
        let position = obj
            .get("position")
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_value(format!("widgets[{i}] 缺少 position")))?;
        if !valid_position(position) {
            return Err(invalid_value(format!(
                "组件 '{key}' 的 position '{position}' 非法（须为 {} 之一）",
                POSITIONS.join(" / ")
            )));
        }
        let enabled = match obj.get("enabled") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            _ => {
                return Err(invalid_value(format!(
                    "组件 '{key}' 的 enabled 必须是布尔值"
                )))
            }
        };
        let sort_order = match obj.get("sort_order") {
            None | Some(Value::Null) => 0,
            Some(Value::Number(n)) => n
                .as_i64()
                .ok_or_else(|| invalid_value(format!("组件 '{key}' 的 sort_order 必须是整数")))?,
            _ => {
                return Err(invalid_value(format!(
                    "组件 '{key}' 的 sort_order 必须是整数"
                )))
            }
        };
        let config_in = match obj.get("config") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            _ => return Err(invalid_value(format!("组件 '{key}' 的 config 必须是对象"))),
        };

        let config = match kind {
            "builtin" => {
                let def = builtin_def(key).ok_or_else(|| unknown_widget(key))?;
                normalize_params_config(key, &def.params, &config_in)?
            }
            "custom" => {
                if builtin_def(key).is_some() {
                    return Err(invalid_value(format!(
                        "自定义组件 key '{key}' 与内置组件冲突"
                    )));
                }
                if let Some(decl) = theme_decls.iter().find(|w| w.key == key) {
                    // 主题声明组件：html 允许后台覆盖，其余参数按声明校验
                    let (html, rest) = split_html(config_in);
                    let mut cfg = normalize_params_config(key, &decl.params, &rest)?;
                    if let Some(h) = html {
                        check_html_len(key, &h)?;
                        cfg.insert("html".to_string(), h);
                    }
                    cfg
                } else {
                    // 后台自建组件：key 合法性 + config 仅 title/html
                    if !valid_setting_key(key) {
                        return Err(invalid_value(format!(
                            "自定义组件 key '{key}' 非法（须匹配 ^[a-z0-9][a-z0-9_-]{{0,63}}$）"
                        )));
                    }
                    let mut cfg = BTreeMap::new();
                    for (k, v) in &config_in {
                        if k != "title" && k != "html" {
                            return Err(invalid_value(format!(
                                "自定义组件 '{key}' 的 config 含未知参数 '{k}'（仅 title/html）"
                            )));
                        }
                        let s = v.as_str().ok_or_else(|| {
                            invalid_value(format!("自定义组件 '{key}' 的 config.{k} 必须是字符串"))
                        })?;
                        if k == "title" && s.chars().count() > MAX_TITLE_CHARS {
                            return Err(invalid_value(format!(
                                "组件 '{key}' 的 title 过长（上限 {MAX_TITLE_CHARS} 字符）"
                            )));
                        }
                        if k == "html" {
                            check_html_len(key, s)?;
                        }
                        cfg.insert(k.clone(), s.to_string());
                    }
                    cfg
                }
            }
            other => {
                return Err(invalid_value(format!(
                    "widgets[{i}] 的 kind '{other}' 非法（须为 builtin 或 custom）"
                )))
            }
        };

        seen.insert(key.to_string(), ());
        out.push(NewWidgetRow {
            key: key.to_string(),
            enabled,
            position: position.to_string(),
            sort_order,
            config,
        });
    }
    Ok(out)
}

fn check_html_len(key: &str, h: &str) -> ApiResult<()> {
    if h.chars().count() > MAX_HTML_CHARS {
        return Err(invalid_value(format!(
            "组件 '{key}' 的 html 过长（上限 {MAX_HTML_CHARS} 字符）"
        )));
    }
    Ok(())
}

/// 从 config 入参中拆出 html 覆盖字段（字符串才认；其余字段走参数校验）
fn split_html(mut config_in: Map<String, Value>) -> (Option<String>, Map<String, Value>) {
    match config_in.remove("html") {
        Some(Value::String(s)) => (Some(s), config_in),
        Some(other) => {
            // 非字符串 html 放回，让参数校验报「未知参数」或类型错
            config_in.insert("html".to_string(), other);
            (None, config_in)
        }
        None => (None, config_in),
    }
}

/// 按参数声明规范化 config（复用 theme_settings 管线；未声明参数/类型不符/超长
/// 统一映射为 422 invalid_value，契约「主题组件」）
fn normalize_params_config(
    key: &str,
    params: &[ThemeSettingDecl],
    config_in: &Map<String, Value>,
) -> ApiResult<BTreeMap<String, String>> {
    theme_settings::validate_values(params, &Value::Object(config_in.clone())).map_err(|e| {
        if e.code == "unknown_setting" {
            invalid_value(format!(
                "组件 '{key}' 的 config 含未声明参数：{}",
                e.message
            ))
        } else {
            e
        }
    })
}

// ---------- 生效合并与响应 ----------

/// 合并后的单个组件（默认值打底、已存行覆盖）
pub struct ResolvedWidget {
    pub key: String,
    pub kind: &'static str,
    pub label: String,
    pub source: &'static str,
    pub enabled: bool,
    pub position: String,
    pub sort_order: i64,
    pub params: Vec<ThemeSettingDecl>,
    /// 已存 config（规范化字符串；admin custom 含 title/html）
    pub stored: BTreeMap<String, String>,
}

impl ResolvedWidget {
    /// 参数生效值（声明 default 与已存 config 合并，按类型输出；admin custom 为原样字符串）
    fn effective_config(&self) -> Value {
        if self.params.is_empty() && self.source == "admin" {
            let mut m = Map::new();
            m.insert(
                "title".to_string(),
                Value::String(self.stored.get("title").cloned().unwrap_or_default()),
            );
            m.insert(
                "html".to_string(),
                Value::String(self.stored.get("html").cloned().unwrap_or_default()),
            );
            return Value::Object(m);
        }
        let mut v = theme_settings::merged_values(&self.params, &self.stored);
        // 主题声明组件的 html 后台覆盖（仅存过时出现；文件内容在响应组装时注入）
        if self.source == "theme" {
            if let Some(h) = self.stored.get("html").filter(|h| !h.is_empty()) {
                v.as_object_mut()
                    .unwrap()
                    .insert("html".to_string(), Value::String(h.clone()));
            }
        }
        v
    }
}

/// 默认值打底 + 已存行覆盖 → 全量合并列表（内置 → 主题声明 → 自建 custom；
/// 已存行中 key 不再被任何声明认识的（如主题更新移除了声明）按自建 custom 兜底展示）
pub fn resolve(theme_decls: &[ThemeWidgetDecl], rows: &[StoredWidget]) -> Vec<ResolvedWidget> {
    let mut out = Vec::new();
    let mut consumed = std::collections::HashSet::new();

    for def in builtin_defs() {
        let row = rows.iter().find(|r| r.key == def.key);
        if let Some(r) = row {
            consumed.insert(r.key.clone());
        }
        out.push(ResolvedWidget {
            key: def.key.to_string(),
            kind: "builtin",
            label: def.label.to_string(),
            source: "builtin",
            enabled: row.map(|r| r.enabled).unwrap_or(def.default_enabled),
            position: row
                .map(|r| r.position.clone())
                .filter(|p| valid_position(p))
                .unwrap_or_else(|| def.default_position.to_string()),
            sort_order: row.map(|r| r.sort_order).unwrap_or(def.default_sort),
            params: def.params.clone(),
            stored: row.map(|r| r.config.clone()).unwrap_or_default(),
        });
    }

    for decl in theme_decls {
        let row = rows.iter().find(|r| r.key == decl.key);
        if let Some(r) = row {
            consumed.insert(r.key.clone());
        }
        out.push(ResolvedWidget {
            key: decl.key.clone(),
            kind: "custom",
            label: decl.label.clone(),
            source: "theme",
            enabled: row.map(|r| r.enabled).unwrap_or(decl.default_enabled),
            position: row
                .map(|r| r.position.clone())
                .filter(|p| valid_position(p))
                .unwrap_or_else(|| decl.default_position.clone()),
            sort_order: row.map(|r| r.sort_order).unwrap_or(decl.default_sort),
            params: decl.params.clone(),
            stored: row.map(|r| r.config.clone()).unwrap_or_default(),
        });
    }

    for r in rows {
        if consumed.contains(&r.key) || !valid_setting_key(&r.key) {
            continue;
        }
        let label = r
            .config
            .get("title")
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| r.key.clone());
        out.push(ResolvedWidget {
            key: r.key.clone(),
            kind: "custom",
            label,
            source: "admin",
            enabled: r.enabled,
            position: if valid_position(&r.position) {
                r.position.clone()
            } else {
                "sidebar".to_string()
            },
            sort_order: r.sort_order,
            params: vec![],
            stored: r.config.clone(),
        });
    }

    out.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

/// custom 组件的原始 HTML（未做令牌替换）：后台覆盖优先，
/// 主题声明组件回退主题包 assets/widgets/<key>.html（每次从磁盘读，缺失为空串）
fn raw_html(themes_dir: &Path, slug: &str, w: &ResolvedWidget) -> String {
    if let Some(h) = w.stored.get("html") {
        if !h.is_empty() {
            return h.clone();
        }
    }
    if w.source == "theme" {
        let path = themes_dir
            .join(slug)
            .join("assets")
            .join("widgets")
            .join(format!("{}.html", w.key));
        return std::fs::read_to_string(path).unwrap_or_default();
    }
    String::new()
}

/// {{param}} 令牌替换（值为生效 config 的字符串化；未知令牌原样保留；不转义——
/// 与插件注入同信任模型）
pub fn substitute_tokens(html: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                out.push_str(&rest[..start]);
                match values.get(name) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(rest);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// 生效值 → 令牌替换用的字符串 map（排除 html 自身，防自引用）
fn token_values(config: &Value) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(obj) = config.as_object() {
        for (k, v) in obj {
            if k == "html" {
                continue;
            }
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            m.insert(k.clone(), s);
        }
    }
    m
}

/// GET /api/themes/:slug/widgets 响应：仅 enabled 组件（sort_order ASC, key ASC），
/// custom 组件 config.html 为令牌替换后的最终片段
pub fn public_response(
    themes_dir: &Path,
    slug: &str,
    theme_decls: &[ThemeWidgetDecl],
    rows: &[StoredWidget],
) -> Value {
    let widgets: Vec<Value> = resolve(theme_decls, rows)
        .into_iter()
        .filter(|w| w.enabled)
        .map(|w| {
            let mut config = w.effective_config();
            if w.kind == "custom" {
                let html =
                    substitute_tokens(&raw_html(themes_dir, slug, &w), &token_values(&config));
                config
                    .as_object_mut()
                    .unwrap()
                    .insert("html".to_string(), Value::String(html));
            }
            json!({
                "key": w.key,
                "kind": w.kind,
                "label": w.label,
                "position": w.position,
                "sort_order": w.sort_order,
                "config": config,
            })
        })
        .collect();
    json!({ "slug": slug, "widgets": widgets })
}

/// GET/PUT /api/admin/themes/:slug/widgets 响应：全量合并列表（含停用），
/// 附 params 声明供后台渲染参数编辑器；主题组件 config.html 仅为后台覆盖
/// （不含文件内容，UI 提示留空即用主题文件）
pub fn admin_response(slug: &str, theme_decls: &[ThemeWidgetDecl], rows: &[StoredWidget]) -> Value {
    let widgets: Vec<Value> = resolve(theme_decls, rows)
        .into_iter()
        .map(|w| {
            json!({
                "key": w.key,
                "kind": w.kind,
                "label": w.label,
                "source": w.source,
                "enabled": w.enabled,
                "position": w.position,
                "sort_order": w.sort_order,
                "config": w.effective_config(),
                "params": w.params,
            })
        })
        .collect();
    json!({
        "slug": slug,
        "positions": POSITIONS,
        "widgets": widgets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl_json() -> Vec<ThemeWidgetDecl> {
        let toml_text = r##"
name = "t"
slug = "t"
version = "1.0.0"

[[widgets]]
key = "notice"
label = "公告栏"
default_enabled = true
default_position = "footer"
default_sort = 70

[[widgets.params]]
key = "text"
type = "text"
default = "欢迎"
"##;
        let m: crate::themes::ThemeManifest = toml::from_str(toml_text).unwrap();
        assert!(m.validate_widgets().is_ok());
        m.normalized_widgets()
    }

    #[test]
    fn builtin_registry_defaults() {
        let defs = builtin_defs();
        assert_eq!(defs.len(), 7);
        let enabled: Vec<&str> = defs
            .iter()
            .filter(|d| d.default_enabled)
            .map(|d| d.key)
            .collect();
        assert_eq!(enabled, vec!["recent-posts", "tag-cloud", "categories"]);
        for d in &defs {
            assert!(valid_position(d.default_position));
            assert!(d.params.iter().any(|p| p.key == "title"));
        }
    }

    #[test]
    fn resolve_merges_defaults_and_rows() {
        let decls = decl_json();
        let rows = vec![
            StoredWidget {
                key: "recent-posts".into(),
                enabled: false,
                position: "left".into(),
                sort_order: 99,
                config: BTreeMap::from([("count".into(), "3".into())]),
            },
            StoredWidget {
                key: "notice".into(),
                enabled: true,
                position: "sidebar".into(),
                sort_order: 5,
                config: BTreeMap::from([("text".into(), "你好".into())]),
            },
            StoredWidget {
                key: "custom-box".into(),
                enabled: true,
                position: "footer".into(),
                sort_order: 200,
                config: BTreeMap::from([
                    ("title".into(), "我的盒子".into()),
                    ("html".into(), "<b>hi</b>".into()),
                ]),
            },
        ];
        let resolved = resolve(&decls, &rows);
        // 数量 = 7 内置 + 1 主题声明 + 1 自建
        assert_eq!(resolved.len(), 9);
        let find = |k: &str| resolved.iter().find(|w| w.key == k).unwrap();

        let rp = find("recent-posts");
        assert!(!rp.enabled && rp.position == "left" && rp.sort_order == 99);
        let cfg = rp.effective_config();
        assert_eq!(cfg["count"], json!(3)); // 已存覆盖
        assert_eq!(cfg["title"], json!("最新文章")); // 默认打底

        // 主题声明组件：行覆盖默认（enabled 保持 true、position 改 sidebar）
        let notice = find("notice");
        assert_eq!(notice.source, "theme");
        assert_eq!(notice.kind, "custom");
        assert!(notice.enabled);
        assert_eq!(notice.position, "sidebar");
        assert_eq!(notice.effective_config()["text"], json!("你好"));

        // 未存过的内置组件 = 注册表默认
        let hot = find("hot-posts");
        assert!(!hot.enabled && hot.position == "sidebar" && hot.sort_order == 20);

        // 自建 custom：label 取 config.title
        let box_ = find("custom-box");
        assert_eq!(box_.source, "admin");
        assert_eq!(box_.label, "我的盒子");

        // 排序：sort_order ASC（notice 5 → site-info 5? site-info 默认 5，
        // 相同时按 key ASC：custom-box(200) 最后）
        assert_eq!(resolved.last().unwrap().key, "custom-box");
    }

    #[test]
    fn validate_put_rules() {
        let decls = decl_json();
        // 合法：builtin 覆盖 + 主题组件带 html 覆盖 + 自建 custom
        let body = json!({"widgets": [
            {"key": "recent-posts", "kind": "builtin", "enabled": true,
             "position": "right", "sort_order": 10, "config": {"count": 3, "title": "最新"}},
            {"key": "notice", "kind": "custom", "enabled": true, "position": "footer",
             "config": {"text": "hi", "html": "<p>{{text}}</p>"}},
            {"key": "custom-x", "kind": "custom", "enabled": false, "position": "sidebar",
             "config": {"title": "X", "html": "<i>x</i>"}},
        ]});
        let rows = validate_put(&decls, &body).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].config["count"], "3"); // number 规范化为字符串存储
        assert!(rows[2].config.contains_key("html"));

        // 未注册 builtin key → 422 unknown_widget
        let e = validate_put(
            &decls,
            &json!({"widgets": [{"key": "nope", "kind": "builtin", "position": "sidebar"}]}),
        )
        .unwrap_err();
        assert_eq!(e.code, "unknown_widget");
        assert_eq!(e.status, StatusCode::UNPROCESSABLE_ENTITY);

        // 非法 position → 422 invalid_value
        let e = validate_put(
            &decls,
            &json!({"widgets": [{"key": "archive", "kind": "builtin", "position": "header"}]}),
        )
        .unwrap_err();
        assert_eq!(e.code, "invalid_value");

        // key 重复 → 422
        let e = validate_put(
            &decls,
            &json!({"widgets": [
                {"key": "archive", "kind": "builtin", "position": "sidebar"},
                {"key": "archive", "kind": "builtin", "position": "left"},
            ]}),
        )
        .unwrap_err();
        assert_eq!(e.code, "invalid_value");

        // custom key 与内置冲突 / key 非法
        assert_eq!(
            validate_put(
                &decls,
                &json!({"widgets": [{"key": "archive", "kind": "custom", "position": "sidebar"}]}),
            )
            .unwrap_err()
            .code,
            "invalid_value"
        );
        assert_eq!(
            validate_put(
                &decls,
                &json!({"widgets": [{"key": "Bad Key", "kind": "custom", "position": "sidebar"}]}),
            )
            .unwrap_err()
            .code,
            "invalid_value"
        );

        // config 含未声明参数 / html 超长 / widgets 非数组
        assert_eq!(
            validate_put(
                &decls,
                &json!({"widgets": [{"key": "recent-posts", "kind": "builtin",
                    "position": "sidebar", "config": {"nope": 1}}]}),
            )
            .unwrap_err()
            .code,
            "invalid_value"
        );
        assert_eq!(
            validate_put(
                &decls,
                &json!({"widgets": [{"key": "custom-x", "kind": "custom", "position": "sidebar",
                    "config": {"html": "长".repeat(65537)}}]}),
            )
            .unwrap_err()
            .code,
            "invalid_value"
        );
        assert_eq!(
            validate_put(&decls, &json!({"widgets": {}}))
                .unwrap_err()
                .code,
            "validation_error"
        );
    }

    #[test]
    fn token_substitution() {
        let values = BTreeMap::from([
            ("text".to_string(), "你好".to_string()),
            ("n".to_string(), "3".to_string()),
        ]);
        assert_eq!(
            substitute_tokens("<p>{{text}} x{{ n }} {{unknown}}</p>", &values),
            "<p>你好 x3 {{unknown}}</p>"
        );
        assert_eq!(substitute_tokens("no tokens", &values), "no tokens");
        assert_eq!(substitute_tokens("dangling {{", &values), "dangling {{");
    }
}
