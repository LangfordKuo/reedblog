import { useState, type FormEvent } from "react"
import { Link, Navigate, useLocation, useNavigate } from "react-router-dom"
import { CheckCircle2Icon, Loader2Icon, LogInIcon } from "lucide-react"
import { toast } from "sonner"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api, ApiError, errorMessage } from "@/lib/api"
import { getToken, setSession } from "@/lib/auth"

/** 剩余秒数 → 友好文案（<60 秒显示秒，否则向上取整到分钟） */
function formatRemaining(secs: number): string {
  return secs >= 60 ? `${Math.ceil(secs / 60)} 分钟` : `${secs} 秒`
}

export default function AdminLoginPage() {
  const navigate = useNavigate()
  const location = useLocation()
  const from =
    (location.state as { from?: { pathname?: string } } | null)?.from?.pathname ?? "/admin"
  const justInstalled = Boolean((location.state as { installed?: boolean } | null)?.installed)

  const [username, setUsername] = useState("")
  const [password, setPassword] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  if (getToken()) return <Navigate to="/admin" replace />

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (submitting) return
    setError(null)
    setSubmitting(true)
    try {
      const result = await api.login(username.trim(), password)
      setSession(result)
      toast.success(`欢迎回来，${result.username}`)
      navigate(from, { replace: true })
    } catch (err) {
      if (err instanceof ApiError && err.code === "too_many_attempts") {
        // 契约「反滥用」：失败退避锁定，提示剩余时间（Retry-After），不泄露阈值细节
        setError(
          err.retryAfter
            ? `登录失败次数过多，请 ${formatRemaining(err.retryAfter)}后重试`
            : "尝试次数过多，请稍后再试",
        )
      } else {
        setError(errorMessage(err, { invalid_credentials: "用户名或密码错误" }))
      }
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-muted/40 p-4">
      <Card className="w-full max-w-sm">
        <CardHeader className="items-center text-center">
          <CardTitle className="text-2xl tracking-tight">reedblog</CardTitle>
          <CardDescription>管理后台登录</CardDescription>
        </CardHeader>
        <CardContent>
          {justInstalled && (
            <Alert className="mb-4 border-emerald-200 bg-emerald-50 text-emerald-800">
              <CheckCircle2Icon />
              <AlertDescription>站点安装完成，请使用刚才设置的管理员账号登录。</AlertDescription>
            </Alert>
          )}
          <form onSubmit={handleSubmit} className="grid gap-4">
            <div className="grid gap-1.5">
              <Label htmlFor="login-username">用户名</Label>
              <Input
                id="login-username"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete="username"
                autoFocus
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="login-password">密码</Label>
              <Input
                id="login-password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="current-password"
              />
            </div>
            {error && (
              <Alert variant="destructive">
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            )}
            <Button type="submit" disabled={submitting} className="w-full">
              {submitting ? <Loader2Icon className="animate-spin" /> : <LogInIcon />}
              登录
            </Button>
            <p className="text-center text-xs text-muted-foreground">
              <Link to="/" className="underline-offset-4 hover:underline">
                ← 返回站点首页
              </Link>
            </p>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
