import { useEffect, useState, type FormEvent } from "react"
import { Loader2Icon, SaveIcon } from "lucide-react"
import { toast } from "sonner"

import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { api, errorMessage } from "@/lib/api"
import { loadSiteSettings } from "@/lib/site"
import type { SiteSettingsAdmin } from "@/lib/types"

/** /admin/settings：站点管理（站点名称/副标题/描述/备案号/页脚文字/每页文章数/base_url） */
export default function AdminSettingsPage() {
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const [title, setTitle] = useState("")
  const [subtitle, setSubtitle] = useState("")
  const [description, setDescription] = useState("")
  const [icpNumber, setIcpNumber] = useState("")
  const [footerText, setFooterText] = useState("")
  const [perPage, setPerPage] = useState("10")
  const [baseUrl, setBaseUrl] = useState("")

  useEffect(() => {
    document.title = "站点管理 · reedblog"
    api.admin
      .siteSettings()
      .then((s: SiteSettingsAdmin) => {
        setTitle(s.title)
        setSubtitle(s.subtitle)
        setDescription(s.description)
        setIcpNumber(s.icp_number)
        setFooterText(s.footer_text)
        setPerPage(String(s.per_page))
        setBaseUrl(s.base_url)
      })
      .catch((e) => setLoadError(errorMessage(e)))
      .finally(() => setLoading(false))
  }, [])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    // 前置校验（与后端契约一致，提前拦截减少往返）
    if (!title.trim()) {
      toast.error("站点名称不能为空")
      return
    }
    const perPageNum = Number(perPage)
    if (!Number.isInteger(perPageNum) || perPageNum < 1 || perPageNum > 100) {
      toast.error("每页文章数必须是 1~100 的整数")
      return
    }
    if (baseUrl.trim()) {
      try {
        const u = new URL(baseUrl.trim())
        if (u.protocol !== "http:" && u.protocol !== "https:") throw new Error()
      } catch {
        toast.error("base_url 必须是合法的 http/https 绝对 URL")
        return
      }
    }

    setSaving(true)
    try {
      const saved = await api.admin.updateSiteSettings({
        title: title.trim(),
        subtitle,
        description,
        icp_number: icpNumber,
        footer_text: footerText,
        per_page: perPageNum,
        base_url: baseUrl.trim(),
      })
      // 回显服务端规范化后的值（trim / base_url 去尾斜杠）
      setTitle(saved.title)
      setSubtitle(saved.subtitle)
      setDescription(saved.description)
      setIcpNumber(saved.icp_number)
      setFooterText(saved.footer_text)
      setPerPage(String(saved.per_page))
      setBaseUrl(saved.base_url)
      // 刷新前台站点设置的模块级缓存（同标签页回前台立即生效）
      loadSiteSettings(true).catch(() => {})
      toast.success("站点设置已保存")
    } catch (err) {
      toast.error(`保存失败：${errorMessage(err)}`)
    } finally {
      setSaving(false)
    }
  }

  if (loading) return <BlockSpinner label="加载站点设置…" />
  if (loadError) {
    return (
      <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
        {loadError}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6">
      <h1 className="text-xl font-bold">站点管理</h1>

      <form onSubmit={submit} className="flex flex-col gap-6">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">基本信息</CardTitle>
            <CardDescription>站点名称、口号与 SEO 描述，前台头部与页面标题实时生效</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="site-title">站点名称 *</Label>
              <Input
                id="site-title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="如：晚风的博客"
                maxLength={255}
                required
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="site-subtitle">副标题 / 口号</Label>
              <Input
                id="site-subtitle"
                value={subtitle}
                onChange={(e) => setSubtitle(e.target.value)}
                placeholder="显示在站点名称下方（选填）"
                maxLength={255}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="site-description">站点描述（meta description）</Label>
              <Textarea
                id="site-description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="供搜索引擎展示的一句话站点简介（选填）"
                rows={3}
                maxLength={1000}
              />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">页脚</CardTitle>
            <CardDescription>备案号有值时在前台页脚显示并链接到工信部备案系统</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="site-icp">ICP 备案号</Label>
              <Input
                id="site-icp"
                value={icpNumber}
                onChange={(e) => setIcpNumber(e.target.value)}
                placeholder="如：京ICP备12345678号（选填）"
                maxLength={100}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="site-footer">页脚自定义文字</Label>
              <Textarea
                id="site-footer"
                value={footerText}
                onChange={(e) => setFooterText(e.target.value)}
                placeholder="显示在版权行下方，支持换行（选填）"
                rows={3}
                maxLength={1000}
              />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">阅读与链接</CardTitle>
            <CardDescription>前台文章列表分页与 RSS / sitemap 绝对链接</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="site-per-page">每页文章数 *</Label>
              <Input
                id="site-per-page"
                type="number"
                min={1}
                max={100}
                step={1}
                value={perPage}
                onChange={(e) => setPerPage(e.target.value)}
                className="max-w-[10rem]"
                required
              />
              <p className="text-xs text-muted-foreground">
                前台文章列表 / 搜索结果的默认每页条数（1~100）
              </p>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="site-base-url">站点 base_url 覆盖</Label>
              <Input
                id="site-base-url"
                type="url"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="如：https://blog.example.com（选填，留空按部署地址推导）"
                maxLength={500}
              />
              <p className="text-xs text-muted-foreground">
                RSS feed.xml 与 sitemap.xml 生成绝对链接时使用；留空则回退服务器配置 / 请求头推导
              </p>
            </div>
          </CardContent>
        </Card>

        <div>
          <Button type="submit" disabled={saving}>
            {saving ? <Loader2Icon className="size-4 animate-spin" /> : <SaveIcon className="size-4" />}
            保存设置
          </Button>
        </div>
      </form>
    </div>
  )
}
