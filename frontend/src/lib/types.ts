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
}

export interface PostDetail extends PostPublic {
  content_md: string
}

/** GET /api/search 单条结果（契约「全文搜索」条款） */
export interface SearchResult extends PostPublic {
  /** 命中点附近的纯文本上下文片段（不含任何 markup）；高亮由前端实现 */
  snippet: string
}

export type PostStatus = "draft" | "published"

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
  published_at: string | null
  created_at: string
  updated_at: string
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

// POST /api/admin/uploads 响应（filename 仅回显原始文件名）
export interface UploadResult {
  url: string
  size: number
  filename: string
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

export interface InjectionFragment {
  plugin: string
  html: string
}

/** GET /api/frontend/injections 响应：启用且声明了 inject 的插件片段 */
export interface FrontendInjections {
  head: InjectionFragment[]
  body_end: InjectionFragment[]
}
