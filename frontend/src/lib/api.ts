import { clearToken, getToken } from "./auth"
import type {
  ActiveTheme,
  ArchiveMonth,
  AuthResult,
  BackupImportResult,
  BackupInfo,
  Category,
  CommentAdmin,
  CommentPub,
  CommentStatus,
  FrontendInjections,
  InstallPayload,
  Items,
  LikeResult,
  MediaItem,
  Page,
  PageAdmin,
  PageDetail,
  PageSaveBody,
  PageSummary,
  PluginInfo,
  PostAdmin,
  PostDetail,
  PostPublic,
  PostRevision,
  PostRevisionSummary,
  PostSaveBody,
  PostStatus,
  ProfileAdmin,
  ProfileSaveBody,
  SearchResult,
  SiteInfo,
  SiteSettings,
  SiteSettingsAdmin,
  SiteSettingsSaveBody,
  SiteStats,
  SmtpSettingsAdmin,
  SmtpSettingsSaveBody,
  Tag,
  ThemeInfo,
  ThemeSettingValue,
  ThemeSettingsPanel,
  ThemeSettingsResponse,
  UploadResult,
  WidgetInput,
  WidgetsAdminResponse,
  WidgetsPublicResponse,
} from "./types"

export class ApiError extends Error {
  status: number
  code: string
  /** 429 响应携带的 Retry-After 剩余秒数（契约「反滥用」）；无该头时为 undefined */
  retryAfter?: number

  constructor(status: number, code: string, message: string, retryAfter?: number) {
    super(message)
    this.name = "ApiError"
    this.status = status
    this.code = code
    this.retryAfter = retryAfter
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
    // 429（反滥用限流/登录退避）带 Retry-After 剩余秒数；只认整数秒格式
    const raw = res.headers.get("Retry-After")?.trim()
    const retryAfter = raw && /^\d+$/.test(raw) ? Number(raw) : undefined
    throw new ApiError(
      res.status,
      err?.code ?? "unknown_error",
      err?.message ?? `请求失败（HTTP ${res.status}）`,
      retryAfter,
    )
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

/** 带 Bearer 的文件下载（备份导出等）：返回 Blob 与 Content-Disposition 里的文件名。
 * 导出接口必须带 Authorization 头，因此不能用 <a href> 直链，只能 fetch → Blob →
 * 前端触发下载。非 2xx 时复用 toResult 的错误解析（后端仍返回 JSON 错误形状） */
async function requestBlob(
  method: string,
  path: string,
): Promise<{ blob: Blob; filename: string | null }> {
  const headers: Record<string, string> = {}
  const token = getToken()
  if (token) headers["Authorization"] = `Bearer ${token}`

  let res: Response
  try {
    res = await fetch(`/api${path}`, { method, headers })
  } catch {
    throw new ApiError(0, "network_error", "无法连接服务器，请确认后端服务已启动")
  }
  if (!res.ok) {
    await toResult<unknown>(res) // 解析错误形状并抛出 ApiError
    throw new ApiError(res.status, "unknown_error", `请求失败（HTTP ${res.status}）`)
  }

  const disposition = res.headers.get("Content-Disposition") ?? ""
  const match = /filename="?([^";]+)"?/.exec(disposition)
  return { blob: await res.blob(), filename: match ? match[1] : null }
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
  /** recent（默认）按置顶+发布时间；hot 按浏览量→评论数（热门文章组件数据源） */
  order?: "recent" | "hot"
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
  // 站点统计（公开；站点信息组件数据源）
  siteStats: () => request<SiteStats>("GET", "/site/stats"),
  posts: (q: PostQuery = {}) => request<Page<PostPublic>>("GET", `/posts${qs(q)}`),
  post: (slug: string) => request<PostDetail>("GET", `/posts/${encodeURIComponent(slug)}`),
  // 相关文章推荐（契约「相关文章推荐」条款）：非分页裸数组；无共享标签/分类时为空数组，
  // 前端据此整块不渲染；limit 缺省由后端取 5
  related: (slug: string, limit?: number) =>
    request<PostPublic[]>("GET", `/posts/${encodeURIComponent(slug)}/related${qs({ limit })}`),
  comments: (slug: string) =>
    request<CommentPub[]>("GET", `/posts/${encodeURIComponent(slug)}/comments`),
  // 点赞三接口（契约「浏览量与点赞」条款；liker_key 为 localStorage 匿名 id，
  // 重复点赞/取消均幂等；DELETE 走 query 传 key——代理兼容性最稳）
  likeStatus: (slug: string, likerKey: string) =>
    request<LikeResult>(
      "GET",
      `/posts/${encodeURIComponent(slug)}/like${qs({ liker_key: likerKey })}`,
    ),
  likePost: (slug: string, likerKey: string) =>
    request<LikeResult>("POST", `/posts/${encodeURIComponent(slug)}/like`, {
      liker_key: likerKey,
    }),
  unlikePost: (slug: string, likerKey: string) =>
    request<LikeResult>(
      "DELETE",
      `/posts/${encodeURIComponent(slug)}/like${qs({ liker_key: likerKey })}`,
    ),
  // parent_id：回复的父评论 id（可选；后端做两级归一化，见契约「评论回复」）；
  // website：蜜罐字段（契约「反滥用」，仅机器人会填；真人不可见）
  createComment: (
    slug: string,
    body: {
      author_name: string
      email?: string
      content: string
      parent_id?: number
      website?: string
    },
  ) => request<CommentPub>("POST", `/posts/${encodeURIComponent(slug)}/comments`, body),
  tags: () => request<Tag[]>("GET", "/tags"),
  categories: () => request<Category[]>("GET", "/categories"),
  archive: () => request<ArchiveMonth[]>("GET", "/archive"),
  // 页面（公开；仅 enabled，sort_order ASC——同时是前台顶栏导航数据源）
  pages: () => request<PageSummary[]>("GET", "/pages"),
  page: (slug: string) => request<PageDetail>("GET", `/pages/${encodeURIComponent(slug)}`),
  // 留言板留言（仅 kind=message_board 的启用页面；复用评论管线，先发后审；
  // parent_id 回复机制与文章评论完全一致，见契约「评论回复」）
  pageComments: (slug: string) =>
    request<CommentPub[]>("GET", `/pages/${encodeURIComponent(slug)}/comments`),
  createPageComment: (
    slug: string,
    body: {
      author_name: string
      email?: string
      content: string
      parent_id?: number
      website?: string
    },
  ) => request<CommentPub>("POST", `/pages/${encodeURIComponent(slug)}/comments`, body),
  // 全文搜索（已安装后公开；q 为空后端会 400，调用方保证非空）
  search: (q: SearchPostsQuery) => request<Page<SearchResult>>("GET", `/search${qs(q)}`),

  // 扩展系统公开接口（未安装门禁白名单内，无需鉴权）
  themeActive: () => request<ActiveTheme>("GET", "/themes/active"),
  // 主题设置生效值（声明 default 与已存值合并；未安装时 default 也可读）
  themeSettings: (slug: string) =>
    request<ThemeSettingsResponse>("GET", `/themes/${encodeURIComponent(slug)}/settings`),
  // 主题组件生效配置（仅 enabled，按 position+sort 渲染；未安装时=内置默认启用集）
  themeWidgets: (slug: string) =>
    request<WidgetsPublicResponse>("GET", `/themes/${encodeURIComponent(slug)}/widgets`),
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
    // 行内快捷置顶/取消置顶（契约「文章置顶与定时发布」条款）
    setPostSticky: (id: number, isSticky: boolean) =>
      request<PostAdmin>("PATCH", `/admin/posts/${id}/sticky`, { is_sticky: isSticky }),
    // 契约「文章回收站」条款：DELETE 为移入回收站（软删除，204）；回收站列表/恢复/彻底删除
    deletePost: (id: number) => request<void>("DELETE", `/admin/posts/${id}`),
    trashPosts: (q: { page?: number; per_page?: number } = {}) =>
      request<Page<PostAdmin>>("GET", `/admin/posts/trash${qs(q)}`),
    restorePost: (id: number) =>
      request<PostAdmin>("POST", `/admin/posts/${id}/restore`),
    purgePost: (id: number) => request<void>("DELETE", `/admin/posts/${id}/purge`),

    // 修订历史（契约「文章修订历史」条款）：列表摘要不含正文；单条完整供差异对比；
    // 恢复写回内容并生成一条新修订（返回 PostAdmin）
    postRevisions: (id: number) =>
      request<PostRevisionSummary[]>("GET", `/admin/posts/${id}/revisions`),
    postRevision: (id: number, revId: number) =>
      request<PostRevision>("GET", `/admin/posts/${id}/revisions/${revId}`),
    restorePostRevision: (id: number, revId: number) =>
      request<PostAdmin>("POST", `/admin/posts/${id}/revisions/${revId}/restore`),

    categories: () => request<Category[]>("GET", "/admin/categories"),
    createCategory: (name: string) => request<Category>("POST", "/admin/categories", { name }),
    updateCategory: (id: number, name: string) =>
      request<Category>("PUT", `/admin/categories/${id}`, { name }),
    deleteCategory: (id: number) => request<void>("DELETE", `/admin/categories/${id}`),

    tags: () => request<Tag[]>("GET", "/admin/tags"),
    createTag: (name: string) => request<Tag>("POST", "/admin/tags", { name }),
    updateTag: (id: number, name: string) => request<Tag>("PUT", `/admin/tags/${id}`, { name }),
    deleteTag: (id: number) => request<void>("DELETE", `/admin/tags/${id}`),

    // 页面管理（kind 不可改；links 全量替换语义，仅 kind=links 页面生效）
    pages: () => request<PageAdmin[]>("GET", "/admin/pages"),
    page: (id: number) => request<PageAdmin>("GET", `/admin/pages/${id}`),
    createPage: (body: PageSaveBody) => request<PageAdmin>("POST", "/admin/pages", body),
    updatePage: (id: number, body: PageSaveBody) =>
      request<PageAdmin>("PUT", `/admin/pages/${id}`, body),
    togglePage: (id: number) => request<PageAdmin>("PATCH", `/admin/pages/${id}/toggle`),
    deletePage: (id: number) => request<void>("DELETE", `/admin/pages/${id}`),

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

    // 媒体库（契约「媒体库」条款）：分页列表（含历史文件惰性扫描）/ 删除（连带删磁盘文件）
    media: (q: { page?: number; per_page?: number } = {}) =>
      request<Page<MediaItem>>("GET", `/admin/media${qs(q)}`),
    deleteMedia: (id: number) => request<void>("DELETE", `/admin/media/${id}`),

    // 备份与恢复（契约「备份与恢复」条款，2026-10-04 新增）：
    // 导出走带 Bearer 的 Blob 下载；导入为 multipart（file + confirm=REPLACE，危险操作显式确认）
    backupInfo: () => request<BackupInfo>("GET", "/admin/backup/info"),
    exportBackup: () => requestBlob("GET", "/admin/backup/export"),
    importBackup: (file: File, confirm: string) => {
      const form = new FormData()
      form.append("file", file)
      form.append("confirm", confirm)
      return requestForm<BackupImportResult>("POST", "/admin/backup/import", form)
    },

    // 站点设置（管理端全字段，含 base_url）
    siteSettings: () => request<SiteSettingsAdmin>("GET", "/admin/site/settings"),
    updateSiteSettings: (body: SiteSettingsSaveBody) =>
      request<SiteSettingsAdmin>("PUT", "/admin/site/settings", body),

    // 管理员资料 / 用户设置（契约「管理员资料 / 用户设置」条款）：改用户名/密码都必须
    // 带 current_password；**改完不会让已签发的 JWT 失效**（其它已登录设备保持在线）
    profile: () => request<ProfileAdmin>("GET", "/admin/profile"),
    updateProfile: (body: ProfileSaveBody) =>
      request<ProfileAdmin>("PUT", "/admin/profile", body),

    // 邮件通知（契约「邮件通知（SMTP）」条款）：密码永不返回，只有 has_password；
    // testSmtp 同步等待发送结果（失败时抛 ApiError，message 即后端给的明确原因）
    smtpSettings: () => request<SmtpSettingsAdmin>("GET", "/admin/smtp"),
    updateSmtpSettings: (body: SmtpSettingsSaveBody) =>
      request<SmtpSettingsAdmin>("PUT", "/admin/smtp", body),
    testSmtp: () => request<{ ok: true }>("POST", "/admin/smtp/test"),

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

    // 主题设置（panel 只服务当前激活主题；PUT 部分更新语义，按 slug 隔离存储）
    themeSettingsPanel: () =>
      request<ThemeSettingsPanel>("GET", "/admin/themes/active/settings-panel"),
    updateThemeSettings: (slug: string, values: Record<string, ThemeSettingValue>) =>
      request<ThemeSettingsResponse>(
        "PUT",
        `/admin/themes/${encodeURIComponent(slug)}/settings`,
        { values },
      ),

    // 主题组件管理（GET 全量合并配置；PUT 全量替换保存，契约「主题组件」）
    themeWidgets: (slug: string) =>
      request<WidgetsAdminResponse>("GET", `/admin/themes/${encodeURIComponent(slug)}/widgets`),
    updateThemeWidgets: (slug: string, widgets: WidgetInput[]) =>
      request<WidgetsAdminResponse>(
        "PUT",
        `/admin/themes/${encodeURIComponent(slug)}/widgets`,
        { widgets },
      ),
  },
}
