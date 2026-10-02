import { Link, useParams } from "react-router-dom"
import { ArrowLeftIcon } from "lucide-react"

import { PostFeed } from "@/components/post-feed"
import { Button } from "@/components/ui/button"

export default function ArchiveMonthPage() {
  const params = useParams()
  const y = Number(params.year)
  const m = Number(params.month)
  const year = Number.isInteger(y) && y > 0 ? y : null
  const month = Number.isInteger(m) && m >= 1 && m <= 12 ? m : null

  if (year === null || month === null) {
    return (
      <div className="flex flex-col items-center gap-4 py-24 text-center">
        <h1 className="text-xl font-semibold">无效的归档时间</h1>
        <Button variant="outline" asChild>
          <Link to="/archive">
            <ArrowLeftIcon />
            返回归档
          </Link>
        </Button>
      </div>
    )
  }

  return <PostFeed year={year} month={month} heading={`归档：${year} 年 ${month} 月`} />
}
