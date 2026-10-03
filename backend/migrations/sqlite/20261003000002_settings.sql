-- 站点设置：key-value 存储（name 唯一键）。
-- 对已有安装幂等：纯新增表；旧库缺行时由后端按键回退默认值（config.toml [site] / [server] base_url）。
-- 时间戳沿用全库 RFC3339 UTC 文本惯例，SQLite/MySQL 共用查询逻辑。

CREATE TABLE IF NOT EXISTS settings (
    name       TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
