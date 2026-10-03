import { useEffect, useRef, useState } from "react"
import { HeartIcon } from "lucide-react"

import { api } from "@/lib/api"
import { getLikeId } from "@/lib/like-id"
import { cn } from "@/lib/utils"

interface LikeButtonProps {
  slug: string
  /** 详情页响应里的点赞总数（PostDetail.likes）作为初始值 */
  initialLikes: number
}

/**
 * 文章点赞按钮（契约「浏览量与点赞」条款）：
 * - 挂载时调 GET /api/posts/:slug/like?liker_key= 查当前访客是否已赞（决定初始态）
 * - 点击乐观更新 + scale 弹跳动画，已赞态高亮，再点取消；
 *   liker_key 为 localStorage 匿名 id（lib/like-id），后端幂等去重
 * - 请求失败回滚本地状态（不 toast 打扰阅读）
 */
export function LikeButton({ slug, initialLikes }: LikeButtonProps) {
  const [likes, setLikes] = useState(initialLikes)
  const [liked, setLiked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [pulse, setPulse] = useState(false)
  const pulseTimer = useRef<number | null>(null)

  // 进详情页时查询当前访客点赞状态
  useEffect(() => {
    let cancelled = false
    api
      .likeStatus(slug, getLikeId())
      .then((r) => {
        if (cancelled) return
        setLiked(r.liked)
        setLikes(r.likes)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [slug])

  useEffect(
    () => () => {
      if (pulseTimer.current !== null) window.clearTimeout(pulseTimer.current)
    },
    [],
  )

  const toggle = async () => {
    if (busy) return
    const next = !liked
    // 乐观更新 + 弹跳动画（scale 一下）
    setBusy(true)
    setLiked(next)
    setLikes((n) => (next ? n + 1 : Math.max(0, n - 1)))
    setPulse(true)
    if (pulseTimer.current !== null) window.clearTimeout(pulseTimer.current)
    pulseTimer.current = window.setTimeout(() => setPulse(false), 300)
    try {
      const key = getLikeId()
      const r = next ? await api.likePost(slug, key) : await api.unlikePost(slug, key)
      setLikes(r.likes)
      setLiked(r.liked)
    } catch {
      // 失败回滚
      setLiked(!next)
      setLikes((n) => (next ? Math.max(0, n - 1) : n + 1))
    } finally {
      setBusy(false)
    }
  }

  return (
    <button
      type="button"
      onClick={() => void toggle()}
      disabled={busy}
      aria-pressed={liked}
      aria-label={liked ? "取消点赞" : "点赞"}
      className={cn(
        "inline-flex items-center gap-2 rounded-full border px-6 py-2.5 text-sm font-medium transition-colors",
        "disabled:pointer-events-none",
        liked
          ? "border-rose-300 bg-rose-50 text-rose-600"
          : "border-border bg-background text-muted-foreground hover:border-rose-300 hover:text-rose-500",
      )}
    >
      <HeartIcon
        className={cn(
          "size-4.5 transition-transform duration-200 ease-out",
          liked && "fill-current",
          pulse && "scale-125",
        )}
      />
      <span className="tabular-nums">{likes}</span>
      <span>{liked ? "已赞" : "点赞"}</span>
    </button>
  )
}
