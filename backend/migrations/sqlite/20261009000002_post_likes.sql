-- 文章点赞（契约「浏览量与点赞」条款）：post_likes 表。
-- liker_key = 前端首次生成并存 localStorage（reedblog_like_id）的匿名 id（UUID）；
-- (post_id, liker_key) UNIQUE 保证同访客对同文章至多一条点赞行（重复点赞幂等）。
-- UNIQUE 约束自动创建 (post_id, liker_key) 索引，按 post_id 计数走其最左前缀。

CREATE TABLE IF NOT EXISTS post_likes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id    INTEGER NOT NULL,
    liker_key  TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (post_id, liker_key)
);
