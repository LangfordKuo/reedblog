// 数据类型 —— 严格对应 docs/api-contract.md「数据形状」一节

export interface SiteInfo {
  title: string
  subtitle: string | null
  installed: boolean
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

export interface CommentAdmin {
  id: number
  post_id: number
  post_title: string
  author_name: string
  email: string | null
  content: string
  status: CommentStatus
  created_at: string
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
