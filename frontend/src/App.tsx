import { Suspense, lazy } from "react"
import { Route, Routes } from "react-router-dom"

import { AdminGuard } from "@/components/admin-guard"
import { InstallGate } from "@/components/install-gate"
import { SiteLayout } from "@/components/site-layout"
import { FullPageSpinner } from "@/components/spinner"
import { Toaster } from "@/components/ui/sonner"

/**
 * 路由级代码分割：所有页面与后台布局用 React.lazy 按需加载，
 * 首屏入口只保留 React / 路由 / 安装与登录守卫等壳层。
 * Markdown 渲染栈（react-markdown + KaTeX + highlight.js，原先占入口一大半）
 * 只被文章详情与编辑器引用，随之进入各自的异步 chunk，公开首页不再加载。
 * 顶层 Suspense 兜首次加载；站点/后台布局内另有 Suspense（见对应组件），
 * 站内跳转只替换内容区、外壳不闪。
 */
const AdminBackupPage = lazy(() => import("@/pages/admin/backup"))
const AdminCategoriesPage = lazy(() => import("@/pages/admin/categories"))
const AdminCommentsPage = lazy(() => import("@/pages/admin/comments"))
const AdminEmailPage = lazy(() => import("@/pages/admin/email"))
const DashboardPage = lazy(() => import("@/pages/admin/dashboard"))
const AdminLayout = lazy(() => import("@/pages/admin/layout"))
const AdminLoginPage = lazy(() => import("@/pages/admin/login"))
const AdminMediaPage = lazy(() => import("@/pages/admin/media"))
const AdminPageEditPage = lazy(() => import("@/pages/admin/page-edit"))
const AdminPagesPage = lazy(() => import("@/pages/admin/pages"))
const AdminPluginsPage = lazy(() => import("@/pages/admin/plugins"))
const AdminPostEditPage = lazy(() => import("@/pages/admin/post-edit"))
const AdminPostsPage = lazy(() => import("@/pages/admin/posts"))
const AdminSettingsPage = lazy(() => import("@/pages/admin/settings"))
const AdminTagsPage = lazy(() => import("@/pages/admin/tags"))
const AdminThemeSettingsPage = lazy(() => import("@/pages/admin/theme-settings"))
const AdminTrashPage = lazy(() => import("@/pages/admin/trash"))
const AdminThemesPage = lazy(() => import("@/pages/admin/themes"))
const ArchivePage = lazy(() => import("@/pages/archive"))
const ArchiveMonthPage = lazy(() => import("@/pages/archive-month"))
const CategoryIndexPage = lazy(() => import("@/pages/category-index"))
const CategoryPostsPage = lazy(() => import("@/pages/category-posts"))
const HomePage = lazy(() => import("@/pages/home"))
const InstallPage = lazy(() => import("@/pages/install"))
const NotFoundPage = lazy(() => import("@/pages/not-found"))
const PageDetailPage = lazy(() => import("@/pages/page-detail"))
const PostDetailPage = lazy(() => import("@/pages/post-detail"))
const SearchPage = lazy(() => import("@/pages/search"))
const TagIndexPage = lazy(() => import("@/pages/tag-index"))
const TagPostsPage = lazy(() => import("@/pages/tag-posts"))

export default function App() {
  return (
    <>
      <Suspense fallback={<FullPageSpinner />}>
        <Routes>
          {/* 全站安装守卫：未安装重定向 /install，已安装访问 /install 重定向首页 */}
          <Route element={<InstallGate />}>
            <Route path="/install" element={<InstallPage />} />

            {/* 公开博客 */}
            <Route element={<SiteLayout />}>
              <Route index element={<HomePage />} />
              <Route path="posts/:slug" element={<PostDetailPage />} />
              {/* 页面详情（/pages/:slug，与 /posts/:slug 互不冲突；停用页后端 404） */}
              <Route path="pages/:slug" element={<PageDetailPage />} />
              <Route path="tags" element={<TagIndexPage />} />
              <Route path="tags/:name" element={<TagPostsPage />} />
              <Route path="categories" element={<CategoryIndexPage />} />
              <Route path="categories/:name" element={<CategoryPostsPage />} />
              <Route path="archive" element={<ArchivePage />} />
              <Route path="archive/:year/:month" element={<ArchiveMonthPage />} />
              <Route path="search" element={<SearchPage />} />
            </Route>

            {/* 管理后台 */}
            <Route path="admin/login" element={<AdminLoginPage />} />
            <Route path="admin" element={<AdminGuard />}>
              <Route element={<AdminLayout />}>
                <Route index element={<DashboardPage />} />
                <Route path="posts" element={<AdminPostsPage />} />
                <Route path="posts/new" element={<AdminPostEditPage />} />
                <Route path="posts/:id/edit" element={<AdminPostEditPage />} />
                {/* 文章回收站（契约「文章回收站」条款）：列表/恢复/彻底删除 */}
                <Route path="trash" element={<AdminTrashPage />} />
                <Route path="pages" element={<AdminPagesPage />} />
                <Route path="pages/new" element={<AdminPageEditPage />} />
                <Route path="pages/:id/edit" element={<AdminPageEditPage />} />
                <Route path="categories" element={<AdminCategoriesPage />} />
                <Route path="tags" element={<AdminTagsPage />} />
                <Route path="comments" element={<AdminCommentsPage />} />
                <Route path="media" element={<AdminMediaPage />} />
                <Route path="plugins" element={<AdminPluginsPage />} />
                <Route path="themes" element={<AdminThemesPage />} />
                {/* 当前激活主题的设置面板（仿 Typecho「外观 → 设置」） */}
                <Route path="themes/settings" element={<AdminThemeSettingsPage />} />
                <Route path="settings" element={<AdminSettingsPage />} />
                <Route path="backup" element={<AdminBackupPage />} />
                {/* 邮件通知（SMTP）：独立路径避免与 /admin/settings 前缀高亮互扰 */}
                <Route path="email" element={<AdminEmailPage />} />
              </Route>
            </Route>

            <Route path="*" element={<NotFoundPage />} />
          </Route>
        </Routes>
      </Suspense>
      <Toaster richColors />
    </>
  )
}
