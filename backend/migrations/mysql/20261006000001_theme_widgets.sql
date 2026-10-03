-- 主题组件（契约「主题组件」条款）：按主题 slug 隔离的 widget 配置行。
-- 只存覆盖行：内置/主题声明组件的默认值来自后端注册表与 theme.toml [[widgets]]，
-- 未保存过的组件不占行。切换主题不丢；删除主题时由后端连带删除其行。
-- 对已有安装幂等：纯新增表。config 为 JSON 对象 TEXT（参数值一律规范化字符串存储），
-- 时间戳沿用全库 RFC3339 UTC 文本惯例，与 SQLite 版共用查询逻辑。

CREATE TABLE IF NOT EXISTS theme_widgets (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    theme_slug VARCHAR(128) NOT NULL,
    widget_key VARCHAR(64) NOT NULL,
    enabled    TINYINT(1) NOT NULL DEFAULT 0,
    position   VARCHAR(20) NOT NULL DEFAULT 'sidebar',
    sort_order BIGINT NOT NULL DEFAULT 0,
    config     MEDIUMTEXT NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    UNIQUE KEY uq_theme_widgets (theme_slug, widget_key),
    INDEX idx_theme_widgets_slug (theme_slug, sort_order)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
