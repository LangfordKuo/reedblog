import type { SiteInfo } from "@/lib/types"

export function SiteFooter({ site }: { site: SiteInfo | null }) {
  return (
    <footer className="border-t py-8">
      <div className="mx-auto w-full max-w-5xl px-4 text-center text-sm text-muted-foreground">
        © {new Date().getFullYear()} {site?.title || "reedblog"} · Powered by{" "}
        <span className="font-medium">reedblog</span>
      </div>
    </footer>
  )
}
