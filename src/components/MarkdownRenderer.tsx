import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import type { Components } from 'react-markdown'

export default function MarkdownRenderer({ content }: { content: string }) {
  return (
    <div className="markdown-body text-sm leading-relaxed">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={components}
      >
        {content}
      </ReactMarkdown>
    </div>
  )
}

const components: Components = {
  code({ className, children, ...props }) {
    const isInline = !className
    const code = String(children).replace(/\n$/, '')
    if (isInline) {
      return (
        <code
          className="px-1.5 py-0.5 rounded bg-white/10 text-accent-teal text-[13px] font-mono"
          {...props}
        >
          {children}
        </code>
      )
    }
    return (
      <div className="relative group my-3">
        <pre className="overflow-x-auto rounded-lg bg-black/40 border border-white/10 p-3 text-[13px] leading-relaxed font-mono text-silver-200">
          <code className={className} {...props}>
            {code}
          </code>
        </pre>
      </div>
    )
  },
  pre({ children }) {
    return <>{children}</>
  },
  a({ href, children }) {
    return (
      <a
        href={href}
        target="_blank"
        rel="noopener noreferrer"
        className="text-accent-cyan underline hover:text-accent-teal transition-colors"
      >
        {children}
      </a>
    )
  },
  strong({ children }) {
    return <strong className="font-semibold text-white">{children}</strong>
  },
  em({ children }) {
    return <em className="italic text-silver-100">{children}</em>
  },
  ul({ children }) {
    return <ul className="list-disc pl-5 my-1.5 space-y-0.5 text-silver-200">{children}</ul>
  },
  ol({ children }) {
    return <ol className="list-decimal pl-5 my-1.5 space-y-0.5 text-silver-200">{children}</ol>
  },
  li({ children }) {
    return <li className="leading-relaxed">{children}</li>
  },
  h1({ children }) {
    return <h1 className="text-base font-bold text-white mt-3 mb-1.5">{children}</h1>
  },
  h2({ children }) {
    return <h2 className="text-[15px] font-bold text-white mt-2.5 mb-1">{children}</h2>
  },
  h3({ children }) {
    return <h3 className="text-sm font-semibold text-white mt-2 mb-1">{children}</h3>
  },
  p({ children }) {
    return <p className="my-1 last:mb-0">{children}</p>
  },
  table({ children }) {
    return (
      <div className="overflow-x-auto my-2">
        <table className="w-full text-sm border-collapse">{children}</table>
      </div>
    )
  },
  thead({ children }) {
    return <thead className="border-b border-white/20">{children}</thead>
  },
  tbody({ children }) {
    return <tbody>{children}</tbody>
  },
  tr({ children }) {
    return <tr className="border-b border-white/10">{children}</tr>
  },
  th({ children }) {
    return <th className="px-3 py-1.5 text-left text-silver-300 font-medium text-xs">{children}</th>
  },
  td({ children }) {
    return <td className="px-3 py-1.5 text-silver-200 text-xs">{children}</td>
  },
  blockquote({ children }) {
    return (
      <blockquote className="border-l-2 border-accent-cyan/40 pl-3 my-2 text-silver-400 italic">
        {children}
      </blockquote>
    )
  },
  hr() {
    return <hr className="my-3 border-white/10" />
  },
}
