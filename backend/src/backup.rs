//! 备份与恢复领域逻辑（契约「备份与恢复」条款，2026-10-04 新增）：
//! - 导出：业务表 → data.json、计数与元信息 → manifest.json、uploads 文件原样入包（方言无关）
//! - 导入：全量校验（zip 结构/manifest/表名/行形状/类型/uploads 路径）→ 单事务重建 + 媒体替换
//!
//! 关键设计：
//! - 表清单**显式列出**（TABLES，含列名与列类型），禁止 `SELECT *`：导出/导入/校验共用同一份
//!   定义，SQLite/MySQL 双方言可跑；导入按依赖顺序插入，主键 id 原值保留。
//! - 内存有界：导出把业务表读进内存（=数据库文本量）后由 spawn_blocking 逐文件逐块写临时 zip；
//!   导入把上传 zip 流式落盘、逐条目解压到 uploads 根内 staging，绝不整读 zip 或媒体。
//! - 路径安全：uploads 条目一律经 `sanitize_upload_rel`（复用既有实现）清洗，staging 位于
//!   uploads 根内，**绝不写出 uploads 根**。
//! - 失败语义：先全量校验，任一失败 → 422 `invalid_backup` 且现有数据不变；DB 重建在单事务内，
//!   提交前失败自动回滚。

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Row};

use crate::error::{ApiError, ApiResult};
use crate::handlers::uploads::sanitize_upload_rel;
use crate::state::{now_rfc3339, AppState};

/// 备份格式版本（manifest.format_version；不兼容 → 422）
pub const FORMAT_VERSION: i64 = 1;
/// 导入请求体上限（1 GiB；超出由 DefaultBodyLimit / 逐块计数返回 413）
pub const MAX_IMPORT_BYTES: usize = 1024 * 1024 * 1024;
/// zip 条目数上限（防 zip 炸弹；媒体文件很多时 4096 不够用，放宽到 10 万）
const MAX_ZIP_ENTRIES: usize = 100_000;
/// 解压后总体积上限（防 zip 炸弹）
const MAX_UNCOMPRESSED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// manifest.json / data.json 单文件读取上限
const MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;
/// 单条 INSERT 的占位符上限（SQLite 默认 999 / MySQL 65535，取保守值分批）
const MAX_SQL_PARAMS: usize = 900;

// ---------- 表清单（显式固定；顺序即依赖顺序，删除时倒序） ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColKind {
    /// 整型列（含 0/1 布尔列；导出为 JSON number，NULL 为 null）
    Int,
    /// 文本列（含时间戳 RFC3339 文本；导出为 JSON string）
    Text,
}

pub struct TableSpec {
    pub name: &'static str,
    pub columns: &'static [(&'static str, ColKind)],
}

macro_rules! cols {
    ($(($name:literal, $kind:ident)),* $(,)?) => {
        &[$(($name, ColKind::$kind)),*]
    };
}

const CATEGORIES_COLS: &[(&str, ColKind)] = cols![("id", Int), ("name", Text)];
const TAGS_COLS: &[(&str, ColKind)] = cols![("id", Int), ("name", Text)];
const USERS_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("username", Text),
    ("password_hash", Text),
    ("created_at", Text)
];
const POSTS_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("title", Text),
    ("slug", Text),
    ("excerpt", Text),
    ("content_md", Text),
    ("status", Text),
    ("category_id", Int),
    ("published_at", Text),
    ("created_at", Text),
    ("updated_at", Text),
    ("is_sticky", Int),
    ("view_count", Int),
    ("deleted_at", Text)
];
const POST_TAGS_COLS: &[(&str, ColKind)] = cols![("post_id", Int), ("tag_id", Int)];
const COMMENTS_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("post_id", Int),
    ("author_name", Text),
    ("email", Text),
    ("content", Text),
    ("status", Text),
    ("created_at", Text),
    ("target_type", Text),
    ("parent_id", Int),
    ("reply_to_id", Int)
];
const PAGES_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("title", Text),
    ("slug", Text),
    ("kind", Text),
    ("content_md", Text),
    ("content_html", Text),
    ("enabled", Int),
    ("sort_order", Int),
    ("built_in", Int),
    ("created_at", Text),
    ("updated_at", Text)
];
const PAGE_LINKS_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("page_id", Int),
    ("name", Text),
    ("url", Text),
    ("description", Text),
    ("sort_order", Int)
];
const PLUGINS_COLS: &[(&str, ColKind)] = cols![
    ("slug", Text),
    ("enabled", Int),
    ("installed_at", Text),
    ("updated_at", Text)
];
const SETTINGS_COLS: &[(&str, ColKind)] =
    cols![("name", Text), ("value", Text), ("updated_at", Text)];
const THEME_SETTINGS_COLS: &[(&str, ColKind)] = cols![
    ("theme_slug", Text),
    ("key", Text),
    ("value", Text),
    ("updated_at", Text)
];
const THEME_WIDGETS_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("theme_slug", Text),
    ("widget_key", Text),
    ("enabled", Int),
    ("position", Text),
    ("sort_order", Int),
    ("config", Text),
    ("created_at", Text),
    ("updated_at", Text)
];
const POST_LIKES_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("post_id", Int),
    ("liker_key", Text),
    ("created_at", Text)
];
const MEDIA_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("url", Text),
    ("filename", Text),
    ("size", Int),
    ("mime", Text),
    ("width", Int),
    ("height", Int),
    ("created_at", Text)
];
const POST_REVISIONS_COLS: &[(&str, ColKind)] = cols![
    ("id", Int),
    ("post_id", Int),
    ("title", Text),
    ("content_md", Text),
    ("excerpt", Text),
    ("created_at", Text)
];

/// 全部业务表（顺序 = 导入插入顺序；删除时倒序）。
/// 表中每一列都必须在此列出（禁止 SELECT *：导出/导入/校验共用这一份定义）。
pub const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "categories",
        columns: CATEGORIES_COLS,
    },
    TableSpec {
        name: "tags",
        columns: TAGS_COLS,
    },
    TableSpec {
        name: "users",
        columns: USERS_COLS,
    },
    TableSpec {
        name: "posts",
        columns: POSTS_COLS,
    },
    TableSpec {
        name: "post_tags",
        columns: POST_TAGS_COLS,
    },
    TableSpec {
        name: "comments",
        columns: COMMENTS_COLS,
    },
    TableSpec {
        name: "pages",
        columns: PAGES_COLS,
    },
    TableSpec {
        name: "page_links",
        columns: PAGE_LINKS_COLS,
    },
    TableSpec {
        name: "plugins",
        columns: PLUGINS_COLS,
    },
    TableSpec {
        name: "settings",
        columns: SETTINGS_COLS,
    },
    TableSpec {
        name: "theme_settings",
        columns: THEME_SETTINGS_COLS,
    },
    TableSpec {
        name: "theme_widgets",
        columns: THEME_WIDGETS_COLS,
    },
    TableSpec {
        name: "post_likes",
        columns: POST_LIKES_COLS,
    },
    TableSpec {
        name: "media",
        columns: MEDIA_COLS,
    },
    TableSpec {
        name: "post_revisions",
        columns: POST_REVISIONS_COLS,
    },
];

fn table_spec(name: &str) -> Option<&'static TableSpec> {
    TABLES.iter().find(|t| t.name == name)
}

// ---------- 错误与守卫 ----------

pub(crate) fn invalid_backup(msg: impl Into<String>) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_backup",
        msg,
    )
}

pub(crate) fn confirmation_required() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "confirmation_required",
        "危险操作需显式确认：multipart 字段 confirm 必须为字面量 REPLACE",
    )
}

pub(crate) fn payload_too_large() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        format!(
            "备份文件超出大小上限（{} MiB）",
            MAX_IMPORT_BYTES / (1024 * 1024)
        ),
    )
}

fn rand_hex() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 临时文件守卫：Drop 时删除（成功、失败、客户端断开/流被丢弃都兜底）
pub(crate) struct TempPath(PathBuf);

impl TempPath {
    /// 在系统临时目录生成唯一路径（文件由调用方创建/读取）
    pub(crate) fn new(tag: &str) -> Self {
        TempPath(std::env::temp_dir().join(format!("reedblog-{tag}-{}.tmp", rand_hex())))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// staging 目录守卫：Drop 时递归删除
struct TempDir(PathBuf);

impl TempDir {
    /// 在 parent 下创建 `.tmp-backup-<hex>` 一次性目录（位于 uploads 根内，
    /// 保证落盘阶段 rename 不跨文件系统；collect_media_files 会跳过 .tmp-* 目录）
    fn new_in(parent: &Path, tag: &str) -> ApiResult<Self> {
        std::fs::create_dir_all(parent)?;
        let dir = parent.join(format!(".tmp-{tag}-{}", rand_hex()));
        std::fs::create_dir_all(&dir)?;
        Ok(TempDir(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------- 导出 ----------

/// 递归收集 uploads 根下全部普通文件（正斜杠相对路径，字典序）；
/// 跳过 `.tmp-*` 暂存目录与符号链接（符号链接不导出/不参与清理）。
/// 根目录不存在 → 空清单（无媒体，不算失败）。
fn collect_media_files(root: &Path) -> ApiResult<Vec<String>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // 目录不可读：跳过该目录（不因权限问题让整个导出失败）
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(".tmp-") {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if ft.is_dir() {
                stack.push((entry.path(), rel));
            } else if ft.is_file() {
                out.push(rel);
            }
            // 符号链接与其他特殊文件：跳过
        }
    }
    out.sort();
    Ok(out)
}

/// 整型列读取：优先 i64；MySQL TINYINT(1) 经 Any 驱动可能按 bool 解码（同仓库其他读取点惯例）
fn int_cell(row: &AnyRow, idx: usize) -> Result<Value, sqlx::Error> {
    match row.try_get::<Option<i64>, _>(idx) {
        Ok(v) => Ok(v.map(Value::from).unwrap_or(Value::Null)),
        Err(_) => {
            let b = row.try_get::<Option<bool>, _>(idx)?;
            Ok(match b {
                Some(b) => Value::from(if b { 1 } else { 0 }),
                None => Value::Null,
            })
        }
    }
}

fn row_to_json(row: &AnyRow, spec: &TableSpec) -> Result<Value, sqlx::Error> {
    let mut map = Map::new();
    for (idx, (name, kind)) in spec.columns.iter().enumerate() {
        let v = match kind {
            ColKind::Int => int_cell(row, idx)?,
            ColKind::Text => row
                .try_get::<Option<String>, _>(idx)?
                .map(Value::from)
                .unwrap_or(Value::Null),
        };
        map.insert((*name).to_string(), v);
    }
    Ok(Value::Object(map))
}

/// 按显式列清单读取单表全部行（列名反引号引用，`key` 等关键字列双方言安全）
async fn fetch_table_rows(pool: &AnyPool, spec: &TableSpec) -> ApiResult<Vec<Value>> {
    let cols = spec
        .columns
        .iter()
        .map(|(c, _)| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {cols} FROM `{}`", spec.name);
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(row_to_json(row, spec)?);
    }
    Ok(out)
}

/// 阻塞式写出备份 zip（在 spawn_blocking 中执行）：data.json → manifest.json → uploads/**
/// 媒体逐块复制（std::io::copy），任何时刻内存只驻留一个复制缓冲。
fn write_backup_zip(
    path: &Path,
    db_type: &str,
    exported_at: &str,
    tables: &[(&'static TableSpec, Vec<Value>)],
    media: &[String],
    uploads_root: &Path,
) -> ApiResult<u64> {
    let zip_err = |e: zip::result::ZipError| ApiError::internal(format!("生成备份 zip 失败: {e}"));

    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();

    // data.json：逐表序列化（不整包拼字符串）
    zip.start_file("data.json", opts).map_err(zip_err)?;
    zip.write_all(b"{")?;
    for (i, (spec, rows)) in tables.iter().enumerate() {
        if i > 0 {
            zip.write_all(b",")?;
        }
        zip.write_all(format!("\"{}\":", spec.name).as_bytes())?;
        serde_json::to_writer(&mut zip, rows)
            .map_err(|e| ApiError::internal(format!("序列化 {} 失败: {e}", spec.name)))?;
    }
    zip.write_all(b"}")?;

    // manifest.json：表计数以实际导出行为准
    let mut counts = BTreeMap::new();
    for (spec, rows) in tables {
        counts.insert(spec.name, rows.len() as i64);
    }
    let manifest = serde_json::json!({
        "format_version": FORMAT_VERSION,
        "exported_at": exported_at,
        "db_type": db_type,
        "tables": counts,
        "media_files": media.len() as i64,
        "app_version": env!("CARGO_PKG_VERSION"),
    });
    zip.start_file("manifest.json", opts).map_err(zip_err)?;
    serde_json::to_writer(&mut zip, &manifest)
        .map_err(|e| ApiError::internal(format!("序列化 manifest 失败: {e}")))?;

    // uploads/**：保持相对路径原样复制
    for rel in media {
        zip.start_file(format!("uploads/{rel}"), opts)
            .map_err(zip_err)?;
        let mut src = std::fs::File::open(uploads_root.join(rel))?;
        std::io::copy(&mut src, &mut zip)?;
    }

    let file = zip.finish().map_err(zip_err)?;
    file.sync_all()?;
    Ok(file.metadata()?.len())
}

/// 导出产物：临时 zip 文件（Drop 兜底删除）+ 大小 + 导出时刻
pub(crate) struct ExportFile {
    pub(crate) tmp: TempPath,
    pub(crate) size: u64,
    pub(crate) exported_at: String,
}

/// 生成备份 zip 到临时文件（供 handlers::backup::admin_export_backup 流出）
pub(crate) async fn build_export(
    state: &AppState,
    pool: &AnyPool,
    db_type: &str,
) -> ApiResult<ExportFile> {
    // 1) 异步阶段：业务表整表读取（导出不落库、不加锁；内存 = 数据库文本量）
    let mut tables: Vec<(&'static TableSpec, Vec<Value>)> = Vec::with_capacity(TABLES.len());
    for spec in TABLES {
        let rows = fetch_table_rows(pool, spec).await?;
        tables.push((spec, rows));
    }
    // 2) uploads 文件清单（跳过符号链接与 .tmp-* 暂存目录）
    let media = collect_media_files(state.uploads_dir())?;

    let exported_at = now_rfc3339();
    let tmp = TempPath::new("backup");
    let path = tmp.path().to_path_buf();
    let uploads_root = state.uploads_dir().to_path_buf();
    let db_type_for_zip = db_type.to_string();
    let exported_at_for_zip = exported_at.clone();

    // 3) 阻塞阶段：写临时 zip（媒体逐块，不整读）
    let size = tokio::task::spawn_blocking(move || {
        write_backup_zip(
            &path,
            &db_type_for_zip,
            &exported_at_for_zip,
            &tables,
            &media,
            &uploads_root,
        )
    })
    .await
    .map_err(|e| ApiError::internal(format!("备份生成任务失败: {e}")))??;

    Ok(ExportFile {
        tmp,
        size,
        exported_at,
    })
}

// ---------- 导入 ----------

#[derive(Debug, Deserialize)]
struct Manifest {
    format_version: i64,
    exported_at: String,
    /// 备份来源库类型（仅记录，不参与校验——允许跨库恢复）
    #[serde(default)]
    #[allow(dead_code)]
    db_type: String,
    tables: BTreeMap<String, i64>,
    #[serde(default)]
    media_files: i64,
    /// 备份来源应用版本（仅记录，不参与校验）
    #[serde(default)]
    #[allow(dead_code)]
    app_version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BackupImportResult {
    pub ok: bool,
    pub format_version: i64,
    pub exported_at: String,
    pub tables: BTreeMap<String, i64>,
    pub media_files: i64,
}

/// GET /api/admin/backup/info 响应形状（内存态：最近一次导出；重启清零）
#[derive(Debug, Serialize)]
pub struct BackupInfo {
    pub last_export_at: Option<String>,
    pub total_size_bytes: Option<u64>,
}

/// 校验单表全部行：键集合必须与表列完全一致；整型列只接受整数、文本列只接受字符串（NULL 允许）
fn validate_rows(spec: &TableSpec, rows: &[Value]) -> ApiResult<()> {
    for (i, row) in rows.iter().enumerate() {
        let Some(obj) = row.as_object() else {
            return Err(invalid_backup(format!(
                "data.json 表 {} 第 {} 行不是对象",
                spec.name, i
            )));
        };
        if obj.len() != spec.columns.len()
            || spec.columns.iter().any(|(c, _)| !obj.contains_key(*c))
        {
            return Err(invalid_backup(format!(
                "data.json 表 {} 第 {} 行的列集合与表结构不符",
                spec.name, i
            )));
        }
        for (col, kind) in spec.columns {
            let v = &obj[*col];
            let ok = match kind {
                ColKind::Int => v.is_null() || v.as_i64().is_some(),
                ColKind::Text => v.is_null() || v.is_string(),
            };
            if !ok {
                return Err(invalid_backup(format!(
                    "data.json 表 {}.{} 第 {} 行值类型不符",
                    spec.name, col, i
                )));
            }
        }
    }
    Ok(())
}

/// 从 zip 条目读取受限长度的字节（manifest.json / data.json）
fn read_entry_limited<R: Read>(entry: &mut R, max: u64) -> ApiResult<Vec<u8>> {
    let mut buf = Vec::new();
    entry
        .take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|e| invalid_backup(format!("zip 条目读取失败: {e}")))?;
    if buf.len() as u64 > max {
        return Err(invalid_backup("备份内 JSON 文件超出大小上限"));
    }
    Ok(buf)
}

/// 导入阶段解压出的 uploads 条目集合（相对 uploads 根、正斜杠路径）
type UploadSet = BTreeSet<String>;

/// 打开 zip 并完成**全部校验**，把 uploads 条目流式解到 staging。
/// 任一失败返回 422（调用方保证尚不触碰数据库/现有文件）。
fn validate_and_stage(
    zip_path: &Path,
    staging: &Path,
) -> ApiResult<(Manifest, BTreeMap<String, Vec<Value>>, UploadSet)> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| invalid_backup(format!("zip 解析失败: {e}")))?;
    if archive.len() == 0 {
        return Err(invalid_backup("zip 内容为空"));
    }
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(invalid_backup("zip 条目数超出上限"));
    }

    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut data_bytes: Option<Vec<u8>> = None;
    let mut uploads: UploadSet = BTreeSet::new();
    let mut declared_total: u64 = 0;
    let mut actual_total: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| invalid_backup(format!("zip 条目读取失败: {e}")))?;
        declared_total = declared_total.saturating_add(entry.size());
        if declared_total > MAX_UNCOMPRESSED_BYTES {
            return Err(invalid_backup("备份解压后体积超出上限"));
        }

        // 条目名归一化（zip 规范为正斜杠；部分工具写反斜杠）
        let raw_name = entry.name().replace('\\', "/");
        let is_dir = entry.is_dir();

        if raw_name == "manifest.json" {
            manifest_bytes = Some(read_entry_limited(&mut entry, MAX_JSON_BYTES)?);
        } else if raw_name == "data.json" {
            data_bytes = Some(read_entry_limited(&mut entry, MAX_JSON_BYTES)?);
        } else if let Some(rel_raw) = raw_name.strip_prefix("uploads/") {
            if is_dir {
                // 目录条目：只校验路径，不落盘
                if !rel_raw.is_empty() && sanitize_upload_rel(rel_raw).is_none() {
                    return Err(invalid_backup(format!(
                        "备份含非法 uploads 路径: {raw_name}"
                    )));
                }
                continue;
            }
            let Some(rel) = sanitize_upload_rel(rel_raw) else {
                return Err(invalid_backup(format!(
                    "备份含非法 uploads 路径: {raw_name}"
                )));
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !uploads.insert(rel.clone()) {
                return Err(invalid_backup(format!(
                    "备份含重复的 uploads 条目: {raw_name}"
                )));
            }
            let target = staging.join(&rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&target)?;
            // 逐块复制（std::io::copy），实际字节数二次计量（防声明 size 造假）
            actual_total = actual_total.saturating_add(std::io::copy(&mut entry, &mut out)?);
            if actual_total > MAX_UNCOMPRESSED_BYTES {
                return Err(invalid_backup("备份解压后体积超出上限"));
            }
        } else {
            return Err(invalid_backup(format!("备份含未知条目: {raw_name}")));
        }
    }

    let manifest_bytes = manifest_bytes.ok_or_else(|| invalid_backup("备份缺少 manifest.json"))?;
    let data_bytes = data_bytes.ok_or_else(|| invalid_backup("备份缺少 data.json"))?;

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| invalid_backup(format!("manifest.json 解析失败: {e}")))?;
    if manifest.format_version != FORMAT_VERSION {
        return Err(invalid_backup(format!(
            "备份格式版本不兼容（备份为 {}，当前支持 {}）",
            manifest.format_version, FORMAT_VERSION
        )));
    }

    let data: BTreeMap<String, Vec<Value>> = serde_json::from_slice(&data_bytes)
        .map_err(|e| invalid_backup(format!("data.json 解析失败: {e}")))?;

    // manifest 与 data.json 完整性一致（表集合与行数）
    if manifest.tables.len() != data.len()
        || manifest
            .tables
            .iter()
            .any(|(k, v)| data.get(k).map(|rows| rows.len() as i64) != Some(*v))
    {
        return Err(invalid_backup("manifest 表计数与 data.json 不一致"));
    }
    // 表名在允许集合内 + 每行列集合/值类型校验
    for (name, rows) in &data {
        let Some(spec) = table_spec(name) else {
            return Err(invalid_backup(format!("data.json 含未知表: {name}")));
        };
        validate_rows(spec, rows)?;
    }
    // 至少一个用户：否则导入后无人能登录（安装即有管理员，正常备份必含 users 行）
    if data
        .get("users")
        .map(|rows| rows.is_empty())
        .unwrap_or(true)
    {
        return Err(invalid_backup("备份不含任何用户，导入后将无法登录"));
    }
    if manifest.media_files != uploads.len() as i64 {
        return Err(invalid_backup(
            "manifest 媒体文件数与备份内 uploads 条目不一致",
        ));
    }

    Ok((manifest, data, uploads))
}

/// 按依赖顺序批量 INSERT（分批规避占位符上限）；行已经过 validate_rows 校验
async fn insert_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    spec: &TableSpec,
    rows: &[Value],
) -> Result<(), sqlx::Error> {
    if rows.is_empty() {
        return Ok(());
    }
    let cols = spec
        .columns
        .iter()
        .map(|(c, _)| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let row_placeholder = format!("({})", vec!["?"; spec.columns.len()].join(", "));
    let batch = std::cmp::max(1, MAX_SQL_PARAMS / spec.columns.len());

    for chunk in rows.chunks(batch) {
        let values = vec![row_placeholder.as_str(); chunk.len()].join(", ");
        let sql = format!("INSERT INTO `{}` ({cols}) VALUES {values}", spec.name);
        let mut q = sqlx::query(&sql);
        for row in chunk {
            // 校验阶段已保证形状；此处 expect 仅为类型收窄
            let obj = row.as_object().expect("导入行已经过校验");
            for (col, kind) in spec.columns {
                let v = &obj[*col];
                match kind {
                    ColKind::Int => q = q.bind(v.as_i64()),
                    ColKind::Text => q = q.bind(v.as_str().map(str::to_string)),
                }
            }
        }
        q.execute(&mut **tx).await?;
    }
    Ok(())
}

/// 把 staging 内文件先写临时再原子替换到 uploads 根，并删除备份外文件（磁盘回到备份时点）
fn place_uploads(uploads_root: &Path, staging: &Path, uploads: &UploadSet) -> ApiResult<()> {
    std::fs::create_dir_all(uploads_root)?;
    for rel in uploads {
        let src = staging.join(rel);
        let dst = uploads_root.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Windows 上 rename 不覆盖已存在目标：先移除旧文件（staging 已是完整临时副本）
        match std::fs::remove_file(&dst) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ApiError::internal(format!("替换媒体文件失败: {e}"))),
        }
        std::fs::rename(&src, &dst)
            .map_err(|e| ApiError::internal(format!("落盘媒体文件失败: {e}")))?;
    }
    // 删除备份外文件（符号链接与 .tmp-* 目录不动），使 uploads 与备份时点一致
    for rel in collect_media_files(uploads_root)? {
        if !uploads.contains(&rel) {
            std::fs::remove_file(uploads_root.join(&rel))
                .map_err(|e| ApiError::internal(format!("清理备份外文件失败: {e}")))?;
        }
    }
    Ok(())
}

/// 用已落盘的 zip 恢复整个站点数据（调用方完成鉴权、confirm 校验与上传落盘）：
/// 全量校验 → staging → 单事务重建 → 媒体替换 → 插件状态重同步。
pub(crate) async fn import_backup(
    state: &AppState,
    pool: &AnyPool,
    zip_path: &Path,
) -> ApiResult<BackupImportResult> {
    // 全量校验 + staging（失败 → 422，现有数据/文件不变）
    let staging = TempDir::new_in(state.uploads_dir(), "backup")?;
    let (manifest, data, uploads) = validate_and_stage(zip_path, staging.path())?;

    // 单事务重建：倒序清空 → 顺序插入（主键保留原值）
    let mut tx = pool.begin().await?;
    for spec in TABLES.iter().rev() {
        sqlx::query(&format!("DELETE FROM `{}`", spec.name))
            .execute(&mut *tx)
            .await?;
    }
    for spec in TABLES {
        let empty: Vec<Value> = Vec::new();
        let rows = data.get(spec.name).unwrap_or(&empty);
        insert_rows(&mut tx, spec, rows).await?;
    }
    tx.commit().await?;

    // 媒体替换（数据库已是备份状态；失败 → 500，可重试导入）
    place_uploads(state.uploads_dir(), staging.path(), &uploads)?;
    drop(staging); // 显式清理 staging（Drop 兜底）

    // 插件启用状态按恢复出的 plugins 表重新同步（内存注册表）
    state.plugins().restore_from_db(pool).await;

    Ok(BackupImportResult {
        ok: true,
        format_version: manifest.format_version,
        exported_at: manifest.exported_at,
        tables: manifest.tables,
        media_files: uploads.len() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_specs_unique_and_columns_covered() {
        let mut names = BTreeSet::new();
        for spec in TABLES {
            assert!(names.insert(spec.name), "表名重复: {}", spec.name);
            assert!(!spec.columns.is_empty(), "表 {} 无列定义", spec.name);
            let mut cols = BTreeSet::new();
            for (c, _) in spec.columns {
                assert!(cols.insert(*c), "表 {} 列重复: {c}", spec.name);
            }
        }
        // 15 张业务表显式固定（新增表必须同步更新契约与这里）
        assert_eq!(TABLES.len(), 15);
    }

    #[test]
    fn validate_rows_rejects_shape_and_type_mismatch() {
        let spec = table_spec("categories").unwrap();
        // 正常行
        assert!(validate_rows(spec, &[serde_json::json!({"id": 1, "name": "默认"})]).is_ok());
        // 缺列
        assert!(validate_rows(spec, &[serde_json::json!({"id": 1})]).is_err());
        // 多列
        assert!(validate_rows(spec, &[serde_json::json!({"id": 1, "name": "a", "x": 1})]).is_err());
        // 类型不符（整型列给字符串）
        assert!(validate_rows(spec, &[serde_json::json!({"id": "1", "name": "a"})]).is_err());
        // NULL 允许
        assert!(validate_rows(spec, &[serde_json::json!({"id": null, "name": null})]).is_ok());
    }

    #[test]
    fn sanitize_upload_rel_blocks_traversal_for_backup_entries() {
        assert!(sanitize_upload_rel("../../evil.txt").is_none());
        assert!(sanitize_upload_rel("2026/10/a.png").is_some());
        assert!(sanitize_upload_rel("C:/windows/win.ini").is_none());
    }
}
