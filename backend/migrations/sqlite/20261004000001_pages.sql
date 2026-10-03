-- 页面功能（契约「页面」条款）：pages 表（站点级单页：关于/留言板/友情链接/自定义）
-- + page_links 表（友情链接数据）+ comments.target_type 扩展（留言=挂在页面上的评论）。
-- 对已有安装幂等：纯新增表；comments 加列带 NOT NULL DEFAULT，旧行自动视为 'post'（文章评论）。
-- 时间戳沿用全库 RFC3339 UTC 文本惯例，SQLite/MySQL 共用查询逻辑。

CREATE TABLE IF NOT EXISTS pages (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    title        TEXT NOT NULL,
    slug         TEXT NOT NULL UNIQUE,
    kind         TEXT NOT NULL DEFAULT 'custom',
    content_md   TEXT NOT NULL DEFAULT '',
    content_html TEXT NOT NULL DEFAULT '',
    enabled      INTEGER NOT NULL DEFAULT 1,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    built_in     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pages_enabled_sort ON pages (enabled, sort_order);

CREATE TABLE IF NOT EXISTS page_links (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id     INTEGER NOT NULL,
    name        TEXT NOT NULL,
    url         TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_page_links_page ON page_links (page_id, sort_order);

-- 评论目标扩展：'post'（文章，默认/旧数据）| 'page'（页面留言）；
-- post_id 列复用为通用目标 id（target_type='page' 时存页面 id），对现有契约破坏最小
ALTER TABLE comments ADD COLUMN target_type TEXT NOT NULL DEFAULT 'post';

CREATE INDEX IF NOT EXISTS idx_comments_target ON comments (target_type, post_id, status);
