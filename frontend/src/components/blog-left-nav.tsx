import { useEffect, useState } from "react"
import { NavLink } from "react-router-dom"
import { ArchiveIcon, FolderIcon, HomeIcon, TagsIcon } from "lucide-react"

import { api } from "@/lib/api"
import { getPageIcon } from "@/lib/page-icons"
import type { Category, PageSummary } from "@/lib/types"
import { cn } from "@/lib/utils"

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  cn(
    "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
    isActive ? "bg-accent font-medium text-accent-foreground" : "text-muted-foreground",
  )

/**
 * 三列布局左栏（layout=topbar-minimal-three-column）：
 * 站点导航（首页 / 启用页面 / 归档 / 分类 / 标签索引）+ 分类直达列表。
 */
export function BlogLeftNav() {
  const [navPages, setNavPages] = useState<PageSummary[]>([])
  const [categories, setCategories] = useState<Category[]>([])

  useEffect(() => {
    // 加载失败静默降级：导航只保留固定项（与 SiteHeader 同款策略）
    api.pages().then(setNavPages).catch(() => {})
    api.categories().then(setCategories).catch(() => {})
  }, [])

  return (
    <nav className="flex flex-col gap-6" aria-label="站点导航">
      <div className="flex flex-col gap-0.5">
        <NavLink to="/" end className={navLinkClass}>
          <HomeIcon className="size-4" />
          首页
        </NavLink>
        {navPages.map((p) => {
          // 图标：后台自定义优先，空/未知回退 kind 默认（契约「页面-图标」）
          const PageIcon = getPageIcon(p.icon, p.kind, p.slug)
          return (
            <NavLink
              key={p.id}
              to={`/pages/${encodeURIComponent(p.slug)}`}
              className={navLinkClass}
            >
              <PageIcon className="size-4" aria-hidden />
              {p.title}
            </NavLink>
          )
        })}
        <NavLink to="/archive" className={navLinkClass}>
          <ArchiveIcon className="size-4" />
          归档
        </NavLink>
        <NavLink to="/categories" className={navLinkClass}>
          <FolderIcon className="size-4" />
          分类
        </NavLink>
        <NavLink to="/tags" className={navLinkClass}>
          <TagsIcon className="size-4" />
          标签
        </NavLink>
      </div>

      {categories.length > 0 && (
        <div className="flex flex-col gap-1">
          <div className="px-2 text-xs font-semibold text-muted-foreground">分类</div>
          {categories.map((c) => (
            <NavLink
              key={c.id}
              to={`/categories/${encodeURIComponent(c.name)}`}
              className={({ isActive }) =>
                cn(
                  "flex items-center justify-between rounded-md px-2 py-1 text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
                  isActive ? "font-medium text-foreground" : "text-muted-foreground",
                )
              }
            >
              <span className="truncate">{c.name}</span>
              <span className="ml-2 text-xs opacity-70">{c.post_count}</span>
            </NavLink>
          ))}
        </div>
      )}
    </nav>
  )
}
