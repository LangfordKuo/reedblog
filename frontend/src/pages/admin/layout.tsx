import { useEffect, useState } from "react"
import { Link, NavLink, Outlet, useLocation, useNavigate } from "react-router-dom"
import {
  DatabaseBackupIcon,
  ExternalLinkIcon,
  FilesIcon,
  FileTextIcon,
  FolderIcon,
  ImageIcon,
  LayoutDashboardIcon,
  LogOutIcon,
  MailIcon,
  MenuIcon,
  MessageSquareIcon,
  PaletteIcon,
  PuzzleIcon,
  SettingsIcon,
  SlidersHorizontalIcon,
  TagsIcon,
  XIcon,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { clearToken, getStoredUsername } from "@/lib/auth"
import { cn } from "@/lib/utils"

const navItems = [
  { to: "/admin", label: "仪表盘", icon: LayoutDashboardIcon, end: true },
  { to: "/admin/posts", label: "文章管理", icon: FileTextIcon, end: false },
  { to: "/admin/pages", label: "页面管理", icon: FilesIcon, end: false },
  { to: "/admin/categories", label: "分类管理", icon: FolderIcon, end: false },
  { to: "/admin/tags", label: "标签管理", icon: TagsIcon, end: false },
  { to: "/admin/comments", label: "评论管理", icon: MessageSquareIcon, end: false },
  { to: "/admin/media", label: "媒体库", icon: ImageIcon, end: false },
  { to: "/admin/plugins", label: "插件", icon: PuzzleIcon, end: false },
  // 主题与其设置面板为父子路径，均用精确匹配避免双重高亮
  { to: "/admin/themes", label: "主题", icon: PaletteIcon, end: true },
  { to: "/admin/themes/settings", label: "主题设置", icon: SlidersHorizontalIcon, end: true },
  { to: "/admin/settings", label: "站点管理", icon: SettingsIcon, end: false },
  { to: "/admin/email", label: "邮件通知", icon: MailIcon, end: true },
  { to: "/admin/backup", label: "备份", icon: DatabaseBackupIcon, end: true },
]

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  cn(
    "flex items-center gap-2 rounded-md px-3 py-2 text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
    isActive ? "bg-accent font-medium text-accent-foreground" : "text-muted-foreground",
  )

export default function AdminLayout() {
  const [drawerOpen, setDrawerOpen] = useState(false)
  const location = useLocation()
  const navigate = useNavigate()
  const username = getStoredUsername() ?? "管理员"

  // 路由切换后收起移动端抽屉
  useEffect(() => {
    setDrawerOpen(false)
  }, [location.pathname])

  const logout = () => {
    clearToken()
    toast.success("已退出登录")
    navigate("/admin/login")
  }

  return (
    <div className="min-h-screen bg-muted/30 md:flex">
      {/* 移动端顶栏 */}
      <div className="sticky top-0 z-30 flex h-14 items-center justify-between border-b bg-background px-4 md:hidden">
        <Button variant="ghost" size="icon" aria-label="打开菜单" onClick={() => setDrawerOpen(true)}>
          <MenuIcon />
        </Button>
        <span className="font-semibold">reedblog 控制台</span>
        <Button variant="ghost" size="icon" aria-label="退出登录" onClick={logout}>
          <LogOutIcon />
        </Button>
      </div>

      {/* 移动端抽屉遮罩 */}
      {drawerOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/50 md:hidden"
          onClick={() => setDrawerOpen(false)}
          aria-hidden
        />
      )}

      {/* 侧栏 */}
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-50 flex w-64 flex-col border-r bg-background transition-transform md:sticky md:top-0 md:h-screen md:translate-x-0",
          drawerOpen ? "translate-x-0" : "-translate-x-full",
        )}
      >
        <div className="flex h-14 items-center justify-between border-b px-4">
          <Link to="/admin" className="font-bold tracking-tight">
            reedblog 控制台
          </Link>
          <Button
            variant="ghost"
            size="icon"
            className="md:hidden"
            aria-label="关闭菜单"
            onClick={() => setDrawerOpen(false)}
          >
            <XIcon />
          </Button>
        </div>
        <nav className="grid content-start flex-1 gap-1 overflow-y-auto p-3">
          {navItems.map((item) => (
            <NavLink key={item.to} to={item.to} end={item.end} className={navLinkClass}>
              <item.icon className="size-4" />
              {item.label}
            </NavLink>
          ))}
        </nav>
        <div className="grid gap-1 border-t p-3">
          <div className="truncate px-3 py-1 text-xs text-muted-foreground">
            已登录：{username}
          </div>
          <Button variant="ghost" className="justify-start" asChild>
            <Link to="/" target="_blank">
              <ExternalLinkIcon />
              查看站点
            </Link>
          </Button>
          <Separator className="my-1" />
          <Button
            variant="ghost"
            className="justify-start text-destructive hover:text-destructive"
            onClick={logout}
          >
            <LogOutIcon />
            退出登录
          </Button>
        </div>
      </aside>

      {/* 主内容区 */}
      <main className="min-w-0 flex-1 p-4 md:p-8">
        <div className="mx-auto w-full max-w-5xl">
          <Outlet />
        </div>
      </main>
    </div>
  )
}
