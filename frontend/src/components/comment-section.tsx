import { useEffect, useState, type FormEvent } from "react"
import { Loader2Icon, SendIcon } from "lucide-react"
import { toast } from "sonner"

import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { api, errorMessage } from "@/lib/api"
import { formatRelative } from "@/lib/utils"
import type { CommentPub } from "@/lib/types"

const AUTHOR_KEY = "reedblog_comment_author"
const EMAIL_KEY = "reedblog_comment_email"

/** 文章详情页评论区：展示 + 发表表单 */
export function CommentSection({ slug }: { slug: string }) {
  const [comments, setComments] = useState<CommentPub[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    setComments(null)
    setError(null)
    api
      .comments(slug)
      .then((c) => {
        if (!cancelled) setComments(c)
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e))
      })
    return () => {
      cancelled = true
    }
  }, [slug])

  return (
    <section className="flex flex-col gap-6">
      <h2 className="text-lg font-semibold">
        评论{comments ? ` (${comments.length})` : ""}
      </h2>
      {error ? (
        <p className="text-sm text-destructive">{error}</p>
      ) : comments === null ? (
        <BlockSpinner label="加载评论…" />
      ) : comments.length === 0 ? (
        <p className="text-sm text-muted-foreground">还没有评论，来发表第一条吧！</p>
      ) : (
        <ul className="flex flex-col gap-5">
          {comments.map((c) => (
            <li key={c.id} className="flex gap-3">
              <div className="flex size-9 shrink-0 items-center justify-center rounded-full bg-secondary text-sm font-medium">
                {c.author_name.slice(0, 1).toUpperCase()}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-baseline gap-x-2 text-sm">
                  <span className="font-medium">{c.author_name}</span>
                  <span className="text-xs text-muted-foreground">
                    {formatRelative(c.created_at)}
                  </span>
                </div>
                <p className="mt-1 whitespace-pre-wrap break-words text-sm text-foreground/90">
                  {c.content}
                </p>
              </div>
            </li>
          ))}
        </ul>
      )}
      <CommentForm slug={slug} onCreated={(c) => setComments((prev) => [...(prev ?? []), c])} />
    </section>
  )
}

function CommentForm({ slug, onCreated }: { slug: string; onCreated: (c: CommentPub) => void }) {
  const [authorName, setAuthorName] = useState(() => localStorage.getItem(AUTHOR_KEY) ?? "")
  const [email, setEmail] = useState(() => localStorage.getItem(EMAIL_KEY) ?? "")
  const [content, setContent] = useState("")
  const [submitting, setSubmitting] = useState(false)

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (!authorName.trim()) {
      toast.error("请填写昵称")
      return
    }
    if (!content.trim()) {
      toast.error("评论内容不能为空")
      return
    }
    setSubmitting(true)
    try {
      const created = await api.createComment(slug, {
        author_name: authorName.trim(),
        ...(email.trim() ? { email: email.trim() } : {}),
        content: content.trim(),
      })
      localStorage.setItem(AUTHOR_KEY, authorName.trim())
      localStorage.setItem(EMAIL_KEY, email.trim())
      setContent("")
      onCreated(created)
      toast.success("评论发表成功")
    } catch (err) {
      toast.error(errorMessage(err))
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="flex flex-col gap-4 rounded-lg border bg-card p-4 sm:p-5"
    >
      <h3 className="text-sm font-medium">发表评论</h3>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="grid gap-1.5">
          <Label htmlFor="comment-author">昵称 *</Label>
          <Input
            id="comment-author"
            value={authorName}
            onChange={(e) => setAuthorName(e.target.value)}
            placeholder="你的名字"
            maxLength={50}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="comment-email">邮箱（选填）</Label>
          <Input
            id="comment-email"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="不会公开显示"
          />
        </div>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="comment-content">内容 *</Label>
        <Textarea
          id="comment-content"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          placeholder="说点什么吧…"
          className="min-h-24"
          maxLength={2000}
        />
      </div>
      <div>
        <Button type="submit" disabled={submitting}>
          {submitting ? <Loader2Icon className="animate-spin" /> : <SendIcon />}
          发表评论
        </Button>
      </div>
    </form>
  )
}
