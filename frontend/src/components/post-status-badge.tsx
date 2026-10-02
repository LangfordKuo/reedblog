import { Badge } from "@/components/ui/badge"
import type { PostStatus } from "@/lib/types"

export function PostStatusBadge({ status }: { status: PostStatus }) {
  return status === "published" ? (
    <Badge className="border-emerald-200 bg-emerald-50 text-emerald-700">已发布</Badge>
  ) : (
    <Badge variant="outline" className="border-amber-200 bg-amber-50 text-amber-700">
      草稿
    </Badge>
  )
}
