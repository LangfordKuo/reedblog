-- 页面图标（契约「页面-图标」条款，2026-10-04 新增）：pages 表加 icon 列。
-- 对已有安装平滑：纯加列 + NOT NULL DEFAULT ''，旧行自动取空串（前端按 kind/slug 兜底默认图标）。
-- 只存图标名（小写 slug 形式，后端只校验格式不枚举；见契约校验规则），空串=未设置。

ALTER TABLE pages ADD COLUMN icon TEXT NOT NULL DEFAULT '';
