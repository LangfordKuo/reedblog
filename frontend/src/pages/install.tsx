import { useState, type FormEvent } from "react"
import { useNavigate } from "react-router-dom"
import { DatabaseIcon, Loader2Icon, ServerIcon } from "lucide-react"
import { toast } from "sonner"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Separator } from "@/components/ui/separator"
import { ApiError, api, errorMessage } from "@/lib/api"
import { markInstalled } from "@/lib/install-state"
import type { InstallPayload } from "@/lib/types"
import { cn } from "@/lib/utils"

type DbType = "sqlite" | "mysql"

function SectionTitle({ step, title }: { step: number; title: string }) {
  return (
    <div className="flex items-center gap-2">
      <span className="flex size-5 items-center justify-center rounded-full bg-primary text-[11px] font-semibold text-primary-foreground">
        {step}
      </span>
      <h2 className="text-sm font-semibold">{title}</h2>
    </div>
  )
}

export default function InstallPage() {
  const navigate = useNavigate()

  const [dbType, setDbType] = useState<DbType>("sqlite")
  const [sqlitePath, setSqlitePath] = useState("reedblog.db")
  const [mysqlHost, setMysqlHost] = useState("localhost")
  const [mysqlPort, setMysqlPort] = useState("3306")
  const [mysqlUser, setMysqlUser] = useState("root")
  const [mysqlPass, setMysqlPass] = useState("")
  const [mysqlDb, setMysqlDb] = useState("reedblog")

  const [siteTitle, setSiteTitle] = useState("")
  const [siteSubtitle, setSiteSubtitle] = useState("")
  const [adminUser, setAdminUser] = useState("")
  const [adminPass, setAdminPass] = useState("")
  const [adminConfirm, setAdminConfirm] = useState("")

  const [submitting, setSubmitting] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (submitting) return
    setFormError(null)

    if (!siteTitle.trim()) {
      setFormError("请填写站点标题")
      return
    }
    if (!adminUser.trim()) {
      setFormError("请填写管理员用户名")
      return
    }
    if (!adminPass) {
      setFormError("请填写管理员密码")
      return
    }
    if (adminPass !== adminConfirm) {
      setFormError("两次输入的密码不一致")
      return
    }

    const payload: InstallPayload = {
      db_type: dbType,
      admin: { username: adminUser.trim(), password: adminPass },
      site: {
        title: siteTitle.trim(),
        ...(siteSubtitle.trim() ? { subtitle: siteSubtitle.trim() } : {}),
      },
    }

    if (dbType === "sqlite") {
      payload.sqlite_path = sqlitePath.trim() || "reedblog.db"
    } else {
      if (!mysqlHost.trim() || !mysqlUser.trim() || !mysqlDb.trim()) {
        setFormError("请完整填写 MySQL 连接信息（主机 / 用户名 / 数据库）")
        return
      }
      const port = Number(mysqlPort)
      if (!Number.isInteger(port) || port <= 0 || port > 65535) {
        setFormError("MySQL 端口无效（应为 1-65535 的整数）")
        return
      }
      payload.mysql = {
        host: mysqlHost.trim(),
        port,
        username: mysqlUser.trim(),
        password: mysqlPass,
        database: mysqlDb.trim(),
      }
    }

    setSubmitting(true)
    try {
      await api.install(payload)
      markInstalled()
      toast.success("安装完成，请登录")
      navigate("/admin/login", { replace: true, state: { installed: true } })
    } catch (err) {
      // 已安装（可能在别的标签页完成了安装）→ 直接回首页
      if (err instanceof ApiError && err.status === 409) {
        markInstalled()
        navigate("/", { replace: true })
        return
      }
      setFormError(errorMessage(err))
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-muted/40 p-4">
      <Card className="w-full max-w-xl">
        <CardHeader>
          <CardTitle className="text-xl">安装 reedblog</CardTitle>
          <CardDescription>完成以下初始化配置，开启你的博客之旅。</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="grid gap-6">
            {/* 1. 站点信息 */}
            <div className="grid gap-4">
              <SectionTitle step={1} title="站点信息" />
              <div className="grid gap-1.5">
                <Label htmlFor="site-title">站点标题 *</Label>
                <Input
                  id="site-title"
                  value={siteTitle}
                  onChange={(e) => setSiteTitle(e.target.value)}
                  placeholder="例如：我的小站"
                  maxLength={100}
                />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="site-subtitle">副标题（选填）</Label>
                <Input
                  id="site-subtitle"
                  value={siteSubtitle}
                  onChange={(e) => setSiteSubtitle(e.target.value)}
                  placeholder="一句话介绍这个站点"
                  maxLength={200}
                />
              </div>
            </div>

            <Separator />

            {/* 2. 数据库 */}
            <div className="grid gap-4">
              <SectionTitle step={2} title="数据库" />
              <div className="grid grid-cols-2 gap-3">
                <button
                  type="button"
                  onClick={() => setDbType("sqlite")}
                  className={cn(
                    "flex flex-col items-start gap-1 rounded-lg border p-4 text-left transition-colors hover:border-primary/50",
                    dbType === "sqlite" && "border-primary bg-primary/5 ring-1 ring-primary",
                  )}
                  aria-pressed={dbType === "sqlite"}
                >
                  <span className="flex items-center gap-2 text-sm font-medium">
                    <DatabaseIcon className="size-4" />
                    SQLite
                  </span>
                  <span className="text-xs text-muted-foreground">
                    单文件数据库，零配置，推荐个人博客使用
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => setDbType("mysql")}
                  className={cn(
                    "flex flex-col items-start gap-1 rounded-lg border p-4 text-left transition-colors hover:border-primary/50",
                    dbType === "mysql" && "border-primary bg-primary/5 ring-1 ring-primary",
                  )}
                  aria-pressed={dbType === "mysql"}
                >
                  <span className="flex items-center gap-2 text-sm font-medium">
                    <ServerIcon className="size-4" />
                    MySQL
                  </span>
                  <span className="text-xs text-muted-foreground">
                    连接已有的 MySQL / MariaDB 服务
                  </span>
                </button>
              </div>

              {dbType === "sqlite" ? (
                <div className="grid gap-1.5">
                  <Label htmlFor="sqlite-path">数据文件路径</Label>
                  <Input
                    id="sqlite-path"
                    value={sqlitePath}
                    onChange={(e) => setSqlitePath(e.target.value)}
                    placeholder="reedblog.db"
                  />
                  <p className="text-xs text-muted-foreground">
                    相对于后端运行目录，默认 reedblog.db
                  </p>
                </div>
              ) : (
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="grid gap-1.5">
                    <Label htmlFor="mysql-host">主机 *</Label>
                    <Input
                      id="mysql-host"
                      value={mysqlHost}
                      onChange={(e) => setMysqlHost(e.target.value)}
                      placeholder="localhost"
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="mysql-port">端口 *</Label>
                    <Input
                      id="mysql-port"
                      value={mysqlPort}
                      onChange={(e) => setMysqlPort(e.target.value)}
                      inputMode="numeric"
                      placeholder="3306"
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="mysql-user">用户名 *</Label>
                    <Input
                      id="mysql-user"
                      value={mysqlUser}
                      onChange={(e) => setMysqlUser(e.target.value)}
                      placeholder="root"
                    />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="mysql-pass">密码</Label>
                    <Input
                      id="mysql-pass"
                      type="password"
                      value={mysqlPass}
                      onChange={(e) => setMysqlPass(e.target.value)}
                      placeholder="数据库密码"
                    />
                  </div>
                  <div className="grid gap-1.5 sm:col-span-2">
                    <Label htmlFor="mysql-db">数据库名 *</Label>
                    <Input
                      id="mysql-db"
                      value={mysqlDb}
                      onChange={(e) => setMysqlDb(e.target.value)}
                      placeholder="reedblog"
                    />
                  </div>
                </div>
              )}
            </div>

            <Separator />

            {/* 3. 管理员账号 */}
            <div className="grid gap-4">
              <SectionTitle step={3} title="管理员账号" />
              <div className="grid gap-1.5">
                <Label htmlFor="admin-user">用户名 *</Label>
                <Input
                  id="admin-user"
                  value={adminUser}
                  onChange={(e) => setAdminUser(e.target.value)}
                  placeholder="登录后台使用的用户名"
                  autoComplete="username"
                  maxLength={50}
                />
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="grid gap-1.5">
                  <Label htmlFor="admin-pass">密码 *</Label>
                  <Input
                    id="admin-pass"
                    type="password"
                    value={adminPass}
                    onChange={(e) => setAdminPass(e.target.value)}
                    autoComplete="new-password"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="admin-confirm">确认密码 *</Label>
                  <Input
                    id="admin-confirm"
                    type="password"
                    value={adminConfirm}
                    onChange={(e) => setAdminConfirm(e.target.value)}
                    autoComplete="new-password"
                  />
                </div>
              </div>
            </div>

            {formError && (
              <Alert variant="destructive">
                <AlertDescription>{formError}</AlertDescription>
              </Alert>
            )}

            <Button type="submit" disabled={submitting} className="w-full">
              {submitting && <Loader2Icon className="animate-spin" />}
              开始安装
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
