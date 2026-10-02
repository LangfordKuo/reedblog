# demo-suite —— reedblog 官方示例插件

一个覆盖插件系统全部能力的示例：**评论违禁词过滤**（`comment.before_create` 钩子，
含 `block` 拦截与放行）、**文章尾部版权声明**（`post.after_render` 钩子改写
`content_html`）、**前端轻注入**（`head` 统计脚本 + `body_end` 悬浮徽标）。

## 目录结构

```
demo-suite/
  manifest.toml      # 元数据 + hooks/inject 声明（必需）
  main.rhai          # 钩子脚本入口（必需）
  inject/
    head.html        # 注入 <head>（统计脚本示例）
    body_end.html    # 注入 </body> 前（悬浮徽标示例）
  README.md          # 本文件（可选，不参与运行）
```

## 打包

zip 根层必须是**单个插件目录**（目录名 = manifest 里的 `slug`）：

```bash
# Linux / macOS（在 examples/plugins 目录下执行）
zip -r demo-suite.zip demo-suite

# Windows PowerShell（在 examples\plugins 目录下执行）
Compress-Archive -Path demo-suite -DestinationPath demo-suite.zip
```

## 安装与启用

1. 登录后台，进入「插件管理」（`/admin/plugins`）；
2. 选择 `demo-suite.zip`，点击「上传安装」→ 安装成功，**默认停用**；
3. 打开该插件行的「启用」开关（此时后端会对 `main.rhai` 做沙箱语法编译，
   语法错误会返回 422 `script_error` 并保持停用）；
4. 验证：
   - 打开任意已发布文章 → 正文尾部出现版权声明，右下角出现徽标，
     浏览器控制台输出 `[demo-suite] 页面访问 #1`；
   - 提交一条包含 `spam` 的评论 → 被拦截，提示「评论包含违禁内容，已拒绝发布」。

修改 `inject/*.html` 后刷新页面即生效；修改 `main.rhai` 后需要**停用再启用**
（或重启后端）才会重新加载。

完整开发文档见 [docs/plugin-development.md](../../../docs/plugin-development.md)。
