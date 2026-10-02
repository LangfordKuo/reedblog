import { Loader2 } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"

export function FullPageSpinner({ label }: { label?: string }) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-3 text-muted-foreground">
      <Loader2 className="size-8 animate-spin" />
      {label && <p className="text-sm">{label}</p>}
    </div>
  )
}

export function BlockSpinner({ label = "加载中…" }: { label?: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-16 text-muted-foreground">
      <Loader2 className="size-6 animate-spin" />
      <p className="text-sm">{label}</p>
    </div>
  )
}

export function BackendDownScreen({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="flex min-h-screen items-center justify-center p-4">
      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>无法连接后端服务</CardTitle>
          <CardDescription>
            请确认后端已在 http://localhost:3000 启动（开发模式下由 Vite 将 /api 代理过去），然后重试。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button onClick={onRetry} className="w-full">
            重试
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
