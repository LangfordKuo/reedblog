# midnight —— reedblog 官方示例暗色主题

一套完整的深蓝色暗色主题：`theme.toml` 覆盖前端认识的**全部 32 个设计令牌**
（含 `[tokens]` 与 `[tokens_dark]` 两份完整色板），`theme.css` 演示如何覆盖
内置写死的代码块浅色背景、自定义字体（注释示例）与滚动条样式，`assets/`
演示静态资源的引用路径。

## 目录结构

```
midnight/
  theme.toml      # 元数据 + 设计令牌（必需）
  theme.css       # 自定义 CSS（可选）
  preview.png     # 后台预览图（可选，本示例未附带；建议 480×280 的 PNG）
  assets/         # 字体/图片等静态资源（可选），见 assets/README.md
```

## 打包

zip 根层必须是**单个主题目录**（目录名 = theme.toml 里的 `slug`）：

```bash
# Linux / macOS（在 examples/themes 目录下执行）
zip -r midnight.zip midnight

# Windows PowerShell（在 examples\themes 目录下执行）
Compress-Archive -Path midnight -DestinationPath midnight.zip
```

## 上传与激活

1. 登录后台，进入「主题管理」（`/admin/themes`）；
2. 选择 `midnight.zip`，点击「上传主题」→ 201 安装成功（未激活）；
3. 点击该主题行的「激活」→ 写入 config.toml `[themes] active`，
   后台页面会立即热切换样式，前台下次加载生效；
4. 想删除时：先激活其他主题（激活中的主题不可删，409 `theme_active`），
   再点「删除」。内置 `default` 主题永远不可删除、不可被上传覆盖。

完整开发文档见 [docs/theme-development.md](../../../docs/theme-development.md)。
