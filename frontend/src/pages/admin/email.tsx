import { useEffect, useState, type FormEvent } from "react"
import { Loader2Icon, MailIcon, SaveIcon, SendIcon } from "lucide-react"
import { toast } from "sonner"

import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { api, errorMessage } from "@/lib/api"
import type { SmtpLastResult, SmtpSettingsAdmin, SmtpTls } from "@/lib/types"

const TLS_OPTIONS: { value: SmtpTls; label: string; hint: string }[] = [
  { value: "starttls", label: "STARTTLS（587 等，推荐）", hint: "明文连接后升级 TLS" },
  { value: "implicit", label: "隐式 TLS / SMTPS（465）", hint: "连接即 TLS" },
  { value: "none", label: "不加密（仅内网 / 本地调试）", hint: "明文传输" },
]

/** 基本邮箱格式（与后端契约「邮件通知」校验同口径） */
function isValidEmail(v: string): boolean {
  if (/\s/.test(v)) return false
  const at = v.indexOf("@")
  if (at <= 0 || at !== v.lastIndexOf("@")) return false
  const local = v.slice(0, at)
  const domain = v.slice(at + 1)
  return (
    !local.startsWith(".") &&
    !local.endsWith(".") &&
    domain.includes(".") &&
    !domain.startsWith(".") &&
    !domain.endsWith(".") &&
    !domain.includes("..")
  )
}

function formatTime(at: string): string {
  const d = new Date(at)
  return Number.isNaN(d.getTime()) ? at : d.toLocaleString()
}

/** /admin/email：邮件通知（SMTP 配置 + 发送测试邮件；密码只在 config.toml / 环境变量配置） */
export default function AdminEmailPage() {
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)

  const [enabled, setEnabled] = useState(false)
  const [host, setHost] = useState("")
  const [port, setPort] = useState("587")
  const [username, setUsername] = useState("")
  const [fromName, setFromName] = useState("")
  const [fromEmail, setFromEmail] = useState("")
  const [toEmail, setToEmail] = useState("")
  const [tls, setTls] = useState<SmtpTls>("starttls")
  const [hasPassword, setHasPassword] = useState(false)
  const [lastResult, setLastResult] = useState<SmtpLastResult | null>(null)

  const applySettings = (s: SmtpSettingsAdmin) => {
    setEnabled(s.enabled)
    setHost(s.host)
    setPort(String(s.port))
    setUsername(s.username)
    setFromName(s.from_name)
    setFromEmail(s.from_email)
    setToEmail(s.to_email)
    setTls(s.tls)
    setHasPassword(s.has_password)
    setLastResult(s.last_result)
  }

  useEffect(() => {
    document.title = "邮件通知 · reedblog"
    api.admin
      .smtpSettings()
      .then(applySettings)
      .catch((e) => setLoadError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [])

  /** 前端本地校验（与契约校验条款同口径，减少往返；后端仍是权威） */
  const validate = (): string | null => {
    const portNum = Number(port)
    if (!Number.isInteger(portNum) || portNum < 1 || portNum > 65535) {
      return "端口必须是 1~65535 的整数"
    }
    if (fromEmail.trim() && !isValidEmail(fromEmail.trim())) return "发件邮箱不是合法的邮箱地址"
    if (toEmail.trim() && !isValidEmail(toEmail.trim())) return "收件邮箱不是合法的邮箱地址"
    if (host.trim() && /[\s/:]/.test(host.trim())) return "SMTP 主机不能包含空白、'/' 或 ':'"
    if (enabled) {
      if (!host.trim()) return "启用邮件通知时 SMTP 主机不能为空"
      if (!fromEmail.trim()) return "启用邮件通知时发件邮箱不能为空"
      if (!toEmail.trim()) return "启用邮件通知时收件邮箱不能为空"
    }
    return null
  }

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const invalid = validate()
    if (invalid) {
      toast.error(invalid)
      return
    }
    setSaving(true)
    try {
      const saved = await api.admin.updateSmtpSettings({
        enabled,
        host: host.trim(),
        port: Number(port),
        username: username.trim(),
        from_name: fromName.trim(),
        from_email: fromEmail.trim(),
        to_email: toEmail.trim(),
        tls,
      })
      applySettings(saved)
      toast.success("邮件通知设置已保存")
    } catch (err) {
      toast.error(`保存失败：${errorMessage(err)}`)
    } finally {
      setSaving(false)
    }
  }

  const sendTest = async () => {
    const invalid = validate()
    if (invalid) {
      toast.error(invalid)
      return
    }
    setTesting(true)
    try {
      await api.admin.testSmtp()
      toast.success("测试邮件已发送，请到收件箱查收")
    } catch (err) {
      // 后端给出明确失败原因（配置不完整 422 / 发送失败 502 smtp_send_failed）
      toast.error(`测试邮件发送失败：${errorMessage(err)}`)
    } finally {
      // 无论成败都刷新最近一次发送结果
      api.admin.smtpSettings().then(applySettings).catch(() => {})
      setTesting(false)
    }
  }

  if (loading) return <BlockSpinner label="加载邮件通知设置…" />
  if (loadError) {
    return (
      <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
        {loadError}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      <h1 className="text-xl font-bold">邮件通知</h1>

      <form onSubmit={submit} className="flex flex-col gap-6">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">通知开关</CardTitle>
            <CardDescription>
              开启后，新评论 / 新回复创建成功时向收件邮箱发送一封通知邮件
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="flex items-center justify-between gap-4 rounded-md border border-border bg-muted/30 px-4 py-3">
              <div className="flex items-center gap-3">
                <MailIcon className="size-4 text-muted-foreground" />
                <div>
                  <p className="text-sm font-medium">启用邮件通知</p>
                  <p className="text-xs text-muted-foreground">
                    发送异步进行，失败只记日志，不影响评论发表
                  </p>
                </div>
              </div>
              <Switch checked={enabled} onCheckedChange={setEnabled} aria-label="启用邮件通知" />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">SMTP 服务器</CardTitle>
            <CardDescription>发信服务器的地址、端口与加密方式</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-4 sm:grid-cols-[1fr_10rem]">
              <div className="grid gap-2">
                <Label htmlFor="smtp-host">SMTP 主机</Label>
                <Input
                  id="smtp-host"
                  value={host}
                  onChange={(e) => setHost(e.target.value)}
                  placeholder="如：smtp.example.com"
                  maxLength={255}
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="smtp-port">端口</Label>
                <Input
                  id="smtp-port"
                  type="number"
                  min={1}
                  max={65535}
                  step={1}
                  value={port}
                  onChange={(e) => setPort(e.target.value)}
                />
              </div>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="smtp-tls">加密方式</Label>
              <Select value={tls} onValueChange={(v) => setTls(v as SmtpTls)}>
                <SelectTrigger id="smtp-tls" className="w-full max-w-md">
                  <SelectValue placeholder="请选择" />
                </SelectTrigger>
                <SelectContent>
                  {TLS_OPTIONS.map((o) => (
                    <SelectItem key={o.value} value={o.value}>
                      {o.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {TLS_OPTIONS.find((o) => o.value === tls)?.hint}
              </p>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="smtp-username">用户名</Label>
              <Input
                id="smtp-username"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="SMTP 登录用户名（免认证中继可留空）"
                maxLength={255}
                autoComplete="off"
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="smtp-password">密码</Label>
              <Input
                id="smtp-password"
                value={hasPassword ? "已配置（来自 config.toml / 环境变量）" : "未设置"}
                readOnly
                disabled
                aria-label="SMTP 密码状态"
              />
              <p className="text-xs text-muted-foreground">
                出于安全考虑，密码不经过数据库与任何接口：请在 <code>config.toml</code> 的{" "}
                <code>[smtp] password</code> 或环境变量{" "}
                <code>REEDBLOG_SMTP_PASSWORD</code> 中配置（环境变量优先），修改后无需重启。
              </p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">收发件人</CardTitle>
            <CardDescription>通知邮件的发件人与收件地址</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="grid gap-2">
                <Label htmlFor="smtp-from-name">发件人名</Label>
                <Input
                  id="smtp-from-name"
                  value={fromName}
                  onChange={(e) => setFromName(e.target.value)}
                  placeholder="如：我的博客"
                  maxLength={100}
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="smtp-from-email">发件邮箱</Label>
                <Input
                  id="smtp-from-email"
                  type="email"
                  value={fromEmail}
                  onChange={(e) => setFromEmail(e.target.value)}
                  placeholder="如：blog@example.com"
                  maxLength={255}
                />
              </div>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="smtp-to-email">收件邮箱</Label>
              <Input
                id="smtp-to-email"
                type="email"
                value={toEmail}
                onChange={(e) => setToEmail(e.target.value)}
                placeholder="接收评论通知的邮箱，如：admin@example.com"
                maxLength={255}
              />
            </div>
          </CardContent>
        </Card>

        {lastResult && (
          <div
            className={
              lastResult.ok
                ? "rounded-lg border border-border bg-muted/30 p-4 text-sm"
                : "rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive"
            }
          >
            <p className="font-medium">
              最近一次发送：{lastResult.ok ? "成功" : "失败"} · {formatTime(lastResult.at)}
            </p>
            <p className="mt-1 break-all opacity-80">{lastResult.message}</p>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" disabled={saving || testing}>
            {saving ? (
              <Loader2Icon className="size-4 animate-spin" />
            ) : (
              <SaveIcon className="size-4" />
            )}
            保存设置
          </Button>
          <Button type="button" variant="secondary" onClick={sendTest} disabled={saving || testing}>
            {testing ? (
              <Loader2Icon className="size-4 animate-spin" />
            ) : (
              <SendIcon className="size-4" />
            )}
            发送测试邮件
          </Button>
          <p className="text-xs text-muted-foreground">
            测试发送会同步等待 SMTP 结果（约 10 秒超时），不需要先开启开关
          </p>
        </div>
      </form>
    </div>
  )
}
