import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Tag, RefreshCw, FolderOpen, ExternalLink, ArrowLeft, Search } from 'lucide-react'
import { useAppStore } from '../store/appStore'

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

interface ClassifyProgress {
  status: string
  current: number
  total: number
  error?: string
}

interface FileEntry {
  id: string
  path: string
  name: string
  extension: string
  mime_type: string
  size: number
  hash: string | null
  modified: string
  created: string
  indexed_at: string
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

function getFileIcon(name: string, ext?: string): string {
  const e = (ext || name.split('.').pop() || '').toLowerCase()
  const icons: Record<string, string> = {
    pdf: '📄', doc: '📝', docx: '📝', xls: '📊', xlsx: '📊',
    ppt: '📽️', pptx: '📽️', txt: '📃', jpg: '🖼️', jpeg: '🖼️',
    png: '🖼️', gif: '🖼️', webp: '🖼️', mp3: '🎵', wav: '🎵',
    mp4: '🎬', avi: '🎬', mkv: '🎬', mov: '🎬',
    zip: '📦', rar: '📦', '7z': '📦',
    rs: '🦀', py: '🐍', js: '📜', ts: '📜',
  }
  return icons[e] || '📁'
}

export default function FileClassify() {
  const [data, setData] = useState<ClassificationResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [totalFiles, setTotalFiles] = useState(0)
  const [classifyProgress, setClassifyProgress] = useState<ClassifyProgress | null>(null)

  // Category drill-down
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null)
  const [categoryFiles, setCategoryFiles] = useState<FileEntry[]>([])

  // Tag drill-down
  const [selectedTag, setSelectedTag] = useState<string | null>(null)
  const [tagFiles, setTagFiles] = useState<FileEntry[]>([])

  const [loadingFiles, setLoadingFiles] = useState(false)

  const handleClassify = async () => {
    setLoading(true)
    setSelectedCategory(null)
    setCategoryFiles([])
    setSelectedTag(null)
    setTagFiles([])
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

  useEffect(() => {
    handleClassify()
  }, [])

  // Listen for classify progress events from backend
  useEffect(() => {
    const unlisten = listen<ClassifyProgress>('classify-progress', (event) => {
      setClassifyProgress(event.payload)
    })
    return () => { unlisten.then(fn => fn()) }
  }, [])

  const clearSelection = () => {
    setSelectedCategory(null)
    setCategoryFiles([])
    setSelectedTag(null)
    setTagFiles([])
  }

  const handleCategoryClick = async (catName: string) => {
    if (selectedCategory === catName) {
      clearSelection()
      return
    }
    setSelectedCategory(catName)
    setSelectedTag(null)
    setTagFiles([])
    setLoadingFiles(true)
    try {
      const files = await invoke<FileEntry[]>('get_files_by_category', {
        categoryName: catName,
      })
      setCategoryFiles(files)
    } catch (err) {
      console.error('Failed to load category files:', err)
      setCategoryFiles([])
    } finally {
      setLoadingFiles(false)
    }
  }

  const handleTagClick = async (tagName: string) => {
    if (selectedTag === tagName) {
      clearSelection()
      return
    }
    setSelectedTag(tagName)
    setSelectedCategory(null)
    setCategoryFiles([])
    setLoadingFiles(true)
    try {
      const files = await invoke<FileEntry[]>('get_files_by_tag', {
        tagName,
      })
      setTagFiles(files)
    } catch (err) {
      console.error('Failed to load tag files:', err)
      setTagFiles([])
    } finally {
      setLoadingFiles(false)
    }
  }

  const handleOpenLocation = async (filePath: string) => {
    try {
      await invoke('open_file_location', { filePath })
    } catch (err) {
      console.error('Open location failed:', err)
    }
  }

  const handleOpenFile = async (filePath: string) => {
    try {
      await invoke('open_file', { filePath })
    } catch (err) {
      console.error('Open file failed:', err)
    }
  }

  const handleSearchView = (filePath: string) => {
    useAppStore.getState().setPage('search')
    useAppStore.getState().setSearchQuery(filePath)
  }

  const drillDownFiles = selectedCategory ? categoryFiles : tagFiles
  const drillDownTitle = selectedCategory
    ? `${selectedCategory} 下的文件`
    : selectedTag
    ? `标签"${selectedTag}"下的文件`
    : ''

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
        {loading && classifyProgress && classifyProgress.status !== 'done' && (
          <div className="glass rounded-xl p-6 mb-4">
            <div className="flex items-center justify-between mb-3">
              <span className="text-sm font-medium">
                {classifyProgress.status === 'classifying' ? '正在分析文件分类...' : '正在保存分类结果...'}
              </span>
            </div>
            <div className="w-full h-2 rounded-full bg-white/10 overflow-hidden">
              <div
                className="h-full rounded-full bg-gradient-to-r from-accent-cyan to-accent-teal animate-pulse"
                style={{ width: classifyProgress.status === 'saving' ? '80%' : '40%' }}
              />
            </div>
          </div>
        )}

        {loading && classifyProgress && classifyProgress.status === 'error' && (
          <div className="glass rounded-xl p-6 mb-4 border border-red-500/20">
            <div className="flex items-center gap-2 text-sm text-red-400">
              <span>分类分析失败: {classifyProgress.error || '未知错误'}</span>
            </div>
          </div>
        )}

        {loading && !classifyProgress && (
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
                {data.categories.map((cat, i) => {
                  const isSelected = selectedCategory === cat.name
                  return (
                    <button
                      key={cat.name}
                      onClick={() => handleCategoryClick(cat.name)}
                      className={`flex items-center gap-3 p-3 rounded-lg text-left transition-all ${
                        isSelected
                          ? 'bg-accent-cyan/15 ring-1 ring-accent-cyan/40'
                          : 'bg-white/5 hover:bg-white/10'
                      }`}
                    >
                      <div className={`w-3 h-3 rounded-full ${COLORS[i % COLORS.length]} shrink-0`} />
                      <div className="min-w-0 flex-1">
                        <div className="text-sm text-silver-300 truncate">{cat.name}</div>
                        <div className="text-xs text-silver-500">
                          {cat.count} 个 · {formatSize(cat.total_size)}
                        </div>
                      </div>
                      {isSelected && (
                        <span className="text-[10px] text-accent-cyan shrink-0">已选</span>
                      )}
                    </button>
                  )
                })}
              </div>
            </div>

            {/* Drill-down file list (category or tag) */}
            {(selectedCategory || selectedTag) && (
              <div className="glass rounded-xl p-6 mb-6">
                <div className="flex items-center justify-between mb-4">
                  <div className="flex items-center gap-2">
                    <button
                      onClick={clearSelection}
                      className="p-1 rounded text-silver-400 hover:text-white hover:bg-white/10 transition-all"
                      title="返回"
                    >
                      <ArrowLeft size={16} />
                    </button>
                    <h3 className="text-sm font-medium">{drillDownTitle}</h3>
                  </div>
                  {loadingFiles && (
                    <RefreshCw size={14} className="text-accent-cyan animate-spin" />
                  )}
                </div>

                {!loadingFiles && drillDownFiles.length === 0 && (
                  <p className="text-xs text-silver-500 py-4 text-center">
                    暂无文件
                  </p>
                )}

                {!loadingFiles && drillDownFiles.length > 0 && (
                  <div className="space-y-1.5 max-h-80 overflow-y-auto">
                    {drillDownFiles.map((f, idx) => (
                      <div
                        key={f.id}
                        className="flex items-center justify-between p-2 rounded-lg bg-white/5 hover:bg-white/10 transition-colors animate-slide-up"
                        style={{ animationDelay: `${idx * 40}ms` }}
                      >
                        <div className="flex items-center gap-2 min-w-0 flex-1">
                          <span className="shrink-0">{getFileIcon(f.name, f.extension)}</span>
                          <div className="min-w-0">
                            <div className="text-sm truncate">{f.name}</div>
                            <div className="text-xs text-silver-500 truncate">{f.path}</div>
                          </div>
                        </div>
                        <div className="flex items-center gap-1.5 shrink-0 ml-3">
                          <span className="text-xs text-silver-500">{formatSize(f.size)}</span>
                          <button
                            onClick={() => handleOpenLocation(f.path)}
                            className="p-1 rounded text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                            title="打开文件位置"
                          >
                            <FolderOpen size={13} />
                          </button>
                          <button
                            onClick={() => handleOpenFile(f.path)}
                            className="p-1 rounded text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                            title="打开文件"
                          >
                            <ExternalLink size={13} />
                          </button>
                          <button
                            onClick={() => handleSearchView(f.path)}
                            className="p-1 rounded text-silver-400 hover:text-accent-teal hover:bg-white/10 transition-all"
                            title="在搜索中查看"
                          >
                            <Search size={13} />
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* Tags */}
            <div className="glass rounded-xl p-6">
              <div className="flex items-center gap-2 mb-4">
                <Tag size={16} className="text-silver-400" />
                <h3 className="text-sm font-medium">智能标签</h3>
              </div>
              <div className="flex flex-wrap gap-2">
                {data.tags.slice(0, 30).map((tag) => {
                  const isSelected = selectedTag === tag.name
                  return (
                    <button
                      key={tag.name}
                      onClick={() => handleTagClick(tag.name)}
                      className={`px-3 py-1 rounded-full text-xs border transition-all ${
                        isSelected
                          ? 'bg-accent-cyan/15 text-accent-cyan border-accent-cyan/40'
                          : 'bg-white/10 text-silver-300 border-white/5 hover:bg-white/15'
                      }`}
                      title={`${tag.count} 个文件`}
                    >
                      {tag.name}
                      <span className="ml-1 text-silver-500">({tag.count})</span>
                    </button>
                  )
                })}
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
