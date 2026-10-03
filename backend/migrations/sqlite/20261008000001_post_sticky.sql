-- 文章置顶（契约「文章置顶与定时发布」条款，2026-10-03 新增）：
-- posts 加 is_sticky 整数列（0/1，默认 0）。公开列表 recent 序改为
-- is_sticky DESC, published_at DESC（置顶在前）；RSS/sitemap/搜索/hot 序不受影响。
-- 对已有安装幂等：纯新增列带 NOT NULL DEFAULT，旧行自动为 0（不置顶）。

ALTER TABLE posts ADD COLUMN is_sticky INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_posts_sticky ON posts (is_sticky, published_at);
