import { Route, Routes } from "react-router-dom"

import { AdminGuard } from "@/components/admin-guard"
import { InstallGate } from "@/components/install-gate"
import { SiteLayout } from "@/components/site-layout"
import { Toaster } from "@/components/ui/sonner"
import AdminCategoriesPage from "@/pages/admin/categories"
import AdminCommentsPage from "@/pages/admin/comments"
import DashboardPage from "@/pages/admin/dashboard"
import AdminLayout from "@/pages/admin/layout"
import AdminLoginPage from "@/pages/admin/login"
import AdminPageEditPage from "@/pages/admin/page-edit"
import AdminPagesPage from "@/pages/admin/pages"
import AdminPluginsPage from "@/pages/admin/plugins"
import AdminPostEditPage from "@/pages/admin/post-edit"
import AdminPostsPage from "@/pages/admin/posts"
import AdminSettingsPage from "@/pages/admin/settings"
import AdminTagsPage from "@/pages/admin/tags"
import AdminThemeSettingsPage from "@/pages/admin/theme-settings"
import AdminThemesPage from "@/pages/admin/themes"
import ArchivePage from "@/pages/archive"
import ArchiveMonthPage from "@/pages/archive-month"
import CategoryIndexPage from "@/pages/category-index"
import CategoryPostsPage from "@/pages/category-posts"
import HomePage from "@/pages/home"
import InstallPage from "@/pages/install"
import NotFoundPage from "@/pages/not-found"
import PageDetailPage from "@/pages/page-detail"
import PostDetailPage from "@/pages/post-detail"
import SearchPage from "@/pages/search"
import TagIndexPage from "@/pages/tag-index"
import TagPostsPage from "@/pages/tag-posts"

export default function App() {
  return (
    <>
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
              <Route path="pages" element={<AdminPagesPage />} />
              <Route path="pages/new" element={<AdminPageEditPage />} />
              <Route path="pages/:id/edit" element={<AdminPageEditPage />} />
              <Route path="categories" element={<AdminCategoriesPage />} />
              <Route path="tags" element={<AdminTagsPage />} />
              <Route path="comments" element={<AdminCommentsPage />} />
              <Route path="plugins" element={<AdminPluginsPage />} />
              <Route path="themes" element={<AdminThemesPage />} />
              {/* 当前激活主题的设置面板（仿 Typecho「外观 → 设置」） */}
              <Route path="themes/settings" element={<AdminThemeSettingsPage />} />
              <Route path="settings" element={<AdminSettingsPage />} />
            </Route>
          </Route>

          <Route path="*" element={<NotFoundPage />} />
        </Route>
      </Routes>
      <Toaster richColors />
    </>
  )
}
