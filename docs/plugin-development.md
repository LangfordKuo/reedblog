# reedblog 插件开发指南

本文面向插件作者，内容以 reedblog 0.1.0 的实际实现为准（后端脚本引擎为 Rhai 1.x，
`backend/Cargo.lock` 锁定 1.26.1）。系统级规格见 [extensibility-contract.md](./extensibility-contract.md) 第一部分，
核心 API 见 [api-contract.md](./api-contract.md)。

---

## 1. 插件是什么

一个 reedblog 插件就是**一个目录**：元数据（`manifest.toml`）+ 一段 Rhai 脚本
（`main.rhai`）+ 可选的前端注入片段（`inject/*.html`）。不需要编译、不需要 Rust
知识，上传 zip 即可安装。

插件有两类能力：

| 能力 | 作用位置 | 说明 |
|---|---|---|
| 后端钩子（4 个） | 文章渲染 / 评论创建 / 文章发布 | 可读写传入数据、可拦截评论，通过返回值影响流程 |
| 前端轻注入（2 个位置） | 公开页面 `<head>` / `<body>` 末尾 | 原始 HTML 片段，可含 `<script>`、`<style>`、`<meta>` 等 |

其他关键特性：

- **热切换**：启用 / 停用 / 删除立即生效，无需重启后端；注入片段每次请求都从磁盘
  读取，改动后刷新页面即生效。
- **安装默认停用**：上传安装后 `enabled:false`，必须显式启用，防止误触发。
- **持久化**：启用状态存数据库 `plugins` 表（`slug, enabled, installed_at, updated_at`），
  重启后自动恢复；manifest 内容以磁盘文件为准，数据库只存开关与时间戳。
- **信任模型**：注入的 HTML **不做任何转义与过滤**，`<script>` 会真实执行——与
  WordPress 插件相同的信任模型。**只安装你信任的插件。**

## 2. 目录结构与存储位置

插件存储根目录为后端运行目录下的 `plugins/`，可在 `config.toml` 配置：

```toml
[plugins]
dir = "plugins"     # 默认值；相对后端运行目录，也可写绝对路径
```

单个插件的目录结构（目录名必须等于 manifest 中的 `slug`）：

```
plugins/
  <plugin-slug>/
    manifest.toml        # 必需：元数据 + hooks/inject 声明
    main.rhai            # 必需：钩子脚本入口
    inject/              # 可选：前端注入片段
      head.html          #   注入 <head>（统计脚本、meta、CSS link 等）
      body_end.html      #   注入 </body> 前（widget、悬浮组件等）
    README.md            # 可选：说明文件，不参与运行
```

扫描规则：

- 后端**启动时**扫描 `plugins/` 下的子目录；以 `.` 开头的目录（如上传解压用的
  `.tmp-*` staging 目录）会被跳过。
- `manifest.toml` 缺失或非法的目录会被跳过，并在后端 stderr 打一条告警
  （`插件 <dir> manifest 无效，已跳过: ...`），不会导致启动失败。
- 通过后台上传 zip 安装的插件立即进入内存注册表，**无需重启**；手工把目录拷进
  `plugins/` 则需要重启后端才能被扫描到（且因数据库无记录，视为未启用）。

## 3. manifest.toml 全字段

```toml
name = "示例插件"            # 必需
slug = "demo-plugin"         # 必需
version = "1.0.0"            # 必需
description = "一句话说明"    # 可选，默认空字符串
author = "作者名"            # 可选，默认空字符串
min_app_version = "0.1.0"    # 可选
hooks = ["post.before_render", "comment.before_create"]   # 可选，默认 []
inject = ["head", "body_end"]                             # 可选，默认 []
```

| 字段 | 必填 | 校验规则 | 不满足时 |
|---|---|---|---|
| `name` | 是 | trim 后非空 | 422 `invalid_manifest` |
| `slug` | 是 | 匹配 `^[a-z0-9-]+$` 且 ≤128 字符；必须与插件目录名（zip 根层目录名）一致；不得与已安装插件冲突 | 422 `invalid_manifest`；冲突时 409 `plugin_exists` |
| `version` | 是 | semver：核心必须是 `X.Y.Z` 三段数字；`-prerelease` / `+build` 后缀允许但不参与校验与比较 | 422 `invalid_manifest` |
| `description` | 否 | 无 | — |
| `author` | 否 | 无 | — |
| `min_app_version` | 否 | semver（同上）；**启用时**若大于当前 reedblog 版本（`CARGO_PKG_VERSION`，当前 0.1.0）则拒绝启用 | 解析失败 422 `invalid_manifest`；版本过高时启用返回 422 `invalid_manifest`（message 形如「插件要求 reedblog >= 99.0.0，当前版本 0.1.0」） |
| `hooks` | 否 | 只能取枚举值：`post.before_render`、`post.after_render`、`comment.before_create`、`post.after_publish` | 出现未知值 → 422 `invalid_manifest` |
| `inject` | 否 | 只能取枚举值：`head`、`body_end` | 出现未知值 → 422 `invalid_manifest` |

说明：

- **未声明的钩子即使脚本里定义了也不会被调用**（后端按 manifest 的 `hooks` 路由）；
  同理，未声明的注入位置即使 `inject/` 下有对应文件也不会下发。
- manifest 中出现上述之外的未知 TOML 字段会被忽略（反序列化不拒绝未知字段）。

## 4. Rhai 钩子完整参考

### 4.1 命名与调用机制

每个钩子对应 `main.rhai` 里的一个函数，**函数名 = 钩子 id 把 `.` 换成 `_`**：

| 钩子 id | Rhai 函数 |
|---|---|
| `post.before_render` | `post_before_render(ctx)` |
| `post.after_render` | `post_after_render(ctx)` |
| `comment.before_create` | `comment_before_create(ctx)` |
| `post.after_publish` | `post_after_publish(ctx)` |

调用机制（与 `backend/src/plugins.rs` 实现一致）：

- 入参 `ctx` 是一个 Rhai object map（字段见下文各钩子）。
- 每次钩子触发，后端把**整个 `main.rhai` 源码 + 一行 `函数名(ctx)` 调用**作为一个
  脚本在沙箱里 eval 一次。这意味着：
  - 脚本顶层语句在**每次**钩子调用时都会执行——不要把昂贵的初始化写在顶层；
  - 两次调用之间**没有持久状态**（全局变量不会保留）。
- `main.rhai` 在**启用时**读入内存并做一次沙箱语法编译（失败 → 422 `script_error`，
  保持停用）；之后磁盘上的脚本改动不会自动生效，需要**停用再启用**（或重启后端）。

### 4.2 各钩子明细

#### `post.before_render` —— 文章渲染前

- **触发时机**：公开端点 `GET /api/posts/:slug`（文章详情，仅已发布文章）每次被请求时。
- **入参**：

  | 字段 | 类型 | 说明 |
  |---|---|---|
  | `ctx.title` | string | 文章标题（可能已被链上更靠前的插件改写过） |
  | `ctx.content_md` | string | Markdown 原文 |
  | `ctx.slug` | string | 文章 slug |

- **返回值**：必须返回 map（通常就是改写后的 `ctx`）。后端只读取其中的 `title` 与
  `content_md` 两个 key，其余 key 忽略；返回原样即不改。
- **改写效果**：
  - `content_md` 的改写会进入后续 Markdown 渲染，并且**响应里的 `content_md` 字段
    返回的就是改写后的值**；
  - `title` 的改写只会作为 `ctx.title` 传给后续 `post.after_render` 链——API 响应中
    的 `title` 字段仍是数据库原值；
  - 改写**不落库**，只影响本次响应（每次请求都重新执行钩子链）。
- 示例：

  ```rhai
  fn post_before_render(ctx) {
      ctx.content_md = ctx.content_md + "\n\n> 本文最后校对于自动化流水线。";
      ctx
  }
  ```

#### `post.after_render` —— 文章渲染后

- **触发时机**：同上端点，在 `before_render` 链改写 `content_md` → Markdown 渲染成
  HTML 之后执行。渲染器为 pulldown-cmark，启用 GFM 表格、删除线、任务列表；渲染
  结果**不做 HTML 消毒**（正文里原有的内联 HTML 会保留）。
- **入参**：

  | 字段 | 类型 | 说明 |
  |---|---|---|
  | `ctx.title` | string | 文章标题（`before_render` 链改写后的值） |
  | `ctx.content_html` | string | 渲染后的 HTML |
  | `ctx.slug` | string | 文章 slug |

- **返回值**：必须返回 map；后端只读取 `content_html`，作为响应的最终
  `content_html`。典型用法是追加版权声明、给外链加 `rel="nofollow"` 等。
- 示例见 §8 完整示例（版权页脚）。

#### `comment.before_create` —— 评论创建前

- **触发时机**：公开端点 `POST /api/posts/:slug/comments` 与留言板
  `POST /api/pages/:slug/comments`（含**回复**，2026-10-03 嵌套评论新增），
  在必填校验与父评论校验/两级归一化之后、写库之前。
- **入参**：

  | 字段 | 类型 | 说明 |
  |---|---|---|
  | `ctx.post_slug` | string | 目标文章 slug（留言板留言时为页面 slug） |
  | `ctx.author_name` | string | 昵称（已 trim，非空） |
  | `ctx.email` | string | 邮箱；**未填时为空字符串**（不会是 null） |
  | `ctx.content` | string | 评论内容（已 trim，非空） |
  | `ctx.parent_id` | int | 所属顶级楼层 id（归一化后的最终存储值）；**顶级评论为 0**。只读 |
  | `ctx.reply_to_id` | int | 被回复的中间楼层 id；**无（顶级评论/直接回复顶级）为 0**。只读 |

- **返回值**：必须返回带 `action` 的 map：
  - `#{ action: "block", reason: "..." }` —— **拦截**。评论创建失败，客户端收到
    `403`，错误形状 `{"error":{"code":"comment_blocked","message":"<reason>"}}`；
    `reason` 为空/缺失时 message 用「评论被插件拦截」。
  - `#{ action: "allow" }` —— **放行**，字段不改。
  - `#{ action: "allow", author_name: "...", email: "...", content: "..." }` ——
    放行并**改写字段**（三个改写 key 都可选，只写要改的）。改写后的
    `author_name` / `content` 仍须非空，否则返回 422 `validation_error`；`email`
    改写为空字符串等价于「无邮箱」。
  - 返回缺少 `action`（或不是 map）——按**运行时错误**处理：写入该插件的
    `last_error`（「返回值缺少 action（须为 "allow" 或 "block"）」），视为本插件
    弃权，链继续、评论照常放行。
- 示例（敏感词 block）见 §8。

#### `post.after_publish` —— 文章发布后（通知类）

- **触发时机**（两种）：
  1. `POST /api/admin/posts` 创建文章且 `status = "published"`；
  2. `PUT /api/admin/posts/:id` 使文章从 draft 转为 published（首次写入
     `published_at`）。
- **入参**：

  | 字段 | 类型 | 说明 |
  |---|---|---|
  | `ctx.title` | string | 文章标题 |
  | `ctx.slug` | string | 文章 slug |
  | `ctx.published_at` | string | 发布时间，RFC3339 UTC（如 `2026-10-03T12:00:00Z`） |

- **返回值**：**忽略**（返回什么都不影响流程；抛错会记 `last_error`）。这是通知类
  钩子；第一版沙箱没有网络能力，无法直接调外部 webhook，适合做 `print()` 日志或
  纯计算。

### 4.3 链式调用规则

多个**启用中且声明了同一钩子**的插件构成一条链：

- **顺序**：按插件 slug **字典序**（内存注册表是 BTreeMap），依次串行调用；
- **数据流**：前一个插件的输出（改写后的字段）作为下一个插件的输入；
- **短路**：`comment.before_create` 链中任一插件返回 `block`，**立即**返回拦截结果，
  字典序在它之后的插件不再执行；
- **容错**：链中某插件运行时出错只跳过它自己（其余插件与主流程不受影响）。

### 4.4 运行时错误与 last_error

- 脚本运行时错误（`throw`、除以零、超沙箱限制、返回类型不对等）**不会阻断主流程**：
  该插件该钩子被跳过，文章照常渲染 / 评论照常放行。
- 错误会：
  1. 打印到后端 stderr：`[reedblog] 插件 <slug> 钩子 <hook_id> 运行时错误（已跳过）: <err>`；
  2. 写入插件详情的 `last_error` 字段，格式 `[<hook_id>] <错误消息>`，可在
     `GET /api/admin/plugins/:slug` 或后台「插件管理」的「最近错误」列看到。
- 该插件任意钩子下一次**成功**执行后，`last_error` 会被自动清空。
- 启用时脚本已加载、但启动恢复时语法校验失败的插件：脚本不加载（钩子不参与链），
  `last_error` 记录原因。

## 5. Rhai 沙箱限制

后端强制的限制（`backend/src/plugins.rs` `sandbox_engine()`，与契约一致）：

| 限制项 | 数值 | 含义 |
|---|---|---|
| `set_max_call_levels` | **64** | 函数调用栈最大深度（含递归），超出即运行时错误 |
| `set_max_operations` | **50 000** | 单次钩子调用可执行的操作总数，防死循环/超大计算 |
| `set_max_string_size` | **1 MB**（1024×1024 字节） | 单个字符串最大长度 |
| `set_max_array_size` | **10 000** | 单个数组最大元素数 |
| `set_max_map_size` | **10 000** | 单个 map 最大键数 |

**没有**的能力（引擎只含 Rhai 标准库，宿主未注册任何外部函数/模块）：

- 文件读写、目录遍历；
- 网络请求（无 HTTP/TCP/socket）；
- 进程/命令执行、环境变量；
- `JSON` 模块（rhai 的 serde 特性未启用，`JSON::deserialize` 等不可用）；
- `Instant` 时间模块（`Instant::now()` 不可用；需要时间的场景只有钩子入参里
  自带的 `published_at`）。

**有**的能力：Rhai 标准库的纯计算部分——字符串方法（`to_lower`、`to_upper`、
`contains`、`split`、`replace`、`trim` 等）、数组/映射操作与迭代器、数学函数
（`max`、`min`、`abs` 等）、自定义函数、`print()` / `debug()`（输出到**后端进程的
stdout**，可用于调试）。

## 6. 前端轻注入

### 6.1 声明与片段文件

manifest 中声明 `inject = ["head", "body_end"]`，对应文件：

```
inject/head.html        → 注入公开页面 <head>
inject/body_end.html    → 注入公开页面 <body> 末尾
```

片段缺失或内容为空白时不下发（不报错）。

### 6.2 下发 API

公开端点（无需鉴权，且在「未安装」门禁白名单内）：

```
GET /api/frontend/injections
→ {"head": [{"plugin": "<slug>", "html": "..."}], "body_end": [...]}
```

- 仅返回 `enabled` 且声明了对应位置的插件；按插件 slug 字典序排列。
- 片段内容**每次请求都从磁盘读取**：直接在服务器改 `inject/*.html`，刷新页面即
  生效，无需停用/启用或重启。
- 未安装任何插件（或站点未安装）时返回空数组。

### 6.3 前端注入行为

前端 `PluginInjections` 组件（`frontend/src/components/plugin-injections.tsx`）：

- **注入哪些页面**：只挂在公开博客布局（`SiteLayout`）上；**`/admin` 与 `/install`
  绝不注入**。
- **时机**：页面挂载（hydration 完成）后请求一次上述 API 并注入；片段变更需刷新
  页面才生效（不轮询）。SPA 跳转离开公开布局时（如进入后台），已注入节点会被移除。
- **位置**：`head` 片段追加到 `document.head`；`body_end` 片段追加到
  `document.body` 末尾。
- **script 可执行**：`innerHTML` 方式插入的 `<script>` 本身不会执行，前端会把片段中
  的每个 `<script>` 重建为真实 script 元素（保留全部属性，内联代码与外链 `src` 都
  会执行）。因此统计脚本（Plausible/Umami/百度统计等）可以直接放 `head.html`。
- **不转义、不过滤**：片段是**原始 HTML**，原样进入 DOM；每个注入节点带
  `data-reedblog-plugin="<slug>"` 属性标记，便于定位与清理。
- **信任模型**：注入内容拥有与站点同源的完整能力（读 cookie/localStorage、发任意
  请求、改 DOM）。只安装可信插件。

## 7. 打包与安装

### 7.1 zip 结构要求

- zip **根层必须是单个目录**，且目录名等于 `slug`；根层出现多个目录或散落文件 →
  422 `invalid_package`。
- 目录内必须含 `manifest.toml` 与 `main.rhai`，缺任一 → 422 `invalid_package`。
- 安全与体积限制（`backend/src/packages.rs`）：
  - 上传请求体 ≤ **32 MB**（`/api/admin/plugins` 的 DefaultBodyLimit）；
  - 解压后总体积 ≤ **64 MB**，条目数 ≤ **4096**（防 zip 炸弹）；
  - 条目路径含 `..`、盘符（`:`）或以绝对路径展开 → 拒绝；反斜杠路径会被归一化；
  - 符号链接条目直接跳过不解压。

打包命令：

```bash
# Linux / macOS（在插件目录的父目录执行）
zip -r demo-suite.zip demo-suite

# Windows PowerShell
Compress-Archive -Path demo-suite -DestinationPath demo-suite.zip
```

### 7.2 后台上传与启停

1. 登录后台 → 「插件管理」（`/admin/plugins`）；
2. 选择 zip 文件 → 点击「上传安装」→ 成功后出现在列表中，**默认停用**；
3. 打开该插件行的「启用」开关：
   - 后端校验 `min_app_version`（过高 → 422 `invalid_manifest`）；
   - 读取 `main.rhai` 做沙箱语法编译（失败 → 422 `script_error`，保持停用）；
   - 成功后 `enabled:true`，钩子与注入**立即生效**（写数据库持久化）。
4. 「停用」：立即卸载内存脚本，钩子与注入立即失效（幂等，可重复调用）。
5. 「删除」：删除插件目录、内存注册表项与数据库行 → 204。**启用中的插件也可以
   直接删除**，删除后钩子立即失效；slug 随即可复用（重新安装）。

### 7.3 管理 API 一览（均需 Bearer）

| 端点 | 成功响应 |
|---|---|
| `GET /api/admin/plugins` | `{"items":[PluginInfo], "total":int}`（按 slug 字典序，不分页） |
| `POST /api/admin/plugins`（multipart，字段 `file` = zip） | 201 `PluginInfo`（`enabled:false`） |
| `GET /api/admin/plugins/:slug` | `PluginInfo`（含 `last_error`） |
| `POST /api/admin/plugins/:slug/enable` | 200 `PluginInfo`（`enabled:true`） |
| `POST /api/admin/plugins/:slug/disable` | 200 `PluginInfo`（`enabled:false`） |
| `DELETE /api/admin/plugins/:slug` | 204 |

`PluginInfo` 形状：

```json
{
  "slug": "demo-suite",
  "name": "博客小套件",
  "version": "1.0.0",
  "description": "...",
  "author": "...",
  "enabled": true,
  "hooks": ["comment.before_create", "post.after_render"],
  "inject": ["head", "body_end"],
  "min_app_version": "0.1.0",
  "last_error": "[post.after_render] ...",
  "installed_at": "2026-10-03T12:00:00Z",
  "updated_at": "2026-10-03T12:00:00Z"
}
```

（`min_app_version` 与 `last_error` 为空时**不出现在 JSON 中**。）

## 8. 完整示例讲解

仓库 [`examples/plugins/demo-suite/`](../examples/plugins/demo-suite/) 是一个可直接打包
安装的示例插件，覆盖三种典型能力：评论敏感词过滤、文章尾部版权声明、head 统计
脚本注入。

### manifest.toml

```toml
name = "博客小套件"
slug = "demo-suite"
version = "1.0.0"
description = "reedblog 官方示例插件：评论违禁词过滤 + 文章版权页脚 + 前端注入演示"
author = "reedblog"
min_app_version = "0.1.0"
hooks = ["comment.before_create", "post.after_render"]
inject = ["head", "body_end"]
```

### main.rhai —— 评论敏感词过滤

```rhai
fn comment_before_create(ctx) {
    // ctx 字段：post_slug / author_name / email（未填时为空字符串）/ content
    let banned = ["spam", "赌场", "代开发票", "加微信", "click here"];

    let content = ctx.content.to_lower();
    let author = ctx.author_name.to_lower();

    for word in banned {
        if content.contains(word) {
            print("demo-suite 拦截了 " + ctx.post_slug + " 的评论：命中违禁词 " + word);
            // 返回 block → 评论创建被拒绝（HTTP 403 comment_blocked），
            // reason 会原样进入响应的 error.message
            return #{ action: "block", reason: "评论包含违禁内容，已拒绝发布" };
        }
        if author.contains(word) {
            return #{ action: "block", reason: "昵称包含违禁内容，已拒绝发布" };
        }
    }

    // 放行；也可携带 author_name / email / content 改写字段
    #{ action: "allow" }
}
```

要点：`to_lower()` + `contains()` 实现大小写不敏感匹配；命中即 `return` block 对象
（链式调用下会立即短路）；`print()` 输出到后端 stdout，方便排查拦截情况。

### main.rhai —— 文章尾部版权声明

```rhai
fn post_after_render(ctx) {
    let footer = "<footer class=\"demo-suite-copyright\" style=\"...\">";
    footer += "本文《" + ctx.title + "》采用 CC BY-NC 4.0 许可协议，转载请注明出处。";
    footer += "</footer>";

    ctx.content_html += footer;
    ctx // 必须返回 map；后端只取其中的 content_html
}
```

要点：直接在渲染后的 HTML 字符串上拼接（沙箱没有 DOM API，就是纯字符串操作）；
样式里用 `var(--border)`、`var(--muted-foreground)` 等主题令牌，自动适配任意主题。

### inject/head.html —— 统计脚本

```html
<script>
  window.__demoSuitePV = (window.__demoSuitePV || 0) + 1;
  console.info("[demo-suite] 页面访问 #" + window.__demoSuitePV, location.pathname);
</script>
<meta name="generator" content="reedblog + demo-suite">
```

要点：`<script>` 会被前端重建为可执行元素；实际使用时替换成 Plausible / Umami /
百度统计等真实统计代码即可。`inject/body_end.html` 则演示了一个使用主题令牌的
「Powered by」悬浮徽标。

### 安装验证

打包上传并启用后：打开任意已发布文章 → 正文尾部出现版权声明、右下角出现徽标、
控制台输出访问日志；提交包含 `spam` 的评论 → 收到 403，提示「评论包含违禁内容，
已拒绝发布」。

## 9. 错误码对照表

| HTTP | code | 触发场景 |
|---|---|---|
| 422 | `invalid_package` | 非 multipart / 缺 `file` 字段；zip 解析失败；zip 为空；条目数超 4096；解压后超 64 MB；路径含 `..`/盘符/绝对路径；根层不是单个目录；缺 `manifest.toml` 或 `main.rhai` |
| 422 | `invalid_manifest` | manifest TOML 解析失败；`name` 为空；`slug` 非法或与目录名不一致；`version`/`min_app_version` 非法 semver；`hooks`/`inject` 含未知值；启用时 `min_app_version` 高于当前应用版本 |
| 422 | `script_error` | 启用时 `main.rhai` 沙箱语法编译失败（插件保持停用） |
| 409 | `plugin_exists` | 上传的 slug 与已安装插件冲突 |
| 403 | `comment_blocked` | 评论被插件 `block`（message = reason，reason 为空时「评论被插件拦截」） |
| 404 | `not_found` | 插件 slug 不存在（查询/启用/停用/删除） |
| 401 | `unauthorized` | 管理端点缺少或携带无效/过期 Bearer token |
| 422 | `validation_error` | 评论必填字段为空（含被插件改写为空的情况） |
| 503 | `not_installed` | 站点未完成安装向导（管理端点均被门禁拦截） |
| 500 | `internal_error` | 磁盘 IO / 数据库错误 |

## 10. 调试技巧

1. **看 `last_error`**：后台「插件管理」列表的「最近错误」列，或
   `GET /api/admin/plugins/:slug`。运行时错误都会记录在这里（格式
   `[hook_id] 消息`），钩子恢复正常后自动清空。
2. **看后端日志**：运行时错误同时打到 stderr（`[reedblog] 插件 <slug> 钩子 ...
   运行时错误（已跳过）`）；脚本里 `print()` 的内容打到 stdout。
3. **本地先验语法**：启用接口就会做沙箱编译，`script_error` 的 message 里带 Rhai
   报错的行号与原因；改完脚本记得**停用→启用**重新加载。
4. **常见 422 原因**：
   - `invalid_package`：打包层级不对（zip 打开后应直接是 `<slug>/` 一个目录）；
     忘了放 `main.rhai`；用了「压缩到 xxx.zip」时把父目录一起压了进去。
   - `invalid_manifest`：`slug` 含大写/下划线；slug 与目录名不一致；`version`
     写成 `1.0`；`hooks` 拼写错误（是 `comment.before_create`，不是
     `comments.before_create`）。
5. **常见 409 原因**：`plugin_exists`——先删除旧插件，或改 slug 再上传。
6. **钩子没生效的排查顺序**：插件是否 `enabled` → manifest 是否声明了该钩子 →
   函数名是否正确（`.`→`_`）→ 是否返回了 map（`comment.before_create` 必须带
   `action`）→ `last_error` 是否有记录。
7. **注入没生效的排查顺序**：manifest 是否声明 `inject` → `inject/<位置>.html`
   是否存在且非空白 → 是否在公开页面（/admin、/install 不注入）→ 是否刷新过页面。
8. **参考集成测试**：`backend/tests/extensibility.rs` 演示了安装→启用→钩子断言→
   注入断言→停用→删除的完整链路，可当作行为规格阅读；
   `cargo test --test extensibility generate_fixtures -- --ignored` 可在
   `backend/tests/fixtures/` 生成示例 zip（`hello-plugin.zip`、`my-theme.zip`）。
