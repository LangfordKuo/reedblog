-- 页面功能（契约「页面」条款）：pages 表（站点级单页：关于/留言板/友情链接/自定义）
-- + page_links 表（友情链接数据）+ comments.target_type 扩展（留言=挂在页面上的评论）。
-- 对已有安装幂等：纯新增表；comments 加列带 NOT NULL DEFAULT，旧行自动视为 'post'（文章评论）。
-- 时间戳沿用全库 RFC3339 UTC 文本惯例，与 SQLite 版共用查询逻辑。

CREATE TABLE IF NOT EXISTS pages (
    id           BIGINT AUTO_INCREMENT PRIMARY KEY,
    title        VARCHAR(255) NOT NULL,
    slug         VARCHAR(255) NOT NULL UNIQUE,
    kind         VARCHAR(20) NOT NULL DEFAULT 'custom',
    content_md   MEDIUMTEXT NOT NULL,
    content_html MEDIUMTEXT NOT NULL,
    enabled      TINYINT(1) NOT NULL DEFAULT 1,
    sort_order   BIGINT NOT NULL DEFAULT 0,
    built_in     TINYINT(1) NOT NULL DEFAULT 0,
    created_at   VARCHAR(40) NOT NULL,
    updated_at   VARCHAR(40) NOT NULL,
    INDEX idx_pages_enabled_sort (enabled, sort_order)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS page_links (
    id          BIGINT AUTO_INCREMENT PRIMARY KEY,
    page_id     BIGINT NOT NULL,
    name        VARCHAR(255) NOT NULL,
    url         VARCHAR(500) NOT NULL,
    description TEXT NOT NULL,
    sort_order  BIGINT NOT NULL DEFAULT 0,
    INDEX idx_page_links_page (page_id, sort_order)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- 评论目标扩展：'post'（文章，默认/旧数据）| 'page'（页面留言）；
-- post_id 列复用为通用目标 id（target_type='page' 时存页面 id），对现有契约破坏最小
ALTER TABLE comments ADD COLUMN target_type VARCHAR(20) NOT NULL DEFAULT 'post';

ALTER TABLE comments ADD INDEX idx_comments_target (target_type, post_id, status);
