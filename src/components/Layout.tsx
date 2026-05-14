import { Search, FolderTree, Sparkles, Shield, Download, CheckCircle, Loader, Sun, Moon, Cpu, X } from 'lucide-react'
import { useAppStore, type NavPage } from '../store/appStore'
import { useThemeStore } from '../store/themeStore'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useState, useEffect } from 'react'

const navItems: { id: NavPage; label: string; icon: React.ReactNode }[] = [
  { id: 'search', label: '智能搜索', icon: <Search size={20} /> },
  { id: 'classify', label: '文件分类', icon: <FolderTree size={20} /> },
  { id: 'organize', label: '整理建议', icon: <Sparkles size={20} /> },
  { id: 'vault', label: '安全空间', icon: <Shield size={20} /> },
]

interface ModelInfo {
  id: string
  name: string
  description: string
  url: string
  target_dir: string
  target_file: string
  expected_size_mb: number
  is_downloaded: boolean
}

interface DownloadProgress {
  model_id: string
  bytes_downloaded: number
  total_bytes: number
  progress_pct: number
  status: string
  error: string | null
}

export default function Layout({ children }: { children: React.ReactNode }) {
  const { currentPage, setPage } = useAppStore()
  const { theme, toggle } = useThemeStore()
  const [models, setModels] = useState<ModelInfo[]>([])
  const [progress, setProgress] = useState<Record<string, DownloadProgress>>({})

  useEffect(() => {
    invoke<ModelInfo[]>('get_models_status').then(setModels).catch(() => {})
  }, [])

  useEffect(() => {
    let unlisten: (() => void) | undefined
    ;(async () => {
      unlisten = await listen<DownloadProgress>('download-progress', (event) => {
        const p = event.payload
        setProgress((prev) => ({ ...prev, [p.model_id]: p }))
        // Remove progress after completion or error
        if (p.status === 'completed' || p.status === 'error') {
          setTimeout(() => {
            setProgress((prev) => {
              const next = { ...prev }
              delete next[p.model_id]
              return next
            })
          }, 3000)
          // Refresh model status
          invoke<ModelInfo[]>('get_models_status').then(setModels).catch(() => {})
        }
      })
    })()
    return () => { unlisten?.() }
  }, [])

  const handleDownload = async (modelId: string) => {
    try {
      await invoke<string>('download_model_file', { modelId })
    } catch (e) {
      console.error('Download failed:', e)
    }
  }

  return (
    <div className="flex h-full">
      {/* Sidebar */}
      <aside className="bg-surface flex w-60 flex-col shrink-0 h-full border-r border-subtle">
        <div className="px-5 py-6 border-b border-subtle">
          <h1 className="text-sm font-semibold tracking-wider text-accent-cyan uppercase">
            Semantic Drive
          </h1>
          <p className="text-xs text-muted mt-1">语义智能文件管家</p>
        </div>

        <nav className="flex-1 py-4 px-3 space-y-1 overflow-y-auto">
          {navItems.map((item) => (
            <button
              key={item.id}
              onClick={() => setPage(item.id)}
              className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition-all duration-200 ${
                currentPage === item.id
                  ? 'bg-deepsea-500/60 text-white shadow-sm'
                  : 'text-secondary hover:text-primary hover:bg-hover'
              }`}
            >
              {item.icon}
              {item.label}
            </button>
          ))}
        </nav>

        {/* Model Status Section */}
        <div className="px-3 py-3 border-t border-subtle">
          <div className="flex items-center gap-2 px-3 py-1.5 text-xs text-dim">
            <Cpu size={12} />
            <span>AI 模型</span>
          </div>
          <div className="mt-1 space-y-1">
            {models.map((model) => {
              const dl = progress[model.id]
              const isDownloading = dl && dl.status === 'downloading'
              return (
                <div key={model.id} className="px-3 py-1.5 rounded-lg text-xs">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2 min-w-0">
                      {model.is_downloaded ? (
                        <CheckCircle size={12} className="text-green-400 shrink-0" />
                      ) : isDownloading ? (
                        <Loader size={12} className="text-accent-cyan animate-spin shrink-0" />
                      ) : (
                        <Cpu size={12} className="text-dim shrink-0" />
                      )}
                      <span className="text-secondary truncate">{model.name}</span>
                    </div>
                    {!model.is_downloaded && (
                      <button
                        onClick={() => handleDownload(model.id)}
                        disabled={!!isDownloading}
                        className="shrink-0 ml-2 p-1 rounded hover:bg-hover text-accent-cyan hover:text-white disabled:opacity-40 transition-colors"
                        title={`下载 (${model.expected_size_mb}MB)`}
                      >
                        {isDownloading ? (
                          <span className="text-xs text-accent-cyan font-medium">{Math.round(dl!.progress_pct)}%</span>
                        ) : (
                          <Download size={12} />
                        )}
                      </button>
                    )}
                    {model.is_downloaded && (
                      <span className="text-xs text-green-400/60 shrink-0 ml-2">就绪</span>
                    )}
                  </div>
                  {/* Progress bar */}
                  {isDownloading && dl && (
                    <div className="mt-2 progress-bar">
                      <div
                        className="progress-bar-fill"
                        style={{ width: `${dl.progress_pct}%` }}
                      />
                    </div>
                  )}
                  {dl && dl.status === 'error' && (
                    <div className="mt-1 flex items-start gap-1 text-xs text-red-400">
                      <X size={10} className="mt-0.5 shrink-0" />
                      <span className="truncate" title={dl.error || ''}>
                        {dl.error ? (
                          dl.error.includes('网络') ? '网络不可达' :
                          dl.error.includes('HTTP') ? '文件不存在' :
                          dl.error.includes('DNS') ? 'DNS解析失败' :
                          '下载失败'
                        ) : '下载失败'}
                      </span>
                    </div>
                  )}
                </div>
              )
            })}
            {models.length === 0 && (
              <div className="px-3 py-2 text-xs text-dim">检测中...</div>
            )}
          </div>
        </div>

        <div className="px-3 py-4 border-t border-subtle">
          <button
            onClick={toggle}
            className="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm text-secondary hover:text-primary hover:bg-hover transition-all duration-200"
          >
            {theme === 'dark' ? <Sun size={18} /> : <Moon size={18} />}
            {theme === 'dark' ? '浅色模式' : '暗夜模式'}
          </button>
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-auto">{children}</main>
    </div>
  )
}
