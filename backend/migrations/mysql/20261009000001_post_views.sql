-- 文章浏览量（契约「浏览量与点赞」条款）：posts 加 view_count 整数列（默认 0）。
-- 计数点：公开 GET /api/posts/:slug（进程内 (ip, post_id) 60 分钟去重，尽力而为）；
-- order=hot 排序改为 view_count DESC, comment_count DESC, published_at DESC。
-- 对已有安装幂等：纯新增列带 NOT NULL DEFAULT，旧行自动为 0，与 SQLite 迁移语义一致。

ALTER TABLE posts ADD COLUMN view_count BIGINT NOT NULL DEFAULT 0;

ALTER TABLE posts ADD INDEX idx_posts_view_count (view_count);
