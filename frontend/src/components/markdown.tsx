import "highlight.js/styles/github.css"
// KaTeX 样式与字体随前端构建产物打包（不放主题包 assets：换主题不影响公式渲染）
import "katex/dist/katex.min.css"

import ReactMarkdown from "react-markdown"
import rehypeHighlight from "rehype-highlight"
import rehypeKatex from "rehype-katex"
import remarkGfm from "remark-gfm"
import remarkMath from "remark-math"

import { preprocessMathSource } from "@/lib/math"
import { cn } from "@/lib/utils"

/**
 * Markdown 渲染：GFM + 代码高亮 + KaTeX 公式（`$…$` 行内 / `$$…$$` 块级）
 * + typography 排版。文章详情页正文与编辑器实时预览共用本组件，公式渲染
 * 只有这一处实现（`@/lib/math` 的预处理与插件同源）。
 *
 * 公式链路：preprocessMathSource 做货币防误判、未闭合块级公式转义与单行
 * `$$…$$` 的块级展开 → remark-math 解析（代码区天然不参与）→ rehype-katex
 * 渲染（颜色继承 currentColor，暗色可读）。
 */
export function Markdown({ children, className }: { children: string; className?: string }) {
  return (
    <div className={cn("prose prose-neutral max-w-none", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex, rehypeHighlight]}
        components={{
          a: ({ node: _node, ...props }) => <a target="_blank" rel="noreferrer" {...props} />,
          // 正文图片懒加载：loading=lazy + decoding=async（首屏 hero 类图片由主题另行提供，
          // 不经过本组件；媒体库缩略图与 widgets 的 HTML 注入也不受影响）
          img: ({ node: _node, ...props }) => (
            <img {...props} loading="lazy" decoding="async" />
          ),
        }}
      >
        {preprocessMathSource(children)}
      </ReactMarkdown>
    </div>
  )
}
