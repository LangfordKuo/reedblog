import type { SiteSettings } from "@/lib/types"

/** 站点页脚：版权行 + 自定义文字（有值时）+ ICP 备案号（有值时，链到工信部备案系统） */
export function SiteFooter({ site }: { site: SiteSettings | null }) {
  return (
    <footer className="border-t py-8">
      <div className="mx-auto flex w-full max-w-5xl flex-col items-center gap-1.5 px-4 text-center text-sm text-muted-foreground">
        <div>
          © {new Date().getFullYear()} {site?.title || "reedblog"} · Powered by{" "}
          <span className="font-medium">reedblog</span>
        </div>
        {site?.footer_text && (
          <div className="whitespace-pre-line">{site.footer_text}</div>
        )}
        {site?.icp_number && (
          <a
            href="https://beian.miit.gov.cn"
            target="_blank"
            rel="noreferrer"
            className="transition-colors hover:text-foreground hover:underline"
          >
            {site.icp_number}
          </a>
        )}
      </div>
    </footer>
  )
}
