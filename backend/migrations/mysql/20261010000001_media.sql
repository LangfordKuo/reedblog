-- 媒体库（契约「媒体库」条款，2026-10-04 新增）：一条记录对应 uploads/ 下一个已上传图片文件。
-- url（存储路径 /api/uploads/<yyyy>/<mm>/<file>）= 唯一键：URL 由内容 sha256 决定，
-- 同图重复上传命中同一路径，不新增行（上传链路先查后插，并发撞唯一约束时兜底重查）。
-- width/height 按图片头尽力解析，失败存 NULL。历史文件由列表接口惰性扫描补建记录。
-- 时间戳沿用全库 RFC3339 UTC 文本约定（字典序即时间序）；BIGINT 与 Any 驱动的 i64 解码对齐。

CREATE TABLE IF NOT EXISTS media (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    url        VARCHAR(255) NOT NULL,
    filename   VARCHAR(255) NOT NULL,
    size       BIGINT NOT NULL,
    mime       VARCHAR(64) NOT NULL,
    width      BIGINT NULL,
    height     BIGINT NULL,
    created_at VARCHAR(40) NOT NULL,
    UNIQUE KEY uq_media_url (url)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
