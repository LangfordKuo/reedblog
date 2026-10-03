// 数据类型 —— 严格对应 docs/api-contract.md「数据形状」一节

export interface SiteInfo {
  title: string
  subtitle: string | null
  installed: boolean
}

/** GET /api/site/settings 响应（契约 SiteSettingsPublic；不含 base_url 等敏感字段） */
export interface SiteSettings {
  title: string
  subtitle: string
  description: string
  icp_number: string
  footer_text: string
  per_page: number
}

/** GET/PUT /api/admin/site/settings 响应（契约 SiteSettingsAdmin） */
export interface SiteSettingsAdmin extends SiteSettings {
  base_url: string
}

/** PUT /api/admin/site/settings 请求体（全量更新语义） */
export interface SiteSettingsSaveBody {
  title: string
  subtitle?: string
  description?: string
  icp_number?: string
  footer_text?: string
  per_page: number
  base_url?: string
}

/** SMTP TLS 模式（契约「邮件通知」条款） */
export type SmtpTls = "starttls" | "implicit" | "none"

/** 最近一次邮件发送尝试结果（内存态；契约 SmtpSettingsAdmin.last_result） */
export interface SmtpLastResult {
  ok: boolean
  message: string
  at: string
}

/** GET/PUT /api/admin/smtp 响应（契约 SmtpSettingsAdmin；密码永不返回，只有 has_password） */
export interface SmtpSettingsAdmin {
  enabled: boolean
  host: string
  port: number
  username: string
  from_name: string
  from_email: string
  to_email: string
  tls: SmtpTls
  /** 密码是否已在 config.toml [smtp] password / 环境变量中配置 */
  has_password: boolean
  last_result: SmtpLastResult | null
}

/** PUT /api/admin/smtp 请求体（部分更新语义；密码不在此接口内） */
export interface SmtpSettingsSaveBody {
  enabled?: boolean
  host?: string
  port?: number
  username?: string
  from_name?: string
  from_email?: string
  to_email?: string
  tls?: SmtpTls
}

export interface NamedRef {
  id: number
  name: string
}

export interface PostPublic {
  id: number
  title: string
  slug: string
  excerpt: string | null
  category: NamedRef | null
  tags: NamedRef[]
  published_at: string
  comment_count: number
  /** 是否置顶（契约「文章置顶与定时发布」条款）；列表卡显示「置顶」徽章 */
  is_sticky: boolean
  /** 浏览量（契约「浏览量与点赞」条款）；列表/详情卡眼睛图标显示 */
  view_count: number
  /** 点赞总数（契约「浏览量与点赞」条款） */
  likes: number
}

/** 上一篇/下一篇导航项（契约「文章上一篇/下一篇」条款）：只带标题与 slug，不含正文 */
export interface PostNavItem {
  title: string
  slug: string
}

export interface PostDetail extends PostPublic {
  content_md: string
  /** 发布时间更早的相邻文章（纯时间序，不受置顶影响）；无则 null */
  prev_post: PostNavItem | null
  /** 发布时间更晚的相邻文章；无则 null */
  next_post: PostNavItem | null
}

/** GET /api/search 单条结果（契约「全文搜索」条款） */
export interface SearchResult extends PostPublic {
  /** 命中点附近的纯文本上下文片段（不含任何 markup）；高亮由前端实现 */
  snippet: string
}

/** scheduled=定时发布（契约「文章置顶与定时发布」条款；到点自动公开可见，无后台任务） */
export type PostStatus = "draft" | "published" | "scheduled"

export interface PostAdmin {
  id: number
  title: string
  slug: string
  content_md: string
  excerpt: string | null
  status: PostStatus
  category_id: number | null
  category_name: string | null
  tag_ids: number[]
  /** status=scheduled 时即计划发布时间（RFC3339 UTC） */
  published_at: string | null
  /** 是否置顶 */
  is_sticky: boolean
  /** 浏览量（后台只读展示；契约「浏览量与点赞」条款） */
  view_count: number
  /** 点赞总数（后台只读展示，不做管理点赞） */
  likes: number
  /** 移入回收站的时刻（RFC3339 UTC；null=正常，非 null=在回收站。契约「文章回收站」条款） */
  deleted_at: string | null
  created_at: string
  updated_at: string
}

/** 修订列表摘要（契约「文章修订历史」条款）：不含正文，只给正文字符数 */
export interface PostRevisionSummary {
  id: number
  post_id: number
  title: string
  content_chars: number
  created_at: string
}

/** 单条完整修订（含 content_md/excerpt，供与当前正文做行级差异对比） */
export interface PostRevision extends PostRevisionSummary {
  content_md: string
  excerpt: string
}

export interface Category {
  id: number
  name: string
  post_count: number
}

export interface Tag {
  id: number
  name: string
  post_count: number
}

export interface CommentPub {
  id: number
  author_name: string
  content: string
  created_at: string
  /** 所属顶级楼层 id（契约「评论回复」条款）；顶级评论本身为 null */
  parent_id: number | null
  /** 被回复的中间楼层 id（仅「回复的回复」非 null；两级归一化后存储） */
  reply_to_id: number | null
  /** 被回复人作者名（reply_to_id 为 null 时同为 null） */
  reply_to_name: string | null
}

export type CommentStatus = "approved" | "hidden"

/** 评论来源：文章评论 / 页面留言（契约 2026-10-03「页面」条款扩展） */
export type CommentTargetType = "post" | "page"

export interface CommentAdmin {
  id: number
  /** 目标 id：target_type=post 时为文章 id，page 时为页面 id */
  post_id: number
  /** 目标标题（文章标题或页面标题） */
  post_title: string
  author_name: string
  email: string | null
  content: string
  status: CommentStatus
  created_at: string
  target_type: CommentTargetType
  /** 所属顶级楼层 id；顶级评论本身为 null（契约「评论回复」条款） */
  parent_id: number | null
  /** 被回复的中间楼层 id（仅「回复的回复」非 null） */
  reply_to_id: number | null
  /** 被回复人作者名 */
  reply_to_name: string | null
  /** 直接子回复条数（两级存储下即整线程楼层数；子回复恒 0）——删除连带提示用 */
  reply_count: number
}

// —— 页面（契约「页面」条款，2026-10-03 新增）——

/** custom=普通页；message_board=留言板（页尾挂留言表单）；links=友情链接（页尾渲染链接卡片） */
export type PageKind = "custom" | "message_board" | "links"

/** GET /api/pages 单条（公开列表摘要，仅 enabled；前台导航数据源） */
export interface PageSummary {
  id: number
  title: string
  slug: string
  kind: PageKind
  sort_order: number
}

/** 友情链接单条（kind=links 页面附带） */
export interface PageLink {
  id: number
  name: string
  url: string
  description: string
  sort_order: number
}

/** GET /api/pages/:slug 响应（content_html 为后端渲染产物） */
export interface PageDetail {
  id: number
  title: string
  slug: string
  kind: PageKind
  content_html: string
  sort_order: number
  updated_at: string
  links: PageLink[]
}

/** 管理端页面形状（/api/admin/pages* 响应） */
export interface PageAdmin {
  id: number
  title: string
  slug: string
  kind: PageKind
  content_md: string
  enabled: boolean
  sort_order: number
  built_in: boolean
  links: PageLink[]
  created_at: string
  updated_at: string
}

/** POST/PUT /api/admin/pages 请求体（POST 必填 title/content_md；PUT 全部可选；kind 不可改） */
export interface PageSaveBody {
  title?: string
  slug?: string
  content_md?: string
  enabled?: boolean
  sort_order?: number
  /** 全量替换语义（数组顺序即排序）；仅 kind=links 页面生效 */
  links?: PageLinkBody[]
}

/** 友情链接入参（sort_order 由服务端按数组顺序重写） */
export interface PageLinkBody {
  name: string
  url: string
  description?: string
}

export interface AuthResult {
  token: string
  username: string
  expires_at: string
}

export interface Page<T> {
  items: T[]
  total: number
  page: number
  per_page: number
}

export interface ArchiveMonth {
  year: number
  month: number
  count: number
}

// POST /api/admin/uploads 响应（filename 仅回显原始文件名；id = media 记录 id）
export interface UploadResult {
  id: number
  url: string
  size: number
  filename: string
}

// GET /api/admin/media 列表条目（契约「媒体库」条款）
export interface MediaItem {
  id: number
  url: string
  /** 原始文件名；历史文件回退存储文件名 */
  filename: string
  size: number
  mime: string
  /** 图片头解析失败为 null */
  width: number | null
  height: number | null
  created_at: string
}

// POST /api/install 请求体
export interface MysqlConfig {
  host: string
  port: number
  username: string
  password: string
  database: string
}

export interface InstallPayload {
  db_type: "sqlite" | "mysql"
  sqlite_path?: string
  mysql?: MysqlConfig
  admin: { username: string; password: string }
  site: { title: string; subtitle?: string }
}

// POST/PUT /api/admin/posts 请求体
export interface PostSaveBody {
  title: string
  slug?: string
  content_md: string
  excerpt?: string
  category_id?: number | null
  tag_ids?: number[]
  status: PostStatus
  /** 置顶（可选；POST 缺省 false、PUT 缺省保持原值） */
  is_sticky?: boolean
  /** 计划发布时间（RFC3339 UTC）：仅 status=scheduled 时接受，须为未来时间 */
  published_at?: string
}

// —— 扩展系统（插件/主题）—— 对应 docs/extensibility-contract.md

/** 插件/主题数量少不分页，但保留 items/total 形状 */
export interface Items<T> {
  items: T[]
  total: number
}

export interface PluginInfo {
  slug: string
  name: string
  version: string
  description?: string | null
  author?: string | null
  enabled: boolean
  hooks?: string[]
  inject?: string[]
  min_app_version?: string | null
  last_error?: string | null
  installed_at: string
  updated_at: string
}

export interface ThemeInfo {
  slug: string
  name: string
  version: string
  description?: string | null
  author?: string | null
  active: boolean
  builtin: boolean
  has_css: boolean
  preview_url?: string | null
  installed_at: string
  updated_at: string
}

/** 设计令牌：key 为 CSS 变量名去 -- 前缀的下划线形式；值为 HSL 分量（radius 带单位） */
export type ThemeTokens = Record<string, string>

/** GET /api/themes/active 响应（未安装时也返回 default，保证安装页有样式） */
export interface ActiveTheme {
  slug: string
  name: string
  tokens: ThemeTokens
  tokens_dark?: ThemeTokens | null
  css_url: string | null
  preview_url?: string | null
}

// —— 主题设置（扩展契约「主题设置项」条款，2026-10-03 新增）——

/** theme.toml [[settings]] 的 type 枚举 */
export type ThemeSettingType = "text" | "textarea" | "color" | "select" | "switch" | "number"

/** select 选项（后端已把字符串写法归一化为 {value, label}） */
export interface ThemeSettingOption {
  value: string
  label: string
}

/** 单项设置声明（theme.toml [[settings]]，后端归一化下发） */
export interface ThemeSettingDecl {
  key: string
  label: string
  type: ThemeSettingType
  group?: string | null
  default?: string | number | boolean | null
  options?: ThemeSettingOption[] | null
}

/** 生效值按类型输出：switch → bool、number → number、其余 → string */
export type ThemeSettingValue = string | number | boolean

/** GET /api/themes/:slug/settings 响应（公开生效值，前台渲染用） */
export interface ThemeSettingsResponse {
  slug: string
  settings: ThemeSettingDecl[]
  values: Record<string, ThemeSettingValue>
}

/** GET /api/admin/themes/active/settings-panel 响应（多一个主题名） */
export interface ThemeSettingsPanel extends ThemeSettingsResponse {
  name: string
}

// —— 主题组件（契约「主题组件」条款，2026-10-03 新增）——

/** 规范位置枚举（存储与校验与布局无关；布局降级映射见 lib/widgets.ts） */
export type WidgetPosition = "sidebar" | "left" | "right" | "footer"

/** builtin=前端内置 React 组件；custom=HTML 片段（主题声明或后台自建） */
export type WidgetKind = "builtin" | "custom"

/** 组件来源：内置注册表 / theme.toml [[widgets]] 声明 / 后台自建 */
export type WidgetSource = "builtin" | "theme" | "admin"

/** 组件参数生效值（title/count 等；custom 组件含 html） */
export type WidgetConfigValues = Record<string, string | number | boolean>

/** GET /api/themes/:slug/widgets 单条（公开生效配置，仅 enabled，已按 sort 排序） */
export interface WidgetPublic {
  key: string
  kind: WidgetKind
  label: string
  position: WidgetPosition
  sort_order: number
  config: WidgetConfigValues
}

export interface WidgetsPublicResponse {
  slug: string
  widgets: WidgetPublic[]
}

/** GET/PUT /api/admin/themes/:slug/widgets 单条（全量合并列表，含停用） */
export interface WidgetAdminItem {
  key: string
  kind: WidgetKind
  label: string
  source: WidgetSource
  enabled: boolean
  position: WidgetPosition
  sort_order: number
  config: WidgetConfigValues
  /** 参数声明（复用主题设置声明形状；后台自建 custom 为空数组） */
  params: ThemeSettingDecl[]
}

export interface WidgetsAdminResponse {
  slug: string
  positions: WidgetPosition[]
  widgets: WidgetAdminItem[]
}

/** PUT /api/admin/themes/:slug/widgets 入参单项（全量替换语义） */
export interface WidgetInput {
  key: string
  kind: WidgetKind
  enabled: boolean
  position: WidgetPosition
  sort_order: number
  config: WidgetConfigValues
}

/** GET /api/site/stats 响应（站点信息组件数据源） */
export interface SiteStats {
  post_count: number
  comment_count: number
  installed_at: string
  /** 所有文章浏览量之和（契约「浏览量与点赞」条款，2026-10-04 新增） */
  total_views: number
}

/** 点赞三接口统一响应（GET/POST/DELETE /api/posts/:slug/like） */
export interface LikeResult {
  /** 点赞新总数 */
  likes: number
  /** 当前访客（liker_key）是否已赞 */
  liked: boolean
}

export interface InjectionFragment {
  plugin: string
  html: string
}

/** GET /api/frontend/injections 响应：启用且声明了 inject 的插件片段 */
export interface FrontendInjections {
  head: InjectionFragment[]
  body_end: InjectionFragment[]
}
