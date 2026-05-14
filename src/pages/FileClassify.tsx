import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Tag, RefreshCw } from 'lucide-react'

interface CategoryInfo {
  name: string
  label: string
  count: number
  total_size: number
}

interface TagInfo {
  name: string
  count: number
}

interface ClassificationResult {
  categories: CategoryInfo[]
  tags: TagInfo[]
}

const COLORS = [
  'bg-emerald-500', 'bg-blue-500', 'bg-purple-500', 'bg-amber-500',
  'bg-cyan-500', 'bg-pink-500', 'bg-red-500', 'bg-indigo-500',
  'bg-slate-500', 'bg-gray-500',
]

function formatSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}

export default function FileClassify() {
  const [data, setData] = useState<ClassificationResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [totalFiles, setTotalFiles] = useState(0)

  const handleClassify = async () => {
    setLoading(true)
    try {
      const result = await invoke<ClassificationResult>('classify_files')
      setData(result)
      const total = result.categories.reduce((sum, c) => sum + c.count, 0)
      setTotalFiles(total)
    } catch (err) {
      console.error('Classify failed:', err)
      setData(null)
    } finally {
      setLoading(false)
    }
  }

  // Auto-classify on mount
  useEffect(() => {
    handleClassify()
  }, [])

  return (
    <div className="h-full flex flex-col">
      <div className="p-6 pb-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold mb-1">文件分类</h2>
            <p className="text-sm text-silver-400">
              自动按内容归类，不移动原文件
              {totalFiles > 0 && ` — 共 ${totalFiles} 个文件`}
            </p>
          </div>
          <button
            onClick={handleClassify}
            disabled={loading}
            className="glass px-4 py-2 rounded-lg text-sm flex items-center gap-2 text-accent-cyan hover:bg-accent-cyan/10 disabled:opacity-40 transition-all"
          >
            <RefreshCw size={16} className={loading ? 'animate-spin' : ''} />
            重新整理
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-auto px-6 pb-6">
        {loading && (
          <div className="flex items-center justify-center h-32">
            <RefreshCw size={24} className="text-accent-cyan animate-spin" />
          </div>
        )}

        {!loading && data && data.categories.length === 0 && (
          <div className="flex flex-col items-center justify-center h-64 text-silver-500">
            <Tag size={40} className="mb-3 opacity-30" />
            <p className="text-sm">请先运行扫描以进行文件分类</p>
          </div>
        )}

        {data && data.categories.length > 0 && (
          <>
            {/* Category stats */}
            <div className="glass rounded-xl p-6 mb-6">
              <h3 className="text-sm font-medium mb-4">分类统计</h3>
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
                {data.categories.map((cat, i) => (
                  <div
                    key={cat.name}
                    className="flex items-center gap-3 p-3 rounded-lg bg-white/5"
                  >
                    <div className={`w-3 h-3 rounded-full ${COLORS[i % COLORS.length]} shrink-0`} />
                    <div className="min-w-0 flex-1">
                      <div className="text-sm text-silver-300 truncate">{cat.name}</div>
                      <div className="text-xs text-silver-500">
                        {cat.count} 个 · {formatSize(cat.total_size)}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>

            {/* Tags */}
            <div className="glass rounded-xl p-6">
              <div className="flex items-center gap-2 mb-4">
                <Tag size={16} className="text-silver-400" />
                <h3 className="text-sm font-medium">智能标签</h3>
              </div>
              <div className="flex flex-wrap gap-2">
                {data.tags.slice(0, 30).map((tag) => (
                  <span
                    key={tag.name}
                    className="px-3 py-1 rounded-full text-xs bg-white/10 text-silver-300 border border-white/5 hover:bg-white/15 cursor-default transition-colors"
                    title={`${tag.count} 个文件`}
                  >
                    {tag.name}
                    <span className="ml-1 text-silver-500">({tag.count})</span>
                  </span>
                ))}
                {data.tags.length === 0 && (
                  <span className="px-3 py-1 rounded-full text-xs bg-white/5 text-silver-400 border border-white/5">
                    暂无标签
                  </span>
                )}
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
