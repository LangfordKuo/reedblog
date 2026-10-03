-- 文章点赞（契约「浏览量与点赞」条款）：post_likes 表。
-- liker_key = 前端首次生成并存 localStorage（reedblog_like_id）的匿名 id（UUID）；
-- (post_id, liker_key) UNIQUE KEY 保证同访客对同文章至多一条点赞行（重复点赞幂等）。
-- UNIQUE KEY 自动创建 (post_id, liker_key) 索引，按 post_id 计数走其最左前缀。
-- liker_key 上限 64 字符（应用层校验同宽），与全库 VARCHAR 惯例一致。

CREATE TABLE IF NOT EXISTS post_likes (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    post_id    BIGINT NOT NULL,
    liker_key  VARCHAR(64) NOT NULL,
    created_at VARCHAR(40) NOT NULL,
    UNIQUE KEY uq_post_likes_post_liker (post_id, liker_key)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
