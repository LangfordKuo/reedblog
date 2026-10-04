import { useEffect, useState, type FormEvent } from "react"
import { CalendarIcon, KeyRoundIcon, Loader2Icon, SaveIcon, ShieldCheckIcon, UserIcon } from "lucide-react"
import { toast } from "sonner"

import { BlockSpinner } from "@/components/spinner"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Separator } from "@/components/ui/separator"
import { api, errorMessage } from "@/lib/api"
import { setStoredUsername } from "@/lib/auth"
import type { ProfileAdmin } from "@/lib/types"
import { formatDateTime } from "@/lib/utils"

/** 用户名长度上限：与安装向导表单一致（后端校验同安装路径：trim 后非空） */
const USERNAME_MAX = 50

/**
 * /admin/profile：用户设置（改用户名 / 改密码，契约「管理员资料 / 用户设置」条款）：
 * - 只读区展示当前用户名与创建时间
 * - 修改必须提供当前密码；用户名/新密码至少改一项
 * - **改完不会让已签发的 JWT 失效**：其它已登录设备不会被强制退出（页面内有明确提示）
 */
export default function AdminProfilePage() {
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [profile, setProfile] = useState<ProfileAdmin | null>(null)

  const [username, setUsername] = useState("")
  const [currentPassword, setCurrentPassword] = useState("")
  const [newPassword, setNewPassword] = useState("")
  const [confirmPassword, setConfirmPassword] = useState("")

  const applyProfile = (p: ProfileAdmin) => {
    setProfile(p)
    setUsername(p.username)
  }

  useEffect(() => {
    document.title = "用户设置 · reedblog"
    api.admin
      .profile()
      .then(applyProfile)
      .catch((e) => setLoadError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (saving || !profile) return

    const nextUsername = username.trim()
    if (!nextUsername) {
      toast.error("用户名不能为空")
      return
    }
    if (!currentPassword) {
      toast.error("请输入当前密码以确认修改")
      return
    }
    if (newPassword !== confirmPassword) {
      toast.error("两次输入的新密码不一致")
      return
    }
    const usernameChanged = nextUsername !== profile.username
    if (!usernameChanged && !newPassword) {
      toast.error("用户名与新密码均为空，没有需要修改的内容")
      return
    }

    setSaving(true)
    try {
      const updated = await api.admin.updateProfile({
        current_password: currentPassword,
        ...(usernameChanged ? { username: nextUsername } : {}),
        ...(newPassword ? { new_password: newPassword } : {}),
      })
      applyProfile(updated)
      // 后台侧栏/顶栏显示的缓存用户名立即跟随（token 不动）
      setStoredUsername(updated.username)
      setCurrentPassword("")
      setNewPassword("")
      setConfirmPassword("")
      const what = [usernameChanged && "用户名", newPassword && "密码"].filter(Boolean).join("与")
      toast.success(`${what}已更新；其它已登录设备不会被强制退出`)
    } catch (err) {
      toast.error(
        errorMessage(err, {
          invalid_credentials: "当前密码错误",
          username_taken: "该用户名已被占用，请换一个",
          validation_error: "内容校验失败，请检查填写项",
        }),
      )
    } finally {
      setSaving(false)
    }
  }

  if (loading) return <BlockSpinner label="加载用户资料…" />
  if (loadError || !profile) {
    return (
      <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
        {loadError || "加载用户资料失败"}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-bold">用户设置</h1>
        <p className="text-sm text-muted-foreground">修改登录用户名与密码（单管理员）</p>
      </div>

      {/* 只读区：当前用户名 / 创建时间 */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <UserIcon className="size-4 text-muted-foreground" />
            当前账号
          </CardTitle>
        </CardHeader>
        <CardContent className="grid gap-2 text-sm">
          <div className="flex items-center gap-2">
            <span className="text-muted-foreground">用户名：</span>
            <span className="font-medium">{profile.username}</span>
          </div>
          <div className="flex items-center gap-2">
            <CalendarIcon className="size-4 text-muted-foreground" />
            <span className="text-muted-foreground">创建时间：</span>
            <span>{formatDateTime(profile.created_at)}</span>
          </div>
        </CardContent>
      </Card>

      {/* 修改表单 */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <KeyRoundIcon className="size-4 text-muted-foreground" />
            修改用户名 / 密码
          </CardTitle>
          <CardDescription>
            用户名与密码可只改其一；任何修改都需要输入当前密码确认
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form className="grid gap-4" onSubmit={(e) => void submit(e)}>
            <div className="grid gap-1.5 sm:max-w-md">
              <Label htmlFor="profile-username">用户名</Label>
              <Input
                id="profile-username"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete="username"
                maxLength={USERNAME_MAX}
              />
              <p className="text-xs text-muted-foreground">
                登录后台使用的用户名；改动后旧用户名立即失效
              </p>
            </div>

            <Separator />

            <div className="grid gap-4 sm:max-w-md">
              <div className="grid gap-1.5">
                <Label htmlFor="profile-current">当前密码 *</Label>
                <Input
                  id="profile-current"
                  type="password"
                  value={currentPassword}
                  onChange={(e) => setCurrentPassword(e.target.value)}
                  autoComplete="current-password"
                  placeholder="确认修改需要验证当前密码"
                />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="profile-new">新密码</Label>
                <Input
                  id="profile-new"
                  type="password"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  autoComplete="new-password"
                  placeholder="留空表示不修改密码"
                />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="profile-confirm">确认新密码</Label>
                <Input
                  id="profile-confirm"
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  autoComplete="new-password"
                  placeholder="与新密码保持一致"
                />
              </div>
            </div>

            {/* 契约明示：改用户名/密码不使已签发 JWT 失效 */}
            <Alert>
              <ShieldCheckIcon />
              <AlertDescription>
                {/* 单个 <span>：AlertDescription 是 grid 容器，行内元素会被拆成多行 */}
                <span>
                  修改用户名或密码<strong>不会</strong>让已签发的登录凭证失效——其它已登录设备
                  <strong>不会被强制退出</strong>
                  （其页面上的用户名显示可能滞后，刷新后即更新）。如需强制全部设备下线，请修改{" "}
                  <code>config.toml</code> 的 <code>jwt_secret</code> 并重启服务。
                </span>
              </AlertDescription>
            </Alert>

            <div className="flex justify-end">
              <Button type="submit" disabled={saving}>
                {saving ? <Loader2Icon className="animate-spin" /> : <SaveIcon />}
                保存修改
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
