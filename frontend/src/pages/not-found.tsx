import { Link } from "react-router-dom"

import { Button } from "@/components/ui/button"

export default function NotFoundPage() {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-4 bg-background p-4 text-center">
      <p className="text-6xl font-bold tracking-tight text-muted-foreground">404</p>
      <h1 className="text-xl font-semibold">页面不存在</h1>
      <p className="text-sm text-muted-foreground">你访问的页面不存在或已被移动。</p>
      <Button asChild>
        <Link to="/">返回首页</Link>
      </Button>
    </div>
  )
}
