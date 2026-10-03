-- 主题组件（契约「主题组件」条款）：按主题 slug 隔离的 widget 配置行。
-- 只存覆盖行：内置/主题声明组件的默认值来自后端注册表与 theme.toml [[widgets]]，
-- 未保存过的组件不占行。切换主题不丢；删除主题时由后端连带删除其行。
-- 对已有安装幂等：纯新增表。config 为 JSON 对象 TEXT（参数值一律规范化字符串存储，
-- 与 theme_settings 同款类型转换）；时间戳沿用全库 RFC3339 UTC 文本惯例。

CREATE TABLE IF NOT EXISTS theme_widgets (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    theme_slug TEXT NOT NULL,
    widget_key TEXT NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 0,
    position   TEXT NOT NULL DEFAULT 'sidebar',
    sort_order INTEGER NOT NULL DEFAULT 0,
    config     TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (theme_slug, widget_key)
);

CREATE INDEX IF NOT EXISTS idx_theme_widgets_slug ON theme_widgets (theme_slug, sort_order);
