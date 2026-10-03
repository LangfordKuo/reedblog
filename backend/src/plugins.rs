//! 插件系统（docs/extensibility-contract.md 第一部分）：
//! - 插件 = plugins/<slug>/（manifest.toml + main.rhai + 可选 inject/head.html、body_end.html）
//! - Rhai 沙箱：max_call_levels 64 / max_operations 50000 / max_string_size 1MB /
//!   array、map 上限 10000；不注册任何文件/网络/进程 API（Rhai 本身也不提供）
//! - 4 个钩子按插件 slug 字典序链式串行调用；comment.before_create 支持 block 短路
//! - 运行时错误跳过不阻断主流程，写 entry.last_error
//! - 启用状态存 DB plugins 表；manifest 以磁盘为准；热加载（启用/停用/装卸不重启）

use serde::{Deserialize, Serialize};
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Row};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{ApiError, ApiResult};
use crate::packages;
use crate::state::now_rfc3339;

/// 当前 reedblog 版本（min_app_version 校验用）
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 全部合法钩子 id（manifest hooks 枚举校验 + 路由）
pub const ALL_HOOKS: [&str; 4] = [
    "post.before_render",
    "post.after_render",
    "comment.before_create",
    "post.after_publish",
];

/// 全部合法前端注入位置
pub const ALL_INJECTS: [&str; 2] = ["head", "body_end"];

/// Rhai 沙箱：按契约设限；Engine::new 仅含 Rhai 标准库（纯计算，无文件/网络/进程能力），
/// 我们不注册任何外部函数。
fn sandbox_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    engine.set_max_call_levels(64);
    engine.set_max_operations(50_000);
    engine.set_max_string_size(1024 * 1024); // 1MB
    engine.set_max_array_size(10_000);
    engine.set_max_map_size(10_000);
    engine
}

/// 钩子 id → Rhai 函数名（`.`→`_`）
fn hook_fn_name(hook_id: &str) -> String {
    hook_id.replace('.', "_")
}

// ---------- manifest ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub slug: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub min_app_version: Option<String>,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub inject: Vec<String>,
}

impl PluginManifest {
    /// 校验 manifest（dir_name 为 zip/磁盘上的目录名）。错误消息用于 422 invalid_manifest。
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
        if let Some(mav) = &self.min_app_version {
            if !packages::valid_semver(mav) {
                return Err(format!("min_app_version '{mav}' 不是合法 semver"));
            }
        }
        for h in &self.hooks {
            if !ALL_HOOKS.contains(&h.as_str()) {
                return Err(format!("未知钩子 '{h}'"));
            }
        }
        for i in &self.inject {
            if !ALL_INJECTS.contains(&i.as_str()) {
                return Err(format!("未知注入位置 '{i}'"));
            }
        }
        Ok(())
    }
}

/// 契约 PluginInfo 形状
#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    pub hooks: Vec<String>,
    pub inject: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub installed_at: String,
    pub updated_at: String,
}

// ---------- 注册表 ----------

struct PluginEntry {
    manifest: PluginManifest,
    enabled: bool,
    /// 启用时从磁盘加载并语法校验过的 main.rhai 源码（钩子执行用）
    script: Option<String>,
    last_error: Option<String>,
    installed_at: String,
    updated_at: String,
}

impl PluginEntry {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            slug: self.manifest.slug.clone(),
            name: self.manifest.name.clone(),
            version: self.manifest.version.clone(),
            description: self.manifest.description.clone(),
            author: self.manifest.author.clone(),
            enabled: self.enabled,
            hooks: self.manifest.hooks.clone(),
            inject: self.manifest.inject.clone(),
            min_app_version: self.manifest.min_app_version.clone(),
            last_error: self.last_error.clone(),
            installed_at: self.installed_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

/// comment.before_create 链式结果
pub enum CommentDecision {
    Allow {
        author_name: String,
        email: Option<String>,
        content: String,
    },
    Block {
        reason: String,
    },
}

/// 前端注入片段
pub struct Injection {
    pub plugin: String,
    pub html: String,
}

/// 插件宿主：目录 + 内存注册表（BTreeMap 按 slug 字典序 = 钩子链式顺序）。
/// Clone 共享同一注册表（Arc）。
#[derive(Clone)]
pub struct PluginHost {
    dir: PathBuf,
    entries: Arc<RwLock<BTreeMap<String, PluginEntry>>>,
}

fn invalid_manifest(msg: impl Into<String>) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_manifest",
        msg,
    )
}

impl PluginHost {
    /// 创建并扫描磁盘插件目录（manifest 非法的目录跳过并告警；均视为未启用，
    /// 启用状态之后由 restore_from_db 按 DB 恢复）
    pub fn new(dir: PathBuf) -> Self {
        let host = Self {
            dir,
            entries: Arc::new(RwLock::new(BTreeMap::new())),
        };
        host.load_disk();
        host
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 扫描磁盘插件目录，加载全部合法 manifest
    fn load_disk(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return; // 目录不存在 = 无插件
        };
        let mut entries = self
            .entries
            .try_write()
            .expect("load_disk 仅在构造时调用，无并发");
        for item in rd.flatten() {
            let path = item.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if dir_name.starts_with('.') {
                continue; // staging 等临时目录
            }
            match load_manifest_from(&path) {
                Ok(manifest) => {
                    let (installed_at, updated_at) = packages::fs_timestamps(&path);
                    entries.insert(
                        manifest.slug.clone(),
                        PluginEntry {
                            manifest,
                            enabled: false,
                            script: None,
                            last_error: None,
                            installed_at,
                            updated_at,
                        },
                    );
                }
                Err(e) => eprintln!("[reedblog] 插件 {dir_name} manifest 无效，已跳过: {e}"),
            }
        }
    }

    pub async fn list(&self) -> Vec<PluginInfo> {
        self.entries
            .read()
            .await
            .values()
            .map(|e| e.info())
            .collect()
    }

    pub async fn get(&self, slug: &str) -> Option<PluginInfo> {
        self.entries.read().await.get(slug).map(|e| e.info())
    }

    // ---------- DB 持久化 ----------

    /// 启动/安装后按 DB plugins 表恢复 enabled 状态与时间戳；
    /// 启用的插件加载并语法校验 main.rhai（失败写 last_error，钩子跳过）。
    /// DB 无记录的磁盘插件视为未启用。
    pub async fn restore_from_db(&self, pool: &AnyPool) {
        let rows = match sqlx::query("SELECT slug, enabled, installed_at, updated_at FROM plugins")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("[reedblog] 读取 plugins 表失败: {e}");
                return;
            }
        };
        let mut entries = self.entries.write().await;
        for r in &rows {
            let slug = r.get::<String, _>("slug");
            let enabled = Self::row_enabled(r);
            let Some(entry) = entries.get_mut(&slug) else {
                eprintln!("[reedblog] plugins 表中的 {slug} 磁盘上不存在，忽略");
                continue;
            };
            entry.enabled = enabled;
            entry.installed_at = r.get::<String, _>("installed_at");
            entry.updated_at = r.get::<String, _>("updated_at");
            if enabled {
                match self.load_and_check_script(&slug) {
                    Ok(src) => entry.script = Some(src),
                    Err(e) => {
                        entry.script = None;
                        entry.last_error = Some(e);
                    }
                }
            } else {
                entry.script = None;
            }
        }
    }

    /// enabled 列 SQLite 返回 INTEGER、MySQL TINYINT(1) 返回 BOOL，两者都兼容
    fn row_enabled(r: &AnyRow) -> bool {
        if let Ok(b) = r.try_get::<bool, _>("enabled") {
            return b;
        }
        r.try_get::<i64, _>("enabled").unwrap_or(0) != 0
    }

    /// 写启用状态（UPDATE 未命中则 INSERT；installed_at 仅首次写入）
    async fn persist_state(
        pool: &AnyPool,
        slug: &str,
        enabled: bool,
        installed_at: &str,
        updated_at: &str,
    ) -> ApiResult<()> {
        let flag: i64 = if enabled { 1 } else { 0 };
        let res = sqlx::query("UPDATE plugins SET enabled = ?, updated_at = ? WHERE slug = ?")
            .bind(flag)
            .bind(updated_at)
            .bind(slug)
            .execute(pool)
            .await?;
        if res.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO plugins (slug, enabled, installed_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(slug)
            .bind(flag)
            .bind(installed_at)
            .bind(updated_at)
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    // ---------- 安装 / 启用 / 停用 / 删除 ----------

    /// POST /api/admin/plugins：解压校验 zip → 移入插件目录 → 注册（enabled=false）→ 写 DB
    pub async fn install_from_zip(&self, pool: &AnyPool, data: &[u8]) -> ApiResult<PluginInfo> {
        let staging = packages::make_staging_dir(&self.dir)?;
        let result = self.install_inner(pool, data, &staging).await;
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    async fn install_inner(
        &self,
        pool: &AnyPool,
        data: &[u8],
        staging: &Path,
    ) -> ApiResult<PluginInfo> {
        let root = packages::extract_single_root_zip(data, staging)?;
        let root_path = staging.join(&root);

        if !root_path.join("manifest.toml").is_file() {
            return Err(ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_package",
                "zip 根目录内缺少 manifest.toml",
            ));
        }
        if !root_path.join("main.rhai").is_file() {
            return Err(ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_package",
                "zip 根目录内缺少 main.rhai",
            ));
        }
        let manifest = load_manifest_from(&root_path).map_err(invalid_manifest)?;
        manifest.validate(&root).map_err(invalid_manifest)?;

        let slug = manifest.slug.clone();
        if self.entries.read().await.contains_key(&slug) || self.dir.join(&slug).exists() {
            return Err(ApiError::conflict(
                "plugin_exists",
                format!("插件 '{slug}' 已存在"),
            ));
        }

        std::fs::create_dir_all(&self.dir)?;
        std::fs::rename(&root_path, self.dir.join(&slug))?;

        let now = now_rfc3339();
        let info = {
            let mut entries = self.entries.write().await;
            entries.insert(
                slug.clone(),
                PluginEntry {
                    manifest,
                    enabled: false,
                    script: None,
                    last_error: None,
                    installed_at: now.clone(),
                    updated_at: now.clone(),
                },
            );
            entries.get(&slug).map(|e| e.info()).unwrap()
        };
        Self::persist_state(pool, &slug, false, &now, &now).await?;
        Ok(info)
    }

    /// POST /api/admin/plugins/:slug/enable：min_app_version 校验 + Rhai 语法校验
    /// （失败 422 script_error 且不启用），成功后加载脚本进内存并写 DB
    pub async fn enable(&self, pool: &AnyPool, slug: &str) -> ApiResult<PluginInfo> {
        let min_required = {
            let entries = self.entries.read().await;
            let entry = entries.get(slug).ok_or_else(ApiError::not_found)?;
            entry.manifest.min_app_version.clone()
        };
        if let Some(min) = &min_required {
            if packages::compare_semver(min, APP_VERSION) == std::cmp::Ordering::Greater {
                return Err(invalid_manifest(format!(
                    "插件要求 reedblog >= {min}，当前版本 {APP_VERSION}"
                )));
            }
        }

        let src = self.load_and_check_script(slug).map_err(|e| {
            ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "script_error",
                e,
            )
        })?;

        let now = now_rfc3339();
        let info = {
            let mut entries = self.entries.write().await;
            let entry = entries.get_mut(slug).ok_or_else(ApiError::not_found)?;
            entry.enabled = true;
            entry.script = Some(src);
            entry.last_error = None;
            entry.updated_at = now.clone();
            entry.info()
        };
        Self::persist_state(pool, slug, true, &info.installed_at, &now).await?;
        Ok(info)
    }

    /// POST /api/admin/plugins/:slug/disable（幂等）
    pub async fn disable(&self, pool: &AnyPool, slug: &str) -> ApiResult<PluginInfo> {
        let now = now_rfc3339();
        let info = {
            let mut entries = self.entries.write().await;
            let entry = entries.get_mut(slug).ok_or_else(ApiError::not_found)?;
            entry.enabled = false;
            entry.script = None;
            entry.updated_at = now.clone();
            entry.info()
        };
        Self::persist_state(pool, slug, false, &info.installed_at, &now).await?;
        Ok(info)
    }

    /// DELETE /api/admin/plugins/:slug：删目录 + 注册表 + DB 行（启用中也可直接删）
    pub async fn delete(&self, pool: &AnyPool, slug: &str) -> ApiResult<()> {
        let known = self.entries.read().await.contains_key(slug);
        let dir = self.dir.join(slug);
        if !known && !dir.exists() {
            return Err(ApiError::not_found());
        }
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        self.entries.write().await.remove(slug);
        sqlx::query("DELETE FROM plugins WHERE slug = ?")
            .bind(slug)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// 读取 main.rhai 并做沙箱语法编译校验；返回源码或错误消息
    fn load_and_check_script(&self, slug: &str) -> Result<String, String> {
        let path = self.dir.join(slug).join("main.rhai");
        let src = std::fs::read_to_string(&path)
            .map_err(|e| format!("无法读取 main.rhai ({}): {e}", path.display()))?;
        sandbox_engine()
            .compile(&src)
            .map_err(|e| format!("Rhai 语法错误: {e}"))?;
        Ok(src)
    }

    // ---------- 钩子链式调用 ----------

    /// 取声明并启用（且脚本已加载）了指定钩子的插件链：按 slug 字典序，(slug, script)
    async fn chain_for(&self, hook_id: &str) -> Vec<(String, String)> {
        self.entries
            .read()
            .await
            .iter()
            .filter(|(_, e)| {
                e.enabled && e.script.is_some() && e.manifest.hooks.iter().any(|h| h == hook_id)
            })
            .map(|(slug, e)| (slug.clone(), e.script.clone().unwrap_or_default()))
            .collect()
    }

    /// 执行单个插件的钩子：返回 Ok(Dynamic) 或 Err(错误消息)。
    /// 脚本源码 + 调用表达式一次性 eval（源码已在启用时通过沙箱语法编译校验）。
    fn invoke(script: &str, fn_name: &str, ctx: rhai::Map) -> Result<rhai::Dynamic, String> {
        let engine = sandbox_engine();
        let mut scope = rhai::Scope::new();
        scope.push("ctx", ctx);
        let call = format!("{script}\n{fn_name}(ctx)");
        engine
            .eval_with_scope::<rhai::Dynamic>(&mut scope, &call)
            .map_err(|e| e.to_string())
    }

    /// 执行钩子并要求返回 map（before_render / after_render / before_create）
    fn invoke_map(script: &str, fn_name: &str, ctx: rhai::Map) -> Result<rhai::Map, String> {
        Self::invoke(script, fn_name, ctx)?
            .try_cast::<rhai::Map>()
            .ok_or_else(|| "钩子必须返回 map".to_string())
    }

    /// 运行时错误：写 last_error 并打日志（不阻断主流程）
    async fn note_hook_error(&self, slug: &str, hook_id: &str, err: &str) {
        eprintln!("[reedblog] 插件 {slug} 钩子 {hook_id} 运行时错误（已跳过）: {err}");
        if let Some(entry) = self.entries.write().await.get_mut(slug) {
            entry.last_error = Some(format!("[{hook_id}] {err}"));
        }
    }

    /// 钩子成功：清掉 last_error（表明插件已恢复正常）
    async fn note_hook_ok(&self, slug: &str) {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.get_mut(slug) {
            if entry.last_error.is_some() {
                entry.last_error = None;
            }
        }
    }

    /// post.before_render 链：返回（可能被改写的 title, content_md）
    pub async fn run_post_before_render(
        &self,
        title: &str,
        content_md: &str,
        slug: &str,
    ) -> (String, String) {
        let hook_id = "post.before_render";
        let (mut out_title, mut out_md) = (title.to_string(), content_md.to_string());
        for (pslug, script) in self.chain_for(hook_id).await {
            let mut ctx = rhai::Map::new();
            ctx.insert("title".into(), out_title.clone().into());
            ctx.insert("content_md".into(), out_md.clone().into());
            ctx.insert("slug".into(), slug.to_string().into());
            match Self::invoke_map(&script, &hook_fn_name(hook_id), ctx) {
                Ok(m) => {
                    if let Some(t) = map_string(&m, "title") {
                        out_title = t;
                    }
                    if let Some(md) = map_string(&m, "content_md") {
                        out_md = md;
                    }
                    self.note_hook_ok(&pslug).await;
                }
                Err(e) => self.note_hook_error(&pslug, hook_id, &e).await,
            }
        }
        (out_title, out_md)
    }

    /// post.after_render 链：返回（可能被改写的）content_html
    pub async fn run_post_after_render(
        &self,
        title: &str,
        content_html: &str,
        slug: &str,
    ) -> String {
        let hook_id = "post.after_render";
        let mut out_html = content_html.to_string();
        for (pslug, script) in self.chain_for(hook_id).await {
            let mut ctx = rhai::Map::new();
            ctx.insert("title".into(), title.to_string().into());
            ctx.insert("content_html".into(), out_html.clone().into());
            ctx.insert("slug".into(), slug.to_string().into());
            match Self::invoke_map(&script, &hook_fn_name(hook_id), ctx) {
                Ok(m) => {
                    if let Some(h) = map_string(&m, "content_html") {
                        out_html = h;
                    }
                    self.note_hook_ok(&pslug).await;
                }
                Err(e) => self.note_hook_error(&pslug, hook_id, &e).await,
            }
        }
        out_html
    }

    /// comment.before_create 链：任一插件 block 立即短路；allow 时可携带修改后字段。
    ///
    /// 对回复（楼中楼）同样生效（契约「评论回复」条款）：parent_id/reply_to_id 为
    /// **两级归一化后**的最终存储值，以 INT 进 ctx（None → 0，扩展契约文档口径）；
    /// 只读——allow 返回 map 中即使改写也不回写存储（可改写字段仍是 author/email/content）。
    pub async fn run_comment_before_create(
        &self,
        post_slug: &str,
        author_name: &str,
        email: Option<&str>,
        content: &str,
        parent_id: Option<i64>,
        reply_to_id: Option<i64>,
    ) -> CommentDecision {
        let hook_id = "comment.before_create";
        let (mut author, mut mail, mut body) = (
            author_name.to_string(),
            email.unwrap_or_default().to_string(),
            content.to_string(),
        );
        for (pslug, script) in self.chain_for(hook_id).await {
            let mut ctx = rhai::Map::new();
            ctx.insert("post_slug".into(), post_slug.to_string().into());
            ctx.insert("author_name".into(), author.clone().into());
            ctx.insert("email".into(), mail.clone().into());
            ctx.insert("content".into(), body.clone().into());
            ctx.insert("parent_id".into(), parent_id.unwrap_or(0).into());
            ctx.insert("reply_to_id".into(), reply_to_id.unwrap_or(0).into());
            match Self::invoke_map(&script, &hook_fn_name(hook_id), ctx) {
                Ok(m) => match map_string(&m, "action").as_deref() {
                    Some("block") => {
                        self.note_hook_ok(&pslug).await;
                        return CommentDecision::Block {
                            reason: map_string(&m, "reason").unwrap_or_default(),
                        };
                    }
                    Some("allow") => {
                        if let Some(a) = map_string(&m, "author_name") {
                            author = a;
                        }
                        if let Some(e) = map_string(&m, "email") {
                            mail = e;
                        }
                        if let Some(c) = map_string(&m, "content") {
                            body = c;
                        }
                        self.note_hook_ok(&pslug).await;
                    }
                    _ => {
                        self.note_hook_error(
                            &pslug,
                            hook_id,
                            "返回值缺少 action（须为 \"allow\" 或 \"block\"）",
                        )
                        .await
                    }
                },
                Err(e) => self.note_hook_error(&pslug, hook_id, &e).await,
            }
        }
        CommentDecision::Allow {
            author_name: author,
            email: if mail.is_empty() { None } else { Some(mail) },
            content: body,
        }
    }

    /// post.after_publish 链：通知类钩子，返回值忽略（脚本内部错误照常记录）
    pub async fn run_post_after_publish(&self, title: &str, slug: &str, published_at: &str) {
        let hook_id = "post.after_publish";
        for (pslug, script) in self.chain_for(hook_id).await {
            let mut ctx = rhai::Map::new();
            ctx.insert("title".into(), title.to_string().into());
            ctx.insert("slug".into(), slug.to_string().into());
            ctx.insert("published_at".into(), published_at.to_string().into());
            match Self::invoke(&script, &hook_fn_name(hook_id), ctx) {
                Ok(_) => self.note_hook_ok(&pslug).await,
                Err(e) => self.note_hook_error(&pslug, hook_id, &e).await,
            }
        }
    }

    // ---------- 前端注入 ----------

    /// GET /api/frontend/injections：enabled 且声明了对应 inject 的插件片段，按 slug 字典序。
    /// 片段每次从磁盘读取（后台改动刷新即生效，无需重启）。
    pub async fn injections(&self) -> (Vec<Injection>, Vec<Injection>) {
        let candidates: Vec<(String, Vec<String>)> = {
            let entries = self.entries.read().await;
            entries
                .iter()
                .filter(|(_, e)| e.enabled && !e.manifest.inject.is_empty())
                .map(|(slug, e)| (slug.clone(), e.manifest.inject.clone()))
                .collect()
        };
        let mut head = Vec::new();
        let mut body_end = Vec::new();
        for (slug, inject) in candidates {
            for (pos, out) in [("head", &mut head), ("body_end", &mut body_end)] {
                if !inject.iter().any(|i| i == pos) {
                    continue;
                }
                let path = self
                    .dir
                    .join(&slug)
                    .join("inject")
                    .join(format!("{pos}.html"));
                if let Ok(html) = std::fs::read_to_string(&path) {
                    if !html.trim().is_empty() {
                        out.push(Injection {
                            plugin: slug.clone(),
                            html,
                        });
                    }
                }
            }
        }
        (head, body_end)
    }
}

/// 从插件目录读取并解析 manifest.toml
fn load_manifest_from(dir: &Path) -> Result<PluginManifest, String> {
    let text = std::fs::read_to_string(dir.join("manifest.toml"))
        .map_err(|e| format!("无法读取 manifest.toml: {e}"))?;
    toml::from_str::<PluginManifest>(&text).map_err(|e| format!("manifest.toml 解析失败: {e}"))
}

/// Rhai map 取字符串字段（非字符串/缺失 → None）
fn map_string(map: &rhai::Map, key: &str) -> Option<String> {
    map.get(key)
        .and_then(|d| d.clone().try_cast::<rhai::ImmutableString>())
        .map(|s| s.to_string())
}
