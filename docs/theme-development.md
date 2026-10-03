# reedblog 主题开发指南

本文面向主题作者，内容以 reedblog 0.1.0 的实际实现为准。系统级规格见
[extensibility-contract.md](./extensibility-contract.md) 第二部分，核心 API 见
[api-contract.md](./api-contract.md)，插件开发见 [plugin-development.md](./plugin-development.md)。

---

## 1. 主题是什么

一个 reedblog 主题就是**一个目录 / 一个 zip 包**，由四部分组成：

1. **设计令牌**（`theme.toml` 的 `[tokens]` / `[tokens_dark]`）——覆盖前端
   shadcn/Tailwind 体系的 CSS 变量，决定全站配色与圆角；
2. **设置项声明**（`theme.toml` 的 `[[settings]]`，可选）——主题自描述的可配置项，
   后台按声明渲染设置面板、按主题独立保存（见 §6）；
3. **自定义 CSS**（`theme.css`，可选）——任意补充样式；
4. **静态资源**（`assets/`，可选）——字体、图片等，由后端托管。

关键特性：

- 前端**启动时**拉取激活主题并应用（公开站点与后台共用同一套机制）；
- 内置一套 `default` 主题（shadcn neutral 令牌），首次运行自动生成，**不可删除、
  不可被上传覆盖**，保证任何情况下站点有样式；
- 当前激活主题的**权威来源是 `config.toml` 的 `[themes] active`**；主题不进数据库；
- 主题目录**每次请求都从磁盘读取**：手工改动 `theme.toml` / `theme.css` / `assets/`
  后刷新前端页面即生效，无需重启后端。

## 2. 目录结构与存储位置

主题存储根目录为后端运行目录下的 `themes/`，可在 `config.toml` 配置：

```toml
[themes]
dir = "themes"        # 默认值；相对后端运行目录，也可写绝对路径
active = "default"    # 当前激活主题 slug（由后台「激活」操作写入）
```

单个主题的目录结构（目录名必须等于 `theme.toml` 中的 `slug`）：

```
themes/
  <theme-slug>/
    theme.toml      # 必需：元数据 + 设计令牌
    theme.css       # 可选：自定义 CSS
    preview.png     # 可选：后台预览图（文件名固定）
    assets/         # 可选：字体/图片等，经 /api/themes/:slug/assets/* 托管
```

扫描规则（`backend/src/themes.rs`）：

- 主题列表每次请求都扫描 `themes/` 目录，按 slug 字典序返回；
- 以 `.` 开头的目录、目录名不是合法 slug（`^[a-z0-9-]+$`，≤128 字符）的目录会被跳过；
- `theme.toml` 缺失或解析失败的目录会被**静默跳过**（不出现在列表、不报错）——
  主题「消失」时优先检查这两点。

## 3. theme.toml 全字段

```toml
name = "午夜暗色"
slug = "midnight"
version = "1.0.0"
description = "深蓝色暗色主题"
author = "作者名"

[tokens]
background = "222 47% 7%"
# ... 完整令牌见 §4

[tokens_dark]
background = "222 47% 7%"
# ... 可选
```

| 字段 | 必填 | 校验规则 | 不满足时 |
|---|---|---|---|
| `name` | 是 | trim 后非空 | 422 `invalid_manifest` |
| `slug` | 是 | 匹配 `^[a-z0-9-]+$` 且 ≤128 字符；必须与目录名一致；**不可为 `default`**（内置保护）；不得与已安装主题冲突 | 422 `invalid_manifest`；`default` → 409 `builtin_protected`；冲突 → 409 `theme_exists` |
| `version` | 是 | semver：核心必须是 `X.Y.Z` 三段数字（`-prerelease`/`+build` 后缀允许但不校验） | 422 `invalid_manifest` |
| `description` / `author` | 否 | 无，默认空字符串 | — |
| `[tokens]` | 否 | `key = "字符串"` 的映射；后端**原样下发**不校验 key；缺省为空表 | — |
| `[tokens_dark]` | 否 | 同上；缺省时响应中不出现该字段 | — |
| `[[settings]]` | 否 | 设置项声明数组，字段与校验规则见 §6.1 | 422 `invalid_manifest` |

manifest 中未知 TOML 字段会被忽略（向前兼容）。主题没有 `min_app_version`、
hooks、inject 之类字段。

## 4. 设计令牌完整清单

### 4.1 映射与取值规则

- **key → CSS 变量**：前端把 key 中的 `_` 换成 `-`、加 `--` 前缀，如
  `primary_foreground` → `--primary-foreground`；`radius` → `--radius`。
- **值的格式**：约定为 **HSL 分量串**（不带 `hsl()` 包裹），如 `"222 47% 7%"`，
  前端自动补全为 `hsl(222 47% 7%)`。
  前端同时兼容：以 `hsl(`/`rgb(`/`hwb(`/`lab(`/`lch(`/`oklab(`/`oklch(`/`color(`/
  `color-mix(`/`light-dark(`/`var(` 开头或以 `#` 开头的值**原样写入**（可以直接给
  oklch、hex 或 var 引用）。
- **`radius` 特殊**：不做颜色包装，**原样写入且必须自带单位**（如 `"0.5rem"`）。
  前端的 `--radius-sm/md/lg/xl` 由 `calc(var(--radius) ± Npx)` 派生，改一个值即可
  调整全站圆角体系。
- **未知 key 忽略**：后端原样下发所有 key，但前端只认下面清单中的
  `KNOWN_TOKEN_KEYS`（`frontend/src/lib/theme.ts`），其余静默忽略。例如内置
  default 主题的 `theme.toml` 里写了 `destructive_foreground`，前端并不认识它，
  不会生效。
- **缺失 key 回落默认**：前端应用主题前会先清掉全部已知变量再写入主题提供的
  部分，没提供的 key 自然回落到 `frontend/src/index.css` 的内置默认值
  （`:root` 亮色 / `.dark` 暗色的 oklch neutral 色板）。

### 4.2 前端认识的全部令牌 key（32 个）

> **务必覆盖完整令牌集。** 只给部分令牌时，缺失的 key 会回落到内置**浅色**默认值，
> 出现「页面背景是深色、卡片和弹层还是白色」这类混搭——尤其 `card`、`popover`、
> `input`、`sidebar_*` 这几组最容易被遗漏。

| key | CSS 变量 | 用途 |
|---|---|---|
| `radius` | `--radius` | 圆角基准值（带单位，如 `0.5rem`） |
| `background` | `--background` | 页面背景 |
| `foreground` | `--foreground` | 页面正文文字 |
| `card` | `--card` | 卡片背景 |
| `card_foreground` | `--card-foreground` | 卡片文字 |
| `popover` | `--popover` | 浮层/下拉菜单背景 |
| `popover_foreground` | `--popover-foreground` | 浮层文字 |
| `primary` | `--primary` | 主色（主按钮、链接、激活态） |
| `primary_foreground` | `--primary-foreground` | 主色上的文字 |
| `secondary` | `--secondary` | 次要按钮/徽章背景 |
| `secondary_foreground` | `--secondary-foreground` | 次要元素文字 |
| `muted` | `--muted` | 弱化区块背景 |
| `muted_foreground` | `--muted-foreground` | 弱化文字（说明、时间戳等） |
| `accent` | `--accent` | hover/选中高亮背景 |
| `accent_foreground` | `--accent-foreground` | 高亮区文字 |
| `destructive` | `--destructive` | 危险操作（删除按钮等） |
| `border` | `--border` | 全局默认边框色 |
| `input` | `--input` | 表单控件边框 |
| `ring` | `--ring` | 焦点环 |
| `chart_1` … `chart_5` | `--chart-1` … `--chart-5` | 图表系列色（1–5） |
| `sidebar` | `--sidebar` | 后台侧边栏背景 |
| `sidebar_foreground` | `--sidebar-foreground` | 侧边栏文字 |
| `sidebar_primary` | `--sidebar-primary` | 侧边栏主色 |
| `sidebar_primary_foreground` | `--sidebar-primary-foreground` | 侧边栏主色上的文字 |
| `sidebar_accent` | `--sidebar-accent` | 侧边栏高亮背景 |
| `sidebar_accent_foreground` | `--sidebar-accent-foreground` | 侧边栏高亮文字 |
| `sidebar_border` | `--sidebar-border` | 侧边栏边框 |
| `sidebar_ring` | `--sidebar-ring` | 侧边栏焦点环 |

### 4.3 `[tokens_dark]` 的语义

`[tokens_dark]` 是**可选**的暗色变体。前端会把它注入为一段
`<style id="reedblog-theme-dark">`，内容是 `.dark { --xxx: ... !important; }`——
即只有当 `<html>` 元素带有 `dark` 类时才生效。**当前版本前端没有内置亮/暗切换
开关**（不会自动加 `.dark` 类），因此：

- 做纯暗色主题：直接把暗色色板放进 `[tokens]`（它写入 `:root`，永远生效）；
- `[tokens_dark]` 用于兼容 `.dark` 类被外部（插件注入脚本、未来的切换功能等）
  加上的场景，可给同一套色板或不提供。

## 5. 主题应用机制（前端）

前端启动时（`frontend/src/main.tsx`）调用 `applyActiveTheme()`：

1. `GET /api/themes/active`（公开端点，未安装门禁白名单内——安装页也有主题样式）；
2. `[tokens]` → 写入 `document.documentElement` 的**内联样式**（先移除全部已知
   变量再写入，保证缺失 key 正确回落、热切换无残留）；
3. `[tokens_dark]` → 注入 `.dark { ... !important }` 样式（加 `!important` 是因为
   亮色令牌在内联样式上，只有 important 作者声明能压过它）；
4. `css_url` 非空 → 以 `<link id="reedblog-theme-css" rel="stylesheet">` **追加到
   `<head>` 末尾**；
5. 接口不可用（后端未起/网络错误）时**静默跳过**，页面保持 index.css 内置默认样式。

`GET /api/themes/active` 的响应形状与兜底顺序（激活主题 → 磁盘 `default` →
后端内置常量，**任何情况下都返回可用主题**）：

```json
{
  "slug": "midnight",
  "name": "午夜暗色",
  "tokens": { "background": "222 47% 7%", "...": "..." },
  "tokens_dark": { "...": "..." },
  "css_url": "/api/themes/midnight/theme.css",
  "preview_url": "/api/themes/midnight/preview.png"
}
```

（`tokens_dark`、`preview_url` 缺失时不出现；无 `theme.css` 时 `css_url` 为
`null`。）

后台「主题管理」里点「激活」后，后台页面会立即重新拉取并热切换；前台页面下次
加载生效。

## 6. 主题设置项（theme.toml `[[settings]]`）

每个主题可以**自描述**一组可配置项（仿 Typecho「外观 → 设置」）：后台
「主题设置」页按声明渲染分组表单，值**按主题 slug 独立持久化**（`theme_settings`
表），切换主题互不影响；**删除主题时其设置一并删除**（重装后回到声明默认值）。
系统级规格见 [extensibility-contract.md](./extensibility-contract.md)「主题设置项」。

设置系统是令牌管线之上的**叠加层**：`[tokens]` / `[tokens_dark]` /
`KNOWN_TOKEN_KEYS` / `.dark` 注入机制完全不变。

### 6.1 声明规格

theme.toml 中用 TOML 数组表 `[[settings]]` 声明（可声明 0~N 项；未声明的主题
面板显示「该主题无自定义设置项」）：

```toml
[[settings]]
key = "layout"                    # 必填：^[a-z0-9][a-z0-9_-]{0,63}$，同一主题内唯一
label = "页面布局"                 # 可选：面板显示名；缺省用 key
type = "select"                   # 必填：text | textarea | color | select | switch | number
group = "布局"                    # 可选：面板分组标题（≤64 字符）；缺省归入「通用」
default = "topbar-two-column"     # 可选：默认值（类型必须与 type 匹配）
options = [                       # select 必填且非空；其余类型不允许出现
  { value = "topbar-two-column", label = "顶栏导航 + 双列" },
  "topbar-minimal-three-column",  # 也接受纯字符串（label=value），下发时归一化为 {value,label}
]
```

| type | 值形态 | 校验（保存时违规 → 422 `invalid_value`） |
|---|---|---|
| `text` | 字符串 | ≤500 字符 |
| `textarea` | 字符串 | ≤5000 字符 |
| `color` | hex 颜色 | `#RGB` / `#RRGGBB` / `#RRGGBBAA`（大小写不敏感，存储时转小写） |
| `select` | 字符串 | 必须在 `options` 的 value 集合内 |
| `switch` | 布尔 | JSON bool 或 `"true"`/`"false"` 字符串 |
| `number` | 数字 | JSON number 或数字字符串（有限值；整数值规范化存储） |

声明本身违规（key 非法/重复、type 未知、select 缺 options、default 与类型不符、
非 select 带 options）→ **上传 zip 时 422 `invalid_manifest`**，主题不会被安装。

### 6.2 内置 layout 设置（前端布局骨架）

前端内置消费一个系统级 key：**`layout`**（type=select）。内置 default 主题声明了
两个选项，任何主题都可声明同 key 的设置复用这两套骨架：

- `topbar-two-column`（默认）：顶栏导航 + 主内容/侧栏双列；
- `topbar-minimal-three-column`：顶栏仅保留搜索与后台管理入口，正文左中右三列
  （左：导航/分类，中：文章流，右：侧栏信息）。

未声明 layout 或值不在上述枚举内时，前端回退双列。生效值同时写在 `<html>` 的
`data-layout` 属性上，theme.css 可用 `html[data-layout="..."]` 选择器做布局级定制。
实现在 `frontend/src/components/site-layout.tsx`（双骨架分派）。

default 主题还声明了两个示例设置：`wide_layout`（switch，宽幅正文）与
`accent_color`（color，强调色——内置消费点为文章卡片「阅读全文」链接色）。

### 6.3 API

公开（未安装门禁白名单，未安装时 `values` = 声明默认值，保证安装页有样式）：

| 端点 | 说明 |
|---|---|
| `GET /api/themes/:slug/settings` | → `{slug, settings: [ThemeSetting], values}`；磁盘无此主题 → 404；default 缺盘时以内置常量兜底 |

管理（需 Bearer）：

| 端点 | 说明 |
|---|---|
| `GET /api/admin/themes/active/settings-panel` | → `{slug, name, settings, values}`（**只服务当前激活主题**，后台面板数据源） |
| `PUT /api/admin/themes/:slug/settings` | body `{values: {key: value, ...}}` → 200 合并后最新形状；**部分更新**语义（仅写入出现的 key）；未声明 key → 422 `unknown_setting`；slug 未安装 → 404 |

`values` 是**生效值**：声明 `default` 与已存值合并（已存值优先），按类型输出
JSON（switch → bool、number → number、其余 → string）；既无 default 也未保存过
的 key 不出现在 `values` 中。

后台入口：侧边栏「主题设置」（`/admin/themes/settings`）；主题列表每行的「设置」
按钮仅激活主题可进（非激活提示先激活）。

### 6.4 CSS 变量与 data 属性约定（theme.css 消费方式）

前端拉到生效值后（启动时随 `applyActiveTheme()`、后台保存/切换主题后即时）：

1. 每个值写入 `:root` 内联 CSS 变量 **`--theme-setting-<key>`**（key 中 `_` 换成
   `-`，如 `accent_color` → `--theme-setting-accent-color`）；换主题时先清空上一
   主题的全部设置变量再写入，无残留；
2. 同时写 `<html>` 的 **`data-setting-<key-dashes>="字符串化值"`** 属性
   （switch 为 `"true"`/`"false"`）；
3. `layout` 生效值额外写 `<html>` 的 **`data-layout`** 属性。

theme.css 消费示例（见 `examples/themes/midnight/theme.css`）：

```css
/* 变量未设置时回退默认值 */
::selection {
  background: color-mix(in srgb, var(--theme-setting-accent-color, hsl(199 89% 55%)) 35%, transparent);
}

/* switch 设置用 data 属性选择器 */
html[data-setting-glow-accent="false"] ::selection {
  background: hsl(217 33% 25%);
}
```

注意：`--theme-setting-*` 写在 `documentElement` **内联样式**上，theme.css 里
直接 `var()` 引用即可；若要在 `.dark` 下给不同值，选择器优先级压不过内联声明，
需用 `html.dark[data-...] { ... }` 包一层自定义属性中转，或接受同值。

## 7. theme.css：用法与优先级

存在 `theme.css` 时，后端通过 `GET /api/themes/:slug/theme.css` 以
`text/css; charset=utf-8` 托管（文件不存在 → 404，`ThemeInfo.has_css` 为
`false`、`css_url` 为 `null`）。

优先级要点：

- 该 `<link>` 是运行时**追加到 `<head>` 末尾**的，位于前端打包样式之后——
  **同优先级**的规则天然覆盖内置样式，多数情况不需要 `!important`；
- 需要 `!important` 的场景：
  1. 覆盖**内联样式**上的令牌变量（如 `:root { --primary: ... !important; }`，
     因为令牌写在 `documentElement` 的内联样式上，普通规则压不过）；
  2. 覆盖内置样式中**写死的颜色**且不想拼选择器优先级——典型例子是 index.css 给
     文章代码块写死的浅色背景 `.prose :where(pre):not(:where([class~="not-prose"] *))`
     （暗色主题必须覆盖它，见 §10 示例）；
  3. `.dark` 类下的暗色令牌本身就以 `!important` 注入，若要再覆盖需同样使用
     `!important` 且保证选择器优先级不低于 `.dark`。
- 可以写任意 CSS；前端是 Tailwind CSS 4 + shadcn 风格组件，类名与变量用浏览器
  devtools 即可确认。

## 8. assets/：字体与图片

`assets/` 下的文件经 `GET /api/themes/:slug/assets/<相对路径>` 托管（该端点在
未安装门禁白名单内，GET 永远可达；文件不存在 → 404）。

- **在 `theme.css` 中用绝对路径引用**：

  ```css
  @font-face {
    font-family: "My Font";
    src: url("/api/themes/midnight/assets/fonts/my-font.woff2") format("woff2");
  }
  ```

- **MIME 按扩展名识别**：`css`、`js`/`mjs`、`json`、`txt`、`html`/`htm`、`png`、
  `jpg`/`jpeg`、`gif`、`svg`、`webp`、`ico`、`avif`、`woff`、`woff2`、`ttf`、
  `otf`、`eot`；未知扩展名按 `application/octet-stream` 下发。
- **防目录穿越**：路径做词法清洗（拒绝 `..`、绝对路径、盘符、反斜杠归一化），
  并二次 `canonicalize` 校验目标必须仍在该主题的 `assets/` 内；越界一律 404。

## 9. preview.png、打包与后台管理

### preview.png

可选。存在时：主题列表项的 `preview_url` 与 `GET /api/themes/active` 响应都会带上
`/api/themes/:slug/preview.png`，由后端以 `image/png` 托管。后台「主题管理」以
96×56（`object-cover` 裁切）缩略图展示，**建议提供约 480×280（≈16:10）的 PNG**，
文件名固定为 `preview.png`。（该端点不在未安装白名单内，仅后台场景使用，无影响。）

### 打包 zip

与插件包同一套规则（`backend/src/packages.rs`）：

- zip **根层必须是单个目录**，目录名 = `slug`，内含 `theme.toml`
  （缺失 → 422 `invalid_package`；根层多目录/散落文件同样拒绝）；
- 上传请求体 ≤ **32 MB**；解压后总体积 ≤ **64 MB**；条目数 ≤ **4096**；
  路径含 `..`/盘符/绝对路径拒绝；符号链接条目跳过。

```bash
# Linux / macOS（在主题目录的父目录执行）
zip -r midnight.zip midnight

# Windows PowerShell
Compress-Archive -Path midnight -DestinationPath midnight.zip
```

### 后台上传 / 激活 / 删除规则

1. 「主题管理」（`/admin/themes`）→ 选择 zip → 「上传主题」→ 201 安装成功（未激活）；
2. 「激活」→ 后端把 `config.toml` 的 `[themes] active` 改写为该 slug（权威来源），
   后台立即热切换、前台下次加载生效；
3. 「删除」→ 204，主题目录整体移除。限制：
   - `default`（builtin）**不可删除**、**不可被上传覆盖** → 409 `builtin_protected`；
   - **当前激活主题不可删除** → 409 `theme_active`（先切换到其他主题）；
4. `installed_at` / `updated_at` 取自主题目录的文件系统创建/修改时间。

管理 API（均需 Bearer）：

| 端点 | 成功响应 |
|---|---|
| `GET /api/admin/themes` | `{"items":[ThemeInfo], "total":int}`（按 slug 字典序） |
| `POST /api/admin/themes`（multipart，字段 `file` = zip） | 201 `ThemeInfo` |
| `POST /api/admin/themes/:slug/activate` | 200 `ThemeInfo`（`active:true`） |
| `DELETE /api/admin/themes/:slug` | 204（连带删除该主题的 theme_settings 行，见 §6） |
| `GET /api/admin/themes/active/settings-panel` | 200 激活主题的 `{slug, name, settings, values}`（见 §6.3） |
| `PUT /api/admin/themes/:slug/settings` | 200 合并后的 `{slug, settings, values}`（见 §6.3） |

公开端点（无需鉴权）：

| 端点 | 说明 |
|---|---|
| `GET /api/themes/active` | 激活主题的令牌 + css_url（白名单：未安装也返回 default 兜底） |
| `GET /api/themes/:slug/theme.css` | `text/css`；不存在 404（白名单） |
| `GET /api/themes/:slug/preview.png` | `image/png`；不存在 404 |
| `GET /api/themes/:slug/assets/*path` | 静态资源；不存在/越界 404（白名单） |
| `GET /api/themes/:slug/settings` | 设置声明 + 生效值（白名单：未安装时 values=声明默认值；见 §6.3） |

`ThemeInfo` 形状：

```json
{
  "slug": "midnight",
  "name": "午夜暗色",
  "version": "1.0.0",
  "description": "...",
  "author": "...",
  "active": false,
  "builtin": false,
  "has_css": true,
  "preview_url": "/api/themes/midnight/preview.png",
  "installed_at": "2026-10-03T12:00:00Z",
  "updated_at": "2026-10-03T12:00:00Z"
}
```

（`preview_url` 无 `preview.png` 时不出现；`builtin:true` 仅 `default`。）

### 内置 default 主题

- 后端首次运行与安装完成时，若 `themes/default` 不存在则自动生成
  （`theme.toml` 为 shadcn neutral 令牌，含 `[tokens_dark]`；无 `theme.css`）；
- 已存在则不动（幂等），上传覆盖一律 409；
- `GET /api/themes/active` 在磁盘没有任何可用主题时用后端内置的同一份常量兜底，
  前端永远拿得到基础样式。

## 10. 完整示例：暗色主题 midnight

仓库 [`examples/themes/midnight/`](../examples/themes/midnight/) 是一套可直接打包
安装的完整暗色主题，`[tokens]` 与 `[tokens_dark]` 均覆盖全部 32 个令牌 key，
并声明了 3 个 `[[settings]]` 设置项（强调色 / 发光选中开关 / 页脚附加文字），
theme.css 演示了 `--theme-setting-*` 变量与 `data-setting-*` 属性的消费方式（见 §6.4）。

### theme.toml（节选，完整文件见示例目录）

```toml
name = "午夜暗色"
slug = "midnight"
version = "1.0.0"
description = "reedblog 官方示例主题：深蓝色暗色调，覆盖全部设计令牌，附自定义 CSS 与静态资源示例"
author = "reedblog"

[tokens]
radius = "0.5rem"
background = "222 47% 7%"          # 深海军蓝页面底色
foreground = "210 40% 92%"         # 浅灰蓝正文
card = "222 44% 10%"               # 卡片比页面底色略亮一档
card_foreground = "210 40% 92%"
popover = "222 44% 10%"            # 浮层与卡片同档，避免弹层"发白"
popover_foreground = "210 40% 92%"
primary = "199 89% 55%"            # 天蓝色主色
primary_foreground = "222 47% 7%"  # 主色上用深色文字保证对比度
secondary = "217 33% 17%"
secondary_foreground = "210 40% 92%"
muted = "217 33% 15%"
muted_foreground = "215 20% 62%"
accent = "217 33% 19%"
accent_foreground = "199 89% 70%"
destructive = "0 72% 51%"
border = "217 33% 20%"
input = "217 33% 22%"
ring = "199 89% 48%"
chart_1 = "199 89% 55%"            # 图表五色：与主色同族的明快色
chart_2 = "160 84% 44%"
chart_3 = "262 83% 66%"
chart_4 = "38 92% 60%"
chart_5 = "330 81% 60%"
sidebar = "222 47% 6%"             # 后台侧边栏比页面底色更深一档
sidebar_foreground = "210 40% 92%"
sidebar_primary = "199 89% 55%"
sidebar_primary_foreground = "222 47% 7%"
sidebar_accent = "217 33% 17%"
sidebar_accent_foreground = "210 40% 92%"
sidebar_border = "217 33% 20%"
sidebar_ring = "199 89% 48%"

[tokens_dark]
# 与 [tokens] 同一套色板（本主题即暗色），保证 .dark 场景观感一致
radius = "0.5rem"
background = "222 47% 7%"
# ...（其余 key 与 [tokens] 相同，完整清单见示例文件）
```

配色思路：`background → card/popover → secondary/muted/accent → border/input`
逐档提亮形成层次；`primary` 用高饱和天蓝在深底上保持醒目；所有
`*_foreground` 与所在背景保持足够对比度。

### theme.css（节选）

```css
/* 内置 index.css 给文章代码块写死了浅色背景，暗色主题必须覆盖，
   用 !important 确保压过 */
.prose :where(pre):not(:where([class~="not-prose"] *)) {
  background-color: hsl(222 44% 12%) !important;
  border-color: hsl(217 33% 20%) !important;
  color: hsl(210 40% 92%) !important;
}

/* 选中文本配色 */
::selection {
  background: hsl(199 89% 55% / 35%);
}
```

自定义字体以注释形式给出（把 woff2 放进 `assets/` 后取消注释即可），`assets/`
目录内有独立的 README 说明引用路径与 MIME/穿越防护规则。

### 安装

打包（见 §9 命令）→ 后台「主题管理」上传 → 激活 → 整站（含后台）立即变为深蓝
暗色调；文章代码块、选中色、滚动条随 theme.css 生效。

## 11. 错误码对照表

| HTTP | code | 触发场景 |
|---|---|---|
| 422 | `invalid_package` | 非 multipart / 缺 `file` 字段；zip 解析失败；zip 为空；条目数超 4096；解压后超 64 MB；路径含 `..`/盘符/绝对路径；根层不是单个目录；缺 `theme.toml` |
| 422 | `invalid_manifest` | `theme.toml` 解析失败；`name` 为空；`slug` 非法或与目录名不一致；`version` 非法 semver；`[[settings]]` 声明违规（key 非法/重复、type 未知、select 缺 options、default 与类型不符等，见 §6.1） |
| 422 | `unknown_setting` | `PUT /api/admin/themes/:slug/settings` 提交了主题未声明的设置 key |
| 422 | `invalid_value` | 保存设置时值与类型不符：select 越界 / color 非 hex / switch 非布尔 / number 非数字 / text·textarea 超长 |
| 422 | `validation_error` | 保存设置的请求体缺少 `values` 字段或 `values` 非 JSON 对象 |
| 409 | `builtin_protected` | 上传覆盖 `default`；删除 `default` |
| 409 | `theme_exists` | 上传的 slug 与已安装主题冲突 |
| 409 | `theme_active` | 删除当前激活主题（需先切换） |
| 404 | `not_found` | 主题 slug 不存在（激活/删除/设置读写），或静态文件不存在 |
| 401 | `unauthorized` | 管理端点缺少或携带无效/过期 Bearer token |
| 503 | `not_installed` | 站点未完成安装向导（管理端点均被门禁拦截） |
| 500 | `internal_error` | config.toml 不可读（激活时）、磁盘 IO 错误 |

## 12. 调试技巧

1. **主题列表里看不到刚装的主题**：目录名是否合法 slug 且与 `theme.toml` 的
   `slug` 一致？`theme.toml` 是否有 TOML 语法错误？——两者都会被**静默跳过**，
   没有任何报错。
2. **激活了但没变化**：确认 `config.toml` 里 `[themes] active` 已变为目标 slug；
   前台页面需刷新（后台激活时会立即热切换）。
3. **令牌没生效/混搭**：devtools → Elements → `<html>` 的内联样式里应能看到
   `--background` 等变量；缺失的变量说明 `theme.toml` 没给该 key（回落内置默认）。
   暗色变体看 `<style id="reedblog-theme-dark">`，theme.css 看
   `<link id="reedblog-theme-css">`。
4. **写了令牌但被忽略**：key 不在 §4.2 的 32 个之内（如 `destructive_foreground`）
   ——前端只认 `KNOWN_TOKEN_KEYS`。
5. **theme.css 404**：文件名必须是 `theme.css` 且位于主题目录根层。
6. **assets 404**：URL 必须是 `/api/themes/<slug>/assets/<相对路径>`；路径中不能
   出现 `..`；确认文件真实存在于 `assets/` 下。
7. **本地开发工作流**：把主题目录直接放进 `themes/`（无需重启，列表每次请求都
   扫描磁盘）→ 后台激活 → 改 `theme.toml`/`theme.css` 后刷新页面即可看到效果；
   发布前再打 zip。
8. **代码块仍是浅色**：内置 index.css 对 `.prose pre` 写死了浅色背景，需在
   theme.css 中显式覆盖（见 §10）。
9. **设置项没出现在面板/上传被拒**：上传报 422 `invalid_manifest` 时看响应
   message（会指明哪个 key 的什么违规）；已装主题的 `[[settings]]` 改动后刷新
   后台设置页即可（声明每次请求从磁盘读取），但**保存值仍按 key 隔离在
   theme_settings 表**，删除声明中不存在的 key 的旧值不会自动清理（不再生效，
   重装主题时随删除操作一并清掉）。
10. **设置值没生效**：devtools → `<html>` 内联样式应有 `--theme-setting-<key>`
    变量、属性面板应有 `data-setting-*` 与 `data-layout`；没有则检查设置页是否
    保存成功、主题是否激活（公开端点只返回生效值，panel 只服务激活主题）。
