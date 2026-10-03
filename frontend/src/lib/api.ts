import { clearToken, getToken } from "./auth"
import type {
  ActiveTheme,
  ArchiveMonth,
  AuthResult,
  Category,
  CommentAdmin,
  CommentPub,
  CommentStatus,
  FrontendInjections,
  InstallPayload,
  Items,
  Page,
  PluginInfo,
  PostAdmin,
  PostDetail,
  PostPublic,
  PostSaveBody,
  PostStatus,
  SearchResult,
  SiteInfo,
  SiteSettings,
  SiteSettingsAdmin,
  SiteSettingsSaveBody,
  Tag,
  ThemeInfo,
  UploadResult,
} from "./types"

export class ApiError extends Error {
  status: number
  code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = "ApiError"
    this.status = status
    this.code = code
  }
}

/** 把 ApiError 转成用户可读文案；codeMap 可按错误码定制提示 */
export function errorMessage(e: unknown, codeMap?: Record<string, string>): string {
  if (e instanceof ApiError) {
    return codeMap?.[e.code] ?? e.message
  }
  if (e instanceof Error) return e.message
  return "未知错误"
}

/** 统一处理响应：204 → undefined，解析 JSON，非 2xx 抛 ApiError */
async function toResult<T>(res: Response): Promise<T> {
  if (res.status === 204) return undefined as T

  let data: unknown = null
  const text = await res.text()
  if (text) {
    try {
      data = JSON.parse(text)
    } catch {
      data = null
    }
  }

  if (!res.ok) {
    const err = (data as { error?: { code?: string; message?: string } } | null)?.error
    // token 无效/过期时清掉本地会话，路由守卫会跳登录
    if (res.status === 401 && err?.code === "unauthorized") clearToken()
    throw new ApiError(res.status, err?.code ?? "unknown_error", err?.message ?? `请求失败（HTTP ${res.status}）`)
  }

  return data as T
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {}
  if (body !== undefined) headers["Content-Type"] = "application/json"
  const token = getToken()
  if (token) headers["Authorization"] = `Bearer ${token}`

  let res: Response
  try {
    res = await fetch(`/api${path}`, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    })
  } catch {
    throw new ApiError(0, "network_error", "无法连接服务器，请确认后端服务已启动")
  }

  return toResult<T>(res)
}

/** multipart/form-data 上传（插件/主题 zip）；不手动设 Content-Type，交给浏览器带 boundary */
async function requestForm<T>(method: string, path: string, form: FormData): Promise<T> {
  const headers: Record<string, string> = {}
  const token = getToken()
  if (token) headers["Authorization"] = `Bearer ${token}`

  let res: Response
  try {
    res = await fetch(`/api${path}`, { method, headers, body: form })
  } catch {
    throw new ApiError(0, "network_error", "无法连接服务器，请确认后端服务已启动")
  }

  return toResult<T>(res)
}

function qs(params: object): string {
  const sp = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== "") sp.set(k, String(v))
  }
  const s = sp.toString()
  return s ? `?${s}` : ""
}

export interface PostQuery {
  page?: number
  per_page?: number
  tag?: string
  category?: string
  year?: number
  month?: number
}

export interface SearchPostsQuery {
  q: string
  page?: number
  per_page?: number
}

export interface AdminPostQuery {
  status?: PostStatus | "all"
  page?: number
  per_page?: number
}

export interface AdminCommentQuery {
  status?: CommentStatus | "all"
  post_id?: number
  page?: number
  per_page?: number
}

export const api = {
  // 安装向导
  installStatus: () => request<{ installed: boolean }>("GET", "/install/status"),
  install: (payload: InstallPayload) => request<{ ok: true }>("POST", "/install", payload),

  // 站点公开接口
  site: () => request<SiteInfo>("GET", "/site"),
  // 站点设置（公开；前台头部/页脚/分页默认值渲染用，不含 base_url）
  siteSettings: () => request<SiteSettings>("GET", "/site/settings"),
  posts: (q: PostQuery = {}) => request<Page<PostPublic>>("GET", `/posts${qs(q)}`),
  post: (slug: string) => request<PostDetail>("GET", `/posts/${encodeURIComponent(slug)}`),
  comments: (slug: string) =>
    request<CommentPub[]>("GET", `/posts/${encodeURIComponent(slug)}/comments`),
  createComment: (slug: string, body: { author_name: string; email?: string; content: string }) =>
    request<CommentPub>("POST", `/posts/${encodeURIComponent(slug)}/comments`, body),
  tags: () => request<Tag[]>("GET", "/tags"),
  categories: () => request<Category[]>("GET", "/categories"),
  archive: () => request<ArchiveMonth[]>("GET", "/archive"),
  // 全文搜索（已安装后公开；q 为空后端会 400，调用方保证非空）
  search: (q: SearchPostsQuery) => request<Page<SearchResult>>("GET", `/search${qs(q)}`),

  // 扩展系统公开接口（未安装门禁白名单内，无需鉴权）
  themeActive: () => request<ActiveTheme>("GET", "/themes/active"),
  frontendInjections: () => request<FrontendInjections>("GET", "/frontend/injections"),

  // 鉴权
  login: (username: string, password: string) =>
    request<AuthResult>("POST", "/auth/login", { username, password }),
  me: () => request<{ username: string }>("GET", "/auth/me"),

  // 管理接口
  admin: {
    posts: (q: AdminPostQuery = {}) =>
      request<Page<PostAdmin>>("GET", `/admin/posts${qs(q)}`),
    post: (id: number) => request<PostAdmin>("GET", `/admin/posts/${id}`),
    createPost: (body: PostSaveBody) => request<PostAdmin>("POST", "/admin/posts", body),
    updatePost: (id: number, body: Partial<PostSaveBody>) =>
      request<PostAdmin>("PUT", `/admin/posts/${id}`, body),
    deletePost: (id: number) => request<void>("DELETE", `/admin/posts/${id}`),

    categories: () => request<Category[]>("GET", "/admin/categories"),
    createCategory: (name: string) => request<Category>("POST", "/admin/categories", { name }),
    updateCategory: (id: number, name: string) =>
      request<Category>("PUT", `/admin/categories/${id}`, { name }),
    deleteCategory: (id: number) => request<void>("DELETE", `/admin/categories/${id}`),

    tags: () => request<Tag[]>("GET", "/admin/tags"),
    createTag: (name: string) => request<Tag>("POST", "/admin/tags", { name }),
    updateTag: (id: number, name: string) => request<Tag>("PUT", `/admin/tags/${id}`, { name }),
    deleteTag: (id: number) => request<void>("DELETE", `/admin/tags/${id}`),

    comments: (q: AdminCommentQuery = {}) =>
      request<Page<CommentAdmin>>("GET", `/admin/comments${qs(q)}`),
    updateComment: (id: number, status: CommentStatus) =>
      request<CommentAdmin>("PUT", `/admin/comments/${id}`, { status }),
    deleteComment: (id: number) => request<void>("DELETE", `/admin/comments/${id}`),

    // 图片上传（multipart 字段 file；仅 png/jpeg/gif/webp，服务端按 magic bytes 判定）
    uploadImage: (file: File) => {
      const form = new FormData()
      form.append("file", file)
      return requestForm<UploadResult>("POST", "/admin/uploads", form)
    },

    // 站点设置（管理端全字段，含 base_url）
    siteSettings: () => request<SiteSettingsAdmin>("GET", "/admin/site/settings"),
    updateSiteSettings: (body: SiteSettingsSaveBody) =>
      request<SiteSettingsAdmin>("PUT", "/admin/site/settings", body),

    // 插件管理（multipart 上传 zip，字段名 file）
    plugins: () => request<Items<PluginInfo>>("GET", "/admin/plugins"),
    plugin: (slug: string) => request<PluginInfo>("GET", `/admin/plugins/${encodeURIComponent(slug)}`),
    uploadPlugin: (file: File) => {
      const form = new FormData()
      form.append("file", file)
      return requestForm<PluginInfo>("POST", "/admin/plugins", form)
    },
    enablePlugin: (slug: string) =>
      request<PluginInfo>("POST", `/admin/plugins/${encodeURIComponent(slug)}/enable`),
    disablePlugin: (slug: string) =>
      request<PluginInfo>("POST", `/admin/plugins/${encodeURIComponent(slug)}/disable`),
    deletePlugin: (slug: string) =>
      request<void>("DELETE", `/admin/plugins/${encodeURIComponent(slug)}`),

    // 主题管理
    themes: () => request<Items<ThemeInfo>>("GET", "/admin/themes"),
    uploadTheme: (file: File) => {
      const form = new FormData()
      form.append("file", file)
      return requestForm<ThemeInfo>("POST", "/admin/themes", form)
    },
    activateTheme: (slug: string) =>
      request<ThemeInfo>("POST", `/admin/themes/${encodeURIComponent(slug)}/activate`),
    deleteTheme: (slug: string) =>
      request<void>("DELETE", `/admin/themes/${encodeURIComponent(slug)}`),
  },
}
