import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { FileWarning, Copy, Trash2, RefreshCw } from 'lucide-react'

interface DuplicateFile {
  name: string
  path: string
  size: number
  modified: string
}

interface DuplicateGroup {
  id: string
  files: DuplicateFile[]
  total_wasted: number
  reason: string
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}

export default function OrganizeSuggestions() {
  const [duplicates, setDuplicates] = useState<DuplicateGroup[]>([])
  const [loading, setLoading] = useState(false)
  const [totalWasted, setTotalWasted] = useState(0)

  const handleFindDuplicates = async () => {
    setLoading(true)
    try {
      const result = await invoke<DuplicateGroup[]>('find_duplicates')
      setDuplicates(result)
      setTotalWasted(result.reduce((sum, g) => sum + g.total_wasted, 0))
    } catch (err) {
      console.error('Find duplicates failed:', err)
      setDuplicates([])
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    handleFindDuplicates()
  }, [])

  return (
    <div className="h-full flex flex-col">
      <div className="p-6 pb-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold mb-1">整理建议</h2>
            <p className="text-sm text-silver-400">
              检测重复文件，释放存储空间
              {totalWasted > 0 && ` — 可释放 ${formatSize(totalWasted)}`}
            </p>
          </div>
          <button
            onClick={handleFindDuplicates}
            disabled={loading}
            className="glass px-4 py-2 rounded-lg text-sm flex items-center gap-2 text-accent-cyan hover:bg-accent-cyan/10 disabled:opacity-40 transition-all"
          >
            <RefreshCw size={16} className={loading ? 'animate-spin' : ''} />
            重新检测
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-auto px-6 pb-6">
        {loading && (
          <div className="flex items-center justify-center h-32">
            <RefreshCw size={24} className="text-accent-cyan animate-spin" />
          </div>
        )}

        {!loading && duplicates.length === 0 && (
          <div className="flex flex-col items-center justify-center h-64 text-silver-500">
            <Copy size={40} className="mb-3 opacity-30" />
            <p className="text-sm">未发现重复文件</p>
            <p className="text-xs mt-1 opacity-60">所有文件都是独一无二的</p>
          </div>
        )}

        <div className="space-y-4">
          {duplicates.map((group) => (
            <div key={group.id} className="glass rounded-xl p-4">
              <div className="flex items-center gap-2 mb-3">
                <FileWarning size={16} className="text-amber-400 shrink-0" />
                <span className="text-sm text-amber-400">
                  {group.reason} — {group.files.length} 个副本
                </span>
                <span className="text-xs text-red-400 ml-auto">
                  可释放 {formatSize(group.total_wasted)}
                </span>
              </div>
              <div className="space-y-1">
                {group.files.map((f, i) => (
                  <div
                    key={f.path}
                    className="flex items-center justify-between p-2 rounded-lg bg-white/5"
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="text-xs text-silver-500 w-5 shrink-0">
                        #{i + 1}
                      </span>
                      <span className="text-sm truncate">{f.name}</span>
                    </div>
                    <div className="flex items-center gap-3 shrink-0 ml-4">
                      <span className="text-xs text-silver-500">{formatSize(f.size)}</span>
                      <span className="text-xs text-silver-600">
                        {f.modified.slice(0, 10)}
                      </span>
                      <button
                        className="p-1 rounded text-silver-400 hover:text-red-400 hover:bg-white/5"
                        title="删除此副本"
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
