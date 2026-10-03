-- 文章回收站（软删除，契约「文章回收站」条款，2026-10-04 新增）：
-- posts 加 deleted_at（RFC3339 UTC 字符串；NULL=正常，非 NULL=在回收站，值为移入时刻）。
-- 可见性谓词扩展为「原条件 AND deleted_at IS NULL」（helpers::VISIBLE_POST_SQL 唯一实现，
-- 内含 NOT_DELETED_SQL）；回收站文章对前台完全不可见、不计入任何计数，且仍占用 slug
-- （软删期间同 slug 新建仍 409），purge 彻底删除后才释放。
-- 对已有安装幂等：纯新增可空列，旧行自动为 NULL（即未删除），与 SQLite 迁移语义一致。

ALTER TABLE posts ADD COLUMN deleted_at VARCHAR(40) NULL;

ALTER TABLE posts ADD INDEX idx_posts_deleted (deleted_at);
