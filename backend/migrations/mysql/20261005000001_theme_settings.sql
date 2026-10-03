-- 主题设置（扩展契约「主题设置项」条款）：按主题 slug 隔离的 key-value 存储。
-- 切换激活主题不清除任何主题的设置；删除主题时由后端连带删除其行。
-- 对已有安装幂等：纯新增表。value 为规范化 TEXT（switch → "true"/"false"，
-- number → 规范化数字串，其余原样）；时间戳沿用全库 RFC3339 UTC 文本惯例，
-- 与 SQLite 版共用查询逻辑。`key` 为 SQL 关键字，列名用反引号引用。

CREATE TABLE IF NOT EXISTS theme_settings (
    theme_slug VARCHAR(128) NOT NULL,
    `key`      VARCHAR(64) NOT NULL,
    value      TEXT NOT NULL,
    updated_at VARCHAR(40) NOT NULL,
    PRIMARY KEY (theme_slug, `key`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
