import { Badge } from "@/components/ui/badge"
import type { PostStatus } from "@/lib/types"

export function PostStatusBadge({ status }: { status: PostStatus }) {
  if (status === "published") {
    return (
      <Badge className="border-emerald-200 bg-emerald-50 text-emerald-700">已发布</Badge>
    )
  }
  if (status === "scheduled") {
    // 定时发布（契约「文章置顶与定时发布」条款）：到点自动公开可见
    return (
      <Badge className="border-sky-200 bg-sky-50 text-sky-700">定时发布</Badge>
    )
  }
  return (
    <Badge variant="outline" className="border-amber-200 bg-amber-50 text-amber-700">
      草稿
    </Badge>
  )
}
