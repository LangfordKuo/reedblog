import { useEffect, useMemo, useState, type FormEvent } from "react"
import { CornerDownRightIcon, Loader2Icon, MessageSquareIcon, SendIcon } from "lucide-react"
import { toast } from "sonner"

import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { api, ApiError, errorMessage } from "@/lib/api"
import { formatRelative } from "@/lib/utils"
import type { CommentPub } from "@/lib/types"

const AUTHOR_KEY = "reedblog_comment_author"
const EMAIL_KEY = "reedblog_comment_email"

/** 一个楼层：顶级评论 + 其下全部回复（契约「评论回复」：存储永远两级） */
type Thread = { top: CommentPub; replies: CommentPub[] }

/**
 * 平铺数组 → 两级树（契约「评论回复」条款）：
 * 后端保证 parent_id 恒指顶级楼层 id，且列表按 created_at ASC, id ASC（父先于子）。
 * 防御：parent 不在列表中的孤儿行按独立楼层展示（正常不发生）。
 */
function buildThreads(list: CommentPub[]): Thread[] {
  const threads: Thread[] = []
  const byTop = new Map<number, Thread>()
  for (const c of list) {
    if (c.parent_id === null) {
      const t: Thread = { top: c, replies: [] }
      threads.push(t)
      byTop.set(c.id, t)
    }
  }
  for (const c of list) {
    if (c.parent_id === null) continue
    const t = byTop.get(c.parent_id)
    if (t) t.replies.push(c)
    else threads.push({ top: c, replies: [] })
  }
  return threads
}

/**
 * 评论/留言区：两级楼中楼展示 + 发表表单 + 内联回复表单。
 * target="post"（默认）挂文章评论接口；target="page" 挂留言板页接口
 * （契约「页面」「评论回复」条款：留言 = 挂在页面上的评论，同一套回复机制）。
 *
 * 昵称/邮箱状态提升到本组件：内联回复表单沿用主表单已填值（仅填内容）。
 */
export function CommentSection({
  slug,
  target = "post",
}: {
  slug: string
  target?: "post" | "page"
}) {
  const [comments, setComments] = useState<CommentPub[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [authorName, setAuthorName] = useState(() => localStorage.getItem(AUTHOR_KEY) ?? "")
  const [email, setEmail] = useState(() => localStorage.getItem(EMAIL_KEY) ?? "")
  // 当前展开内联回复表单的目标评论（一次只开一个）
  const [replyTo, setReplyTo] = useState<CommentPub | null>(null)
  const noun = target === "page" ? "留言" : "评论"

  useEffect(() => {
    let cancelled = false
    setComments(null)
    setError(null)
    const load = target === "page" ? api.pageComments(slug) : api.comments(slug)
    load
      .then((c) => {
        if (!cancelled) setComments(c)
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e))
      })
    return () => {
      cancelled = true
    }
  }, [slug, target])

  const threads = useMemo(() => buildThreads(comments ?? []), [comments])

  /**
   * 提交评论/回复（主表单与内联回复表单共用）。
   * parentId 传被点击评论的 id 即可——「回复的回复」由后端归一化到同一顶级楼层，
   * 前端按 parent_id 分组后自动落回原楼层。成功返回 true（供回复表单关闭）。
   * website 为蜜罐字段（契约「反滥用」）：真人为空，机器人自动填充 → 后端 201 假成功；
   * 429/403 的提示文案固定为友好话术，不回显后端规则细节。
   */
  const submitComment = async (
    content: string,
    parentId?: number,
    website?: string,
  ): Promise<boolean> => {
    const isReply = parentId !== undefined
    if (!authorName.trim()) {
      toast.error(isReply ? "请先在下方发表表单填写昵称" : "请填写昵称")
      return false
    }
    try {
      const create = target === "page" ? api.createPageComment : api.createComment
      const created = await create(slug, {
        author_name: authorName.trim(),
        ...(email.trim() ? { email: email.trim() } : {}),
        content: content.trim(),
        ...(isReply ? { parent_id: parentId } : {}),
        ...(website ? { website } : {}),
      })
      localStorage.setItem(AUTHOR_KEY, authorName.trim())
      localStorage.setItem(EMAIL_KEY, email.trim())
      setComments((prev) => [...(prev ?? []), created])
      setReplyTo(null)
      toast.success(isReply ? `回复发表成功` : `${noun}发表成功`)
      return true
    } catch (err) {
      if (err instanceof ApiError && err.code === "too_many_requests") {
        // 契约「反滥用」：Retry-After 剩余秒数可用于提示；不暴露阈值规则
        toast.error(
          err.retryAfter
            ? `提交过于频繁，请 ${err.retryAfter} 秒后再试`
            : "提交过于频繁，请稍后再试",
        )
        return false
      }
      if (err instanceof ApiError && err.code === "comment_rejected") {
        // 契约「反滥用」：不透露命中的关键词或规则类型
        toast.error("内容未通过校验，请修改后重试")
        return false
      }
      toast.error(errorMessage(err))
      return false
    }
  }

  return (
    <section className="flex flex-col gap-6">
      {/* 计数口径不变：线程内所有 visible 评论（平铺数组长度）都计数 */}
      <h2 className="text-lg font-semibold">
        {noun}
        {comments ? ` (${comments.length})` : ""}
      </h2>
      {error ? (
        <p className="text-sm text-destructive">{error}</p>
      ) : comments === null ? (
        <BlockSpinner label={`加载${noun}…`} />
      ) : comments.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          还没有{noun}，来发表第一条吧！
        </p>
      ) : (
        <ul className="flex flex-col gap-5">
          {threads.map(({ top, replies }) => (
            <li key={top.id} className="flex flex-col gap-4">
              <CommentItem
                c={top}
                replyOpen={replyTo?.id === top.id}
                onReply={() => setReplyTo(replyTo?.id === top.id ? null : top)}
                onSubmit={submitComment}
              />
              {replies.length > 0 && (
                <ul className="ml-4 flex flex-col gap-4 border-l pl-4 sm:ml-12">
                  {replies.map((r) => (
                    <li key={r.id}>
                      <CommentItem
                        c={r}
                        replyOpen={replyTo?.id === r.id}
                        onReply={() => setReplyTo(replyTo?.id === r.id ? null : r)}
                        onSubmit={submitComment}
                      />
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}
      <CommentForm
        noun={noun}
        authorName={authorName}
        email={email}
        onAuthorNameChange={setAuthorName}
        onEmailChange={setEmail}
        onSubmit={(content, website) => submitComment(content, undefined, website)}
      />
    </section>
  )
}

/** 单条评论：头像/作者/时间/「回复 @xxx」标记/内容/回复按钮 + 内联回复表单 */
function CommentItem({
  c,
  replyOpen,
  onReply,
  onSubmit,
}: {
  c: CommentPub
  replyOpen: boolean
  onReply: () => void
  onSubmit: (content: string, parentId: number | undefined, website: string) => Promise<boolean>
}) {
  return (
    <div className="flex gap-3">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-full bg-secondary text-sm font-medium">
        {c.author_name.slice(0, 1).toUpperCase()}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2 text-sm">
          <span className="font-medium">{c.author_name}</span>
          {/* 被回复人标记（契约：reply_to_id 非空时 reply_to_name 为其作者名） */}
          {c.reply_to_id !== null && c.reply_to_name && (
            <span className="inline-flex items-center gap-0.5 text-xs text-muted-foreground">
              <CornerDownRightIcon className="size-3" />
              回复 <span className="font-medium text-foreground/80">@{c.reply_to_name}</span>
            </span>
          )}
          <span className="text-xs text-muted-foreground">
            {formatRelative(c.created_at)}
          </span>
        </div>
        <p className="mt-1 whitespace-pre-wrap break-words text-sm text-foreground/90">
          {c.content}
        </p>
        <button
          type="button"
          onClick={onReply}
          className="mt-1 inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          <MessageSquareIcon className="size-3" />
          回复
        </button>
        {replyOpen && (
          <ReplyForm
            replyToName={c.author_name}
            onSubmit={(content, website) => onSubmit(content, c.id, website)}
            onCancel={onReply}
          />
        )}
      </div>
    </div>
  )
}

/**
 * 蜜罐输入（契约「反滥用」条款）：对真人不可见、不可聚焦，机器人自动填充后会被
 * 后端识别（201 假成功但不落库）。用绝对定位移出视口 + opacity:0——**不用
 * display:none**（太容易被简单识别）；tabIndex={-1} 移出 Tab 序列、aria-hidden
 * 对读屏器隐藏，且不配 <label>（真人不该感知到它的存在）。
 */
function HoneypotField({
  value,
  onChange,
}: {
  value: string
  onChange: (v: string) => void
}) {
  return (
    <div
      className="absolute -left-[9999px] top-0 h-px w-px overflow-hidden opacity-0"
      aria-hidden="true"
    >
      <input
        type="text"
        name="website"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        tabIndex={-1}
        autoComplete="off"
        aria-hidden="true"
        className="opacity-0"
      />
    </div>
  )
}

/** 内联回复小表单：昵称/邮箱沿用主表单已填值，仅填内容；提交带 parent_id（后端归一化） */
function ReplyForm({
  replyToName,
  onSubmit,
  onCancel,
}: {
  replyToName: string
  onSubmit: (content: string, website: string) => Promise<boolean>
  onCancel: () => void
}) {
  const [content, setContent] = useState("")
  const [website, setWebsite] = useState("")
  const [submitting, setSubmitting] = useState(false)

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (!content.trim()) {
      toast.error("回复内容不能为空")
      return
    }
    setSubmitting(true)
    try {
      await onSubmit(content, website)
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="relative mt-2 flex flex-col gap-3 rounded-lg border bg-muted/30 p-3"
    >
      <HoneypotField value={website} onChange={setWebsite} />
      <Textarea
        value={content}
        onChange={(e) => setContent(e.target.value)}
        placeholder={`回复 @${replyToName}…`}
        className="min-h-20 bg-background"
        maxLength={2000}
        autoFocus
      />
      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" disabled={submitting}>
          {submitting ? <Loader2Icon className="animate-spin" /> : <SendIcon />}
          发表回复
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          取消
        </Button>
      </div>
    </form>
  )
}

/** 主表单：昵称/邮箱状态由 CommentSection 持有（内联回复表单沿用同一份值） */
function CommentForm({
  noun,
  authorName,
  email,
  onAuthorNameChange,
  onEmailChange,
  onSubmit,
}: {
  noun: string
  authorName: string
  email: string
  onAuthorNameChange: (v: string) => void
  onEmailChange: (v: string) => void
  onSubmit: (content: string, website: string) => Promise<boolean>
}) {
  const [content, setContent] = useState("")
  const [website, setWebsite] = useState("")
  const [submitting, setSubmitting] = useState(false)

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (!content.trim()) {
      toast.error(`${noun}内容不能为空`)
      return
    }
    setSubmitting(true)
    try {
      if (await onSubmit(content, website)) setContent("")
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="relative flex flex-col gap-4 rounded-lg border bg-card p-4 sm:p-5"
    >
      <HoneypotField value={website} onChange={setWebsite} />
      <h3 className="text-sm font-medium">发表{noun}</h3>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="grid gap-1.5">
          <Label htmlFor="comment-author">昵称 *</Label>
          <Input
            id="comment-author"
            value={authorName}
            onChange={(e) => onAuthorNameChange(e.target.value)}
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
            onChange={(e) => onEmailChange(e.target.value)}
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
          发表{noun}
        </Button>
      </div>
    </form>
  )
}
