import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

interface PaginationProps {
  page: number
  perPage: number
  total: number
  onChange: (page: number) => void
  className?: string
}

/** 生成页码窗口：1 … 4 5 6 … 20 */
function pageWindow(current: number, totalPages: number): (number | "ellipsis")[] {
  if (totalPages <= 7) {
    return Array.from({ length: totalPages }, (_, i) => i + 1)
  }
  const pages: (number | "ellipsis")[] = [1]
  if (current > 3) pages.push("ellipsis")
  for (
    let i = Math.max(2, current - 1);
    i <= Math.min(totalPages - 1, current + 1);
    i++
  ) {
    pages.push(i)
  }
  if (current < totalPages - 2) pages.push("ellipsis")
  pages.push(totalPages)
  return pages
}

export function Pagination({ page, perPage, total, onChange, className }: PaginationProps) {
  const totalPages = Math.max(1, Math.ceil(total / perPage))
  if (totalPages <= 1) return null

  const go = (p: number) => {
    if (p < 1 || p > totalPages || p === page) return
    onChange(p)
    window.scrollTo({ top: 0, behavior: "smooth" })
  }

  return (
    <nav
      className={cn("flex items-center justify-center gap-1", className)}
      aria-label="分页"
    >
      <Button
        variant="outline"
        size="icon"
        className="size-8"
        disabled={page <= 1}
        onClick={() => go(page - 1)}
        aria-label="上一页"
      >
        <ChevronLeftIcon className="size-4" />
      </Button>
      {pageWindow(page, totalPages).map((p, i) =>
        p === "ellipsis" ? (
          <span key={`e-${i}`} className="text-muted-foreground px-1 text-sm">
            …
          </span>
        ) : (
          <Button
            key={p}
            variant={p === page ? "default" : "outline"}
            size="icon"
            className="size-8"
            onClick={() => go(p)}
            aria-current={p === page ? "page" : undefined}
          >
            {p}
          </Button>
        ),
      )}
      <Button
        variant="outline"
        size="icon"
        className="size-8"
        disabled={page >= totalPages}
        onClick={() => go(page + 1)}
        aria-label="下一页"
      >
        <ChevronRightIcon className="size-4" />
      </Button>
    </nav>
  )
}
