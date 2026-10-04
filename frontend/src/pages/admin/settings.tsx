import { useEffect, useState, type FormEvent } from "react"
import { Loader2Icon, SaveIcon } from "lucide-react"
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
import { Textarea } from "@/components/ui/textarea"
import { api, errorMessage } from "@/lib/api"
import { loadSiteSettings } from "@/lib/site"
import type { CommentModeration, SiteSettingsAdmin } from "@/lib/types"

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
  const [ogImage, setOgImage] = useState("")
  // 反滥用（契约「反滥用」条款）：仅后台可见，公开接口不返回
  const [blockedKeywords, setBlockedKeywords] = useState("")
  const [maxLinks, setMaxLinks] = useState("3")
  // 评论审核方式（契约「评论审核方式」条款）：post=先发后审（默认）/ pre=先审后发
  const [commentModeration, setCommentModeration] = useState<CommentModeration>("post")

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
        setOgImage(s.og_image)
        setBlockedKeywords(s.comment_blocked_keywords)
        setMaxLinks(String(s.comment_max_links))
        setCommentModeration(s.comment_moderation)
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
    if (
      ogImage.trim() &&
      !ogImage.trim().startsWith("/api/uploads/") &&
      !ogImage.trim().startsWith("http://") &&
      !ogImage.trim().startsWith("https://")
    ) {
      toast.error("OG 分享图必须以 /api/uploads/ 或 http://、https:// 开头")
      return
    }
    const maxLinksNum = Number(maxLinks)
    if (!Number.isInteger(maxLinksNum) || maxLinksNum < 0 || maxLinksNum > 100) {
      toast.error("评论链接数上限必须是 0~100 的整数（0=不限制）")
      return
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
        og_image: ogImage.trim(),
        comment_blocked_keywords: blockedKeywords,
        comment_max_links: maxLinksNum,
        comment_moderation: commentModeration,
      })
      // 回显服务端规范化后的值（trim / base_url 去尾斜杠）
      setTitle(saved.title)
      setSubtitle(saved.subtitle)
      setDescription(saved.description)
      setIcpNumber(saved.icp_number)
      setFooterText(saved.footer_text)
      setPerPage(String(saved.per_page))
      setBaseUrl(saved.base_url)
      setOgImage(saved.og_image)
      setBlockedKeywords(saved.comment_blocked_keywords)
      setMaxLinks(String(saved.comment_max_links))
      setCommentModeration(saved.comment_moderation)
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
            <div className="grid gap-2">
              <Label htmlFor="site-og-image">OG 分享图</Label>
              <Input
                id="site-og-image"
                value={ogImage}
                onChange={(e) => setOgImage(e.target.value)}
                placeholder="/api/uploads/… 或 https://…（选填）"
                maxLength={500}
              />
              <p className="text-xs text-muted-foreground">
                分享到社媒/聊天工具的卡片兜底图；留空则用文章正文第一张图，都没有时不输出
                og:image
              </p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">评论</CardTitle>
            <CardDescription>
              新评论/留言的审核方式；与下方反滥用规则相互独立，反滥用判定不受此设置影响
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="comment-moderation">评论审核方式</Label>
              <Select
                value={commentModeration}
                onValueChange={(v) => setCommentModeration(v as CommentModeration)}
              >
                <SelectTrigger id="comment-moderation" className="w-full max-w-md">
                  <SelectValue placeholder="请选择" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="post">
                    先发后审（评论立即公开，可在后台隐藏）
                  </SelectItem>
                  <SelectItem value="pre">先审后发（评论需后台通过后才公开）</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                切换只影响之后的新评论，不会改动已存在的评论
              </p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">反滥用</CardTitle>
            <CardDescription>
              评论与留言的内容校验规则；限流阈值（60 秒 / 10 分钟）为后端固定常量，此处的设置
              不会出现在任何公开接口
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="comment-blocked-keywords">评论关键词黑名单</Label>
              <Textarea
                id="comment-blocked-keywords"
                value={blockedKeywords}
                onChange={(e) => setBlockedKeywords(e.target.value)}
                placeholder={"每行一个关键词，或用逗号分隔（不区分大小写）\n如：加微信\n代开发票"}
                rows={4}
                maxLength={2000}
              />
              <p className="text-xs text-muted-foreground">
                命中任一关键词的评论/留言会被拒绝（提示不会指明命中的是哪个词）
              </p>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="comment-max-links">评论链接数上限</Label>
              <Input
                id="comment-max-links"
                type="number"
                min={0}
                max={100}
                step={1}
                value={maxLinks}
                onChange={(e) => setMaxLinks(e.target.value)}
                className="max-w-[10rem]"
              />
              <p className="text-xs text-muted-foreground">
                正文中 http:// 与 https:// 出现次数的上限（0~100；0 表示不限制，默认 3）
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
