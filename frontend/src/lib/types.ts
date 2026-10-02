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
