-- 嵌套评论（楼中楼回复，契约「评论回复」条款，2026-10-03 新增）：
-- comments 加 parent_id（所属顶级楼层 id，可空自引用）+ reply_to_id（被回复的中间楼层 id，可空）。
-- 两级归一化（Typecho/WP 风格）：存储上永远两级——parent_id 恒指顶级祖先，
-- 回复中间楼层时被回复人记在 reply_to_id。SQLite 的 ADD COLUMN 不支持内联
-- REFERENCES 子句，自引用一致性由应用层校验保证（422 拒绝跨目标/不存在/未过审的父评论）。
-- 对已有安装幂等：纯新增可空列，旧行 parent_id/reply_to_id 均为 NULL（即顶级评论）。

ALTER TABLE comments ADD COLUMN parent_id INTEGER;

ALTER TABLE comments ADD COLUMN reply_to_id INTEGER;

CREATE INDEX IF NOT EXISTS idx_comments_parent ON comments (parent_id);
