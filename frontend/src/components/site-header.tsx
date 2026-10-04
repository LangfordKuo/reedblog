import { useEffect, useState, type FormEvent } from "react"
import { Link, NavLink, useNavigate } from "react-router-dom"
import { MoonIcon, SearchIcon, SunIcon } from "lucide-react"

import { Button, buttonVariants } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import { isDarkRendered, setColorMode } from "@/lib/color-mode"
import { getPageIcon } from "@/lib/page-icons"
import type { PageSummary, SiteSettings } from "@/lib/types"
import { cn } from "@/lib/utils"

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  cn(
    "rounded-md px-3 py-1.5 text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
    isActive ? "bg-accent font-medium text-accent-foreground" : "text-muted-foreground",
  )

/** wide：wide_layout 主题设置开启时放宽容器（与 SiteLayout 主区宽度保持一致） */
export function SiteHeader({ site, wide = false }: { site: SiteSettings | null; wide?: boolean }) {
  const navigate = useNavigate()
  const [searchOpen, setSearchOpen] = useState(false)
  const [keyword, setKeyword] = useState("")
  const [dark, setDark] = useState(isDarkRendered)
  // 导航页面项（契约「页面」条款：GET /api/pages 仅 enabled，sort_order ASC）；
  // 加载失败静默降级为「首页 + 固定栏目」
  const [navPages, setNavPages] = useState<PageSummary[]>([])

  useEffect(() => {
    let cancelled = false
    api
      .pages()
      .then((ps) => {
        if (!cancelled) setNavPages(ps)
      })
      .catch(() => {
        /* 未安装/网络错误：导航只保留固定项 */
      })
    return () => {
      cancelled = true
    }
  }, [])

  // system 态下系统偏好变化时，color-mode 的单例监听会先改 <html> 的 class，
  // 这里跟随刷新图标（监听注册顺序在 color-mode 之后，读到的是切换后的状态）
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)")
    const sync = () => setDark(isDarkRendered())
    mq.addEventListener("change", sync)
    return () => mq.removeEventListener("change", sync)
  }, [])

  // 搜索入口：icon 点击展开输入框，回车/提交 → /search?q=…（URL 携带查询词）
  const submitSearch = (e: FormEvent) => {
    e.preventDefault()
    const kw = keyword.trim()
    setSearchOpen(false)
    navigate(kw ? `/search?q=${encodeURIComponent(kw)}` : "/search")
  }

  // 暗色切换：双态直切（当前暗→light，当前亮→dark；点了就固化选择，不再回 system）
  const toggleDark = () => {
    setColorMode(dark ? "light" : "dark")
    setDark(isDarkRendered())
  }

  return (
    <header className="sticky top-0 z-40 border-b bg-background/90 backdrop-blur">
      <div
        className={cn(
          "mx-auto flex h-16 w-full items-center justify-between gap-4 px-4",
          wide ? "max-w-7xl" : "max-w-5xl",
        )}
      >
        <Link to="/" className="flex min-w-0 flex-col">
          <span className="truncate text-lg font-bold tracking-tight">
            {site?.title || "reedblog"}
          </span>
          {site?.subtitle && (
            <span className="hidden truncate text-xs text-muted-foreground sm:block">
              {site.subtitle}
            </span>
          )}
        </Link>
        <nav className="flex items-center gap-0.5">
          {searchOpen ? (
            <form onSubmit={submitSearch} className="mr-1 flex items-center">
              <Input
                autoFocus
                value={keyword}
                onChange={(e) => setKeyword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    setSearchOpen(false)
                    setKeyword("")
                  }
                }}
                placeholder="搜索…"
                aria-label="搜索关键词"
                className="h-8 w-32 sm:w-48"
              />
            </form>
          ) : (
            <Button
              variant="ghost"
              size="icon"
              className="size-8 text-muted-foreground"
              aria-label="打开搜索"
              onClick={() => setSearchOpen(true)}
            >
              <SearchIcon className="size-4" />
            </Button>
          )}
          <NavLink to="/" end className={navLinkClass}>
            首页
          </NavLink>
          {/* 启用页面导航项（关于/留言板/友情链接/自定义页）；图标同左栏：自定义优先，kind 默认兜底 */}
          {navPages.map((p) => {
            const PageIcon = getPageIcon(p.icon, p.kind, p.slug)
            return (
              <NavLink
                key={p.id}
                to={`/pages/${encodeURIComponent(p.slug)}`}
                className={(s) => cn(navLinkClass(s), "inline-flex items-center gap-1.5")}
              >
                <PageIcon className="size-4" aria-hidden />
                {p.title}
              </NavLink>
            )
          })}
          <NavLink to="/archive" className={navLinkClass}>
            归档
          </NavLink>
          <NavLink to="/categories" className={navLinkClass}>
            分类
          </NavLink>
          <NavLink to="/tags" className={navLinkClass}>
            标签
          </NavLink>
          {/* 暗色切换：图标显示与当前渲染模式相反（即"点了会切到什么"） */}
          <Button
            variant="ghost"
            size="icon"
            className="ml-1 size-8 text-muted-foreground"
            aria-label={dark ? "切换为亮色模式" : "切换为暗色模式"}
            title={dark ? "切换为亮色模式" : "切换为暗色模式"}
            onClick={toggleDark}
          >
            {dark ? <SunIcon className="size-4" /> : <MoonIcon className="size-4" />}
          </Button>
          <Link
            to="/admin"
            className={cn(buttonVariants({ variant: "ghost", size: "sm" }), "ml-1")}
          >
            管理
          </Link>
        </nav>
      </div>
    </header>
  )
}
