//! 主题设置（docs/extensibility-contract.md「主题设置项」条款）：
//! - theme_settings 表读写：SQLite/MySQL 共用一份 SQL（Any 驱动、`?` 占位符），
//!   upsert 与 settings 表同款「先 UPDATE，rows_affected=0 再 INSERT」；
//!   `key` 为 SQL 关键字，列名统一反引号引用（双方言均支持）；
//! - PUT 入参校验与类型转换：未声明 key → 422 unknown_setting；
//!   值与类型不符 / select 越界 / color 非 hex / 超长 → 422 invalid_value；
//! - 生效值合并：声明 default 与已存值合并（已存值优先），按类型输出 JSON
//!   （switch → bool、number → number、其余 → string）；
//! - 删除主题时连带删除其行（契约：卸载即删除设置）。

use axum::http::StatusCode;
use serde_json::{Map, Value};
use sqlx::{AnyPool, Row};
use std::collections::BTreeMap;

use crate::error::{ApiError, ApiResult};
use crate::state::now_rfc3339;
use crate::themes::{valid_hex_color, ThemeSettingDecl};

fn invalid_value(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_value", msg)
}

fn unknown_setting(key: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown_setting",
        format!("主题未声明设置项 '{key}'"),
    )
}

/// PUT 入参值 → 规范化存储字符串（契约「值的存储与类型转换」；失败 422 invalid_value）
pub fn normalize_value(decl: &ThemeSettingDecl, input: &Value) -> ApiResult<String> {
    let key = &decl.key;
    match decl.kind.as_str() {
        "switch" => {
            let b = match input {
                Value::Bool(b) => *b,
                Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(invalid_value(format!(
                            "设置项 '{key}'（switch）只接受布尔值"
                        )))
                    }
                },
                _ => {
                    return Err(invalid_value(format!(
                        "设置项 '{key}'（switch）只接受布尔值"
                    )))
                }
            };
            Ok(if b { "true" } else { "false" }.to_string())
        }
        "number" => {
            let f = match input {
                Value::Number(n) => n.as_f64().ok_or_else(|| {
                    invalid_value(format!("设置项 '{key}'（number）数值超出范围"))
                })?,
                Value::String(s) => s
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| invalid_value(format!("设置项 '{key}'（number）不是合法数字")))?,
                _ => return Err(invalid_value(format!("设置项 '{key}'（number）只接受数字"))),
            };
            if !f.is_finite() {
                return Err(invalid_value(format!(
                    "设置项 '{key}'（number）不是有限数字"
                )));
            }
            // 规范化：整数值存 i64 串，其余存最短 f64 表示
            Ok(
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    (f as i64).to_string()
                } else {
                    format!("{f}")
                },
            )
        }
        "select" => {
            let s = input
                .as_str()
                .ok_or_else(|| invalid_value(format!("设置项 '{key}'（select）只接受字符串")))?;
            let opts = decl.options.as_deref().unwrap_or(&[]);
            if !opts.iter().any(|o| o.value == s) {
                return Err(invalid_value(format!(
                    "设置项 '{key}' 的值 '{s}' 不在 options 范围内"
                )));
            }
            Ok(s.to_string())
        }
        "color" => {
            let s = input
                .as_str()
                .map(str::trim)
                .ok_or_else(|| invalid_value(format!("设置项 '{key}'（color）只接受字符串")))?;
            if !valid_hex_color(s) {
                return Err(invalid_value(format!(
                    "设置项 '{key}' 的值 '{s}' 不是合法 hex 颜色（#RGB/#RRGGBB/#RRGGBBAA）"
                )));
            }
            Ok(s.to_ascii_lowercase())
        }
        // text / textarea
        other => {
            let s = match input {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => {
                    return Err(invalid_value(format!(
                        "设置项 '{key}'（{other}）只接受字符串值"
                    )))
                }
            };
            let max = if other == "textarea" { 5000 } else { 500 };
            if s.chars().count() > max {
                return Err(invalid_value(format!(
                    "设置项 '{key}' 过长（上限 {max} 字符）"
                )));
            }
            Ok(s)
        }
    }
}

/// 存储字符串 → 响应 JSON 值（按声明类型）；存储值损坏 → None（调用方回退 default）
pub fn stored_to_json(decl: &ThemeSettingDecl, stored: &str) -> Option<Value> {
    match decl.kind.as_str() {
        "switch" => Some(Value::Bool(stored == "true")),
        "number" => stored.parse::<i64>().ok().map(Value::from).or_else(|| {
            stored
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
        }),
        _ => Some(Value::String(stored.to_string())),
    }
}

/// 校验整个 PUT 入参 values（须为 JSON 对象；key 必须在声明内）→ 规范化后的存储 map
pub fn validate_values(
    decls: &[ThemeSettingDecl],
    body_values: &Value,
) -> ApiResult<BTreeMap<String, String>> {
    let obj = body_values
        .as_object()
        .ok_or_else(|| ApiError::validation("values 必须是 JSON 对象"))?;
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        let decl = decls
            .iter()
            .find(|d| d.key == *k)
            .ok_or_else(|| unknown_setting(k))?;
        out.insert(k.clone(), normalize_value(decl, v)?);
    }
    Ok(out)
}

/// 声明 default 与已存值合并 → 生效值对象（已存值优先；两者皆无的 key 不出现）
pub fn merged_values(decls: &[ThemeSettingDecl], stored: &BTreeMap<String, String>) -> Value {
    let mut map = Map::new();
    for decl in decls {
        if let Some(s) = stored.get(&decl.key) {
            if let Some(v) = stored_to_json(decl, s) {
                map.insert(decl.key.clone(), v);
                continue;
            }
        }
        if let Some(d) = &decl.default {
            map.insert(decl.key.clone(), d.clone());
        }
    }
    Value::Object(map)
}

// ---------- theme_settings 表 ----------

/// 读取某主题的全部已存值（按 slug 隔离）
pub async fn load_stored(
    pool: &AnyPool,
    slug: &str,
) -> Result<BTreeMap<String, String>, sqlx::Error> {
    let rows = sqlx::query("SELECT `key`, value FROM theme_settings WHERE theme_slug = ?")
        .bind(slug)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<String, _>("key"), r.get::<String, _>("value")))
        .collect())
}

/// 批量 upsert（先 UPDATE，rows_affected=0 再 INSERT；双方言共用，不用单方言冲突子句）
pub async fn upsert_many(
    pool: &AnyPool,
    slug: &str,
    values: &BTreeMap<String, String>,
) -> Result<(), sqlx::Error> {
    let now = now_rfc3339();
    for (key, value) in values {
        let updated = sqlx::query(
            "UPDATE theme_settings SET value = ?, updated_at = ? WHERE theme_slug = ? AND `key` = ?",
        )
        .bind(value)
        .bind(&now)
        .bind(slug)
        .bind(key)
        .execute(pool)
        .await?
        .rows_affected();
        if updated == 0 {
            sqlx::query(
                "INSERT INTO theme_settings (theme_slug, `key`, value, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(slug)
            .bind(key)
            .bind(value)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// 删除主题的全部设置行（卸载主题时调用；契约：卸载即删除设置）
pub async fn delete_for_theme(pool: &AnyPool, slug: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM theme_settings WHERE theme_slug = ?")
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::ThemeManifest;

    fn decls() -> Vec<ThemeSettingDecl> {
        let toml_text = r##"
name = "t"
slug = "t"
version = "1.0.0"

[[settings]]
key = "layout"
type = "select"
default = "a"
options = ["a", { value = "b", label = "B 布局" }]

[[settings]]
key = "accent_color"
type = "color"
default = "#0f172a"

[[settings]]
key = "wide"
type = "switch"
default = false

[[settings]]
key = "cols"
type = "number"

[[settings]]
key = "note"
type = "textarea"
"##;
        let m: ThemeManifest = toml::from_str(toml_text).unwrap();
        assert!(m.validate_settings().is_ok());
        m.normalized_settings()
    }

    fn decl<'a>(decls: &'a [ThemeSettingDecl], key: &str) -> &'a ThemeSettingDecl {
        decls.iter().find(|d| d.key == key).unwrap()
    }

    #[test]
    fn options_normalized_and_labels_defaulted() {
        let d = decls();
        let layout = decl(&d, "layout");
        let opts = layout.options.as_ref().unwrap();
        assert_eq!(opts[0].value, "a");
        assert_eq!(opts[0].label, "a"); // 字符串写法 label=value
        assert_eq!(opts[1].label, "B 布局");
        assert_eq!(layout.label, "layout"); // label 缺省填 key
    }

    #[test]
    fn normalize_values_by_type() {
        let d = decls();
        assert_eq!(
            normalize_value(decl(&d, "layout"), &Value::from("b")).unwrap(),
            "b"
        );
        assert!(normalize_value(decl(&d, "layout"), &Value::from("c")).is_err());
        assert_eq!(
            normalize_value(decl(&d, "accent_color"), &Value::from("#ABC")).unwrap(),
            "#abc"
        );
        assert!(normalize_value(decl(&d, "accent_color"), &Value::from("red")).is_err());
        assert_eq!(
            normalize_value(decl(&d, "wide"), &Value::from(true)).unwrap(),
            "true"
        );
        assert_eq!(
            normalize_value(decl(&d, "wide"), &Value::from("False")).unwrap(),
            "false"
        );
        assert!(normalize_value(decl(&d, "wide"), &Value::from("yes")).is_err());
        assert_eq!(
            normalize_value(decl(&d, "cols"), &Value::from(3)).unwrap(),
            "3"
        );
        assert_eq!(
            normalize_value(decl(&d, "cols"), &Value::from("2.5")).unwrap(),
            "2.5"
        );
        assert!(normalize_value(decl(&d, "cols"), &Value::from("x")).is_err());
        assert!(normalize_value(decl(&d, "note"), &Value::from("长".repeat(5001))).is_err());
        assert_eq!(
            normalize_value(decl(&d, "note"), &Value::from("hi")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn unknown_key_rejected() {
        let d = decls();
        let err = validate_values(&d, &serde_json::json!({"nope": "x"})).unwrap_err();
        assert_eq!(err.code, "unknown_setting");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn merged_values_prefer_stored_and_type_output() {
        let d = decls();
        let mut stored = BTreeMap::new();
        stored.insert("wide".to_string(), "true".to_string());
        stored.insert("cols".to_string(), "4".to_string());
        stored.insert("layout".to_string(), "b".to_string());
        let v = merged_values(&d, &stored);
        assert_eq!(v["wide"], Value::Bool(true));
        assert_eq!(v["cols"], Value::from(4));
        assert_eq!(v["layout"], Value::from("b"));
        assert_eq!(v["accent_color"], Value::from("#0f172a")); // 回退 default
        assert!(v.get("note").is_none()); // 无 default 无存储 → 不出现
    }

    #[test]
    fn setting_decl_validation_errors() {
        // 未知 type
        let m: ThemeManifest = toml::from_str(
            "name=\"t\"\nslug=\"t\"\nversion=\"1.0.0\"\n[[settings]]\nkey=\"a\"\ntype=\"bogus\"\n",
        )
        .unwrap();
        assert!(m.validate_settings().is_err());
        // key 重复
        let m: ThemeManifest = toml::from_str(
            "name=\"t\"\nslug=\"t\"\nversion=\"1.0.0\"\n[[settings]]\nkey=\"a\"\ntype=\"text\"\n[[settings]]\nkey=\"a\"\ntype=\"text\"\n",
        )
        .unwrap();
        assert!(m.validate_settings().is_err());
        // select 缺 options
        let m: ThemeManifest = toml::from_str(
            "name=\"t\"\nslug=\"t\"\nversion=\"1.0.0\"\n[[settings]]\nkey=\"a\"\ntype=\"select\"\n",
        )
        .unwrap();
        assert!(m.validate_settings().is_err());
        // 非 select 带 options
        let m: ThemeManifest = toml::from_str(
            "name=\"t\"\nslug=\"t\"\nversion=\"1.0.0\"\n[[settings]]\nkey=\"a\"\ntype=\"text\"\noptions=[\"x\"]\n",
        )
        .unwrap();
        assert!(m.validate_settings().is_err());
        // default 类型不符
        let m: ThemeManifest = toml::from_str(
            "name=\"t\"\nslug=\"t\"\nversion=\"1.0.0\"\n[[settings]]\nkey=\"a\"\ntype=\"switch\"\ndefault=\"yes\"\n",
        )
        .unwrap();
        assert!(m.validate_settings().is_err());
        // select default 越界
        let m: ThemeManifest = toml::from_str(
            "name=\"t\"\nslug=\"t\"\nversion=\"1.0.0\"\n[[settings]]\nkey=\"a\"\ntype=\"select\"\ndefault=\"z\"\noptions=[\"x\"]\n",
        )
        .unwrap();
        assert!(m.validate_settings().is_err());
        // key 非法
        let m: ThemeManifest = toml::from_str(
            "name=\"t\"\nslug=\"t\"\nversion=\"1.0.0\"\n[[settings]]\nkey=\"Bad Key\"\ntype=\"text\"\n",
        )
        .unwrap();
        assert!(m.validate_settings().is_err());
    }

    #[test]
    fn builtin_default_theme_toml_is_valid_and_has_settings() {
        let m = crate::themes::builtin_default_manifest();
        assert!(m.validate("default").is_ok());
        let d = m.normalized_settings();
        let layout = d
            .iter()
            .find(|s| s.key == "layout")
            .expect("default 须声明 layout");
        assert_eq!(layout.kind, "select");
        let opts = layout.options.as_ref().unwrap();
        assert_eq!(opts[0].value, "topbar-two-column");
        assert_eq!(opts[1].value, "topbar-minimal-three-column");
        assert_eq!(layout.default.as_ref().unwrap(), "topbar-two-column");
        // 至少 2 个配色/开关类示例设置
        assert!(d.iter().any(|s| s.kind == "switch"));
        assert!(d.iter().any(|s| s.kind == "color"));
    }
}
