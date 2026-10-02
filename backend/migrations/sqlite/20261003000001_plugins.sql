-- 插件系统：启用状态持久化（manifest 内容以磁盘文件为准，DB 只存开关与时间戳）
-- 对已有安装幂等：纯新增表，不触碰旧数据。

CREATE TABLE IF NOT EXISTS plugins (
    slug         TEXT PRIMARY KEY,
    enabled      INTEGER NOT NULL DEFAULT 0,
    installed_at TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
