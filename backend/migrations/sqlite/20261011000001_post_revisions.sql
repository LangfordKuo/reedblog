-- 文章修订历史（契约「文章修订历史」条款，2026-10-04 新增）：
-- 创建文章时插入一条初始修订；之后每次「title/content_md/excerpt 任一变化」的保存
-- 追加一条保存后内容的快照。每篇文章最多保留 20 条（应用层常量，插入后在同一事务内
-- 按 id 从新到旧裁剪）。修订只记内容，不受定时发布/置顶等状态影响。
-- content_md 存完整历史正文（单条 GET 返回，供前端差异对比；列表摘要不返回）。
-- 时间戳沿用全库 RFC3339 UTC 文本约定（字典序即时间序；同秒以 id DESC 兜底）。

CREATE TABLE IF NOT EXISTS post_revisions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id    INTEGER NOT NULL,
    title      TEXT NOT NULL,
    content_md TEXT NOT NULL,
    excerpt    TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_post_revisions_post ON post_revisions (post_id, created_at, id);
