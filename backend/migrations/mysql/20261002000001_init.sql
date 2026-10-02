-- reedblog 初始 schema（MySQL 版）
-- 约定：时间戳一律存 RFC3339 UTC 字符串（YYYY-MM-DDTHH:MM:SSZ），字典序即时间序，与 SQLite 版共用查询逻辑。

CREATE TABLE IF NOT EXISTS users (
    id            BIGINT AUTO_INCREMENT PRIMARY KEY,
    username      VARCHAR(255) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    created_at    VARCHAR(40) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS categories (
    id   BIGINT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(255) NOT NULL UNIQUE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS tags (
    id   BIGINT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(255) NOT NULL UNIQUE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS posts (
    id           BIGINT AUTO_INCREMENT PRIMARY KEY,
    title        VARCHAR(255) NOT NULL,
    slug         VARCHAR(255) NOT NULL UNIQUE,
    excerpt      TEXT NOT NULL,
    content_md   MEDIUMTEXT NOT NULL,
    status       VARCHAR(20) NOT NULL DEFAULT 'draft',
    category_id  BIGINT NULL,
    published_at VARCHAR(40) NULL,
    created_at   VARCHAR(40) NOT NULL,
    updated_at   VARCHAR(40) NOT NULL,
    INDEX idx_posts_status_published (status, published_at),
    INDEX idx_posts_category (category_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS post_tags (
    post_id BIGINT NOT NULL,
    tag_id  BIGINT NOT NULL,
    PRIMARY KEY (post_id, tag_id),
    INDEX idx_post_tags_tag (tag_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS comments (
    id          BIGINT AUTO_INCREMENT PRIMARY KEY,
    post_id     BIGINT NOT NULL,
    author_name VARCHAR(255) NOT NULL,
    email       VARCHAR(255) NULL,
    content     TEXT NOT NULL,
    status      VARCHAR(20) NOT NULL DEFAULT 'approved',
    created_at  VARCHAR(40) NOT NULL,
    INDEX idx_comments_post (post_id, status)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
