import { Link, NavLink } from "react-router-dom"

import { buttonVariants } from "@/components/ui/button"
import type { SiteInfo } from "@/lib/types"
import { cn } from "@/lib/utils"

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  cn(
    "rounded-md px-3 py-1.5 text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
    isActive ? "bg-accent font-medium text-accent-foreground" : "text-muted-foreground",
  )

export function SiteHeader({ site }: { site: SiteInfo | null }) {
  return (
    <header className="sticky top-0 z-40 border-b bg-background/90 backdrop-blur">
      <div className="mx-auto flex h-16 w-full max-w-5xl items-center justify-between gap-4 px-4">
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
          <NavLink to="/" end className={navLinkClass}>
            首页
          </NavLink>
          <NavLink to="/archive" className={navLinkClass}>
            归档
          </NavLink>
          <NavLink to="/categories" className={navLinkClass}>
            分类
          </NavLink>
          <NavLink to="/tags" className={navLinkClass}>
            标签
          </NavLink>
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
