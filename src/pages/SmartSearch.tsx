import { useState, useEffect, useCallback, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Search, Mic, FolderOpen, ExternalLink, RefreshCw, HardDrive, Upload, Plus, X, Check, ChevronDown, Tags } from 'lucide-react'
import { open } from '@tauri-apps/plugin-dialog'
import type { DirEntry, ClipboardState } from '../components/FileBrowser'
import FileBrowser from '../components/FileBrowser'
import { useSettingsStore } from '../store/settingsStore'
import { useAppStore } from '../store/appStore'

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

interface SearchResult {
  file_id: string
  file_name: string
  file_path: string
  score: number
  match_type: string
  snippet: string
  file_size: number
  modified: string
}

interface ScanProgress {
  files_found: number
  files_processed: number
  current_file: string
  done: boolean
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
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
    zip: '📦', rar: '📦', '7z': '📦', tar: '📦', gz: '📦',
    rs: '🦀', py: '🐍', js: '📜', ts: '📜', html: '🌐', css: '🎨',
  }
  return icons[e] || '📁'
}

export default function SmartSearch() {
  const [query, setQuery] = useState('')
  const [searching, setSearching] = useState(false)
  const [_files, _setFiles] = useState<FileEntry[]>([])
  const [searchResults, setSearchResults] = useState<SearchResult[]>([])
  const [fileCount, setFileCount] = useState(0)
  const [scanning, setScanning] = useState(false)
  const [scanProgress, setScanProgress] = useState<ScanProgress | null>(null)
  const [deviceRoot, setDeviceRoot] = useState('')
  const [isSearchMode, setIsSearchMode] = useState(false)
  const [searchTags, setSearchTags] = useState<string[]>([])
  const [feedback, setFeedback] = useState('')
  const [feedbackType, setFeedbackType] = useState<'info' | 'error'>('info')

  // Custom user tags state
  const [fileTags, setFileTags] = useState<Record<string, string[]>>({})
  const [topUserTags, setTopUserTags] = useState<[string, number][]>([])
  const [editingTagFileId, setEditingTagFileId] = useState<string | null>(null)
  const [tagInputText, setTagInputText] = useState('')
  const [showTagSuggestions, setShowTagSuggestions] = useState(false)
  const [collapsedFolders, setCollapsedFolders] = useState<Set<string>>(new Set())

  // Tag browsing mode state
  const [tagBrowseMode, setTagBrowseMode] = useState(false)
  const [allTags, setAllTags] = useState<[string, number][]>([])
  const [selectedBrowseTag, setSelectedBrowseTag] = useState<string | null>(null)
  const [tagBrowseResults, setTagBrowseResults] = useState<SearchResult[]>([])
  const [browseFileTags, setBrowseFileTags] = useState<Record<string, string[]>>({})

  // Directory browser state
  const [dirEntries, setDirEntries] = useState<DirEntry[]>([])
  const [currentPath, setCurrentPath] = useState('')
  const [clipboard, setClipboard] = useState<ClipboardState | null>(null)

  const showFeedback = useCallback((msg: string, type: 'info' | 'error' = 'info') => {
    setFeedback(msg)
    setFeedbackType(type)
    setTimeout(() => setFeedback(''), 5000)
  }, [])

  const loadDirectory = useCallback(async (path: string) => {
    try {
      const entries = await invoke<DirEntry[]>('get_directory', { dirPath: path })
      setDirEntries(entries)
      setCurrentPath(path)
    } catch (err) {
      showFeedback(`无法加载目录: ${err}`, 'error')
    }
  }, [showFeedback])

  useEffect(() => {
    invoke<string>('get_device_root')
      .then(setDeviceRoot)
      .catch(() => setDeviceRoot('未知'))
    invoke<number>('get_file_count')
      .then(count => {
        setFileCount(count)
      })
      .catch(() => setFileCount(0))
    invoke<[string, number][]>('get_top_user_tags', { limit: 50 })
      .then(setTopUserTags)
      .catch(() => {})
    invoke<FileEntry[]>('get_files')
      .then(_setFiles)
      .catch(() => {})
    loadDirectory('')
  }, [loadDirectory])

  // Listen for watcher "files-changed" events
  useEffect(() => {
    const unlisten = listen<{ kind: string; path: string }>('files-changed', (event) => {
      showFeedback(`检测到文件变化: ${event.payload.kind} - ${event.payload.path}`, 'info')
    })
    return () => { unlisten.then(fn => fn()) }
  }, [showFeedback])

  // Handle searchQuery from appStore (cross-page navigation)
  useEffect(() => {
    const sq = useAppStore.getState().searchQuery
    if (sq) {
      setQuery(sq)
      useAppStore.getState().setSearchQuery(null)
      // Trigger search after state updates
      const trigger = async () => {
        setSearching(true)
        try {
          const bilingualSearch = useSettingsStore.getState().bilingualSearch
          const results = await invoke<SearchResult[]>('search_files', {
            query: sq,
            tags: [],
            maxResults: 50,
            bilingual: bilingualSearch,
          })
          setSearchResults(results)
          setIsSearchMode(true)
          if (results.length > 0) {
            const ftags = await invoke<Record<string, string[]>>('get_files_custom_tags_batch', {
              fileIds: results.map(r => r.file_id),
            })
            setFileTags(ftags)
          } else {
            setFileTags({})
            showFeedback('未找到匹配的文件', 'info')
          }
        } catch (err) {
          showFeedback(`搜索失败: ${err}`, 'error')
        } finally {
          setSearching(false)
        }
      }
      trigger()
    }
  }, [showFeedback])

  // Poll scan progress every 300ms while scanning
  useEffect(() => {
    if (!scanning) return
    const interval = setInterval(async () => {
      try {
        const prog = await invoke<ScanProgress | null>('get_scan_progress')
        if (prog) {
          setScanProgress(prog)
          if (prog.done) {
            setScanning(false)
            setTimeout(() => setScanProgress(null), 1000)
          }
        }
      } catch { /* command may not be available yet */ }
    }, 300)
    return () => clearInterval(interval)
  }, [scanning])

  const fileIdByPath = useMemo(() => {
    const map: Record<string, string> = {}
    for (const f of _files) {
      map[f.path] = f.id
    }
    return map
  }, [_files])

  // Load custom tags for current directory files (browse mode tag editing)
  const loadBrowseTagsForDir = useCallback(async (entries: DirEntry[]) => {
    const fileIds = entries.filter(e => !e.is_dir).map(e => fileIdByPath[e.path]).filter(Boolean)
    if (fileIds.length === 0) { setBrowseFileTags({}); return }
    try {
      const ftags = await invoke<Record<string, string[]>>('get_files_custom_tags_batch', { fileIds })
      // Re-key by path
      const byPath: Record<string, string[]> = {}
      for (const e of entries) {
        const id = fileIdByPath[e.path]
        if (id && ftags[id]) byPath[e.path] = ftags[id]
      }
      setBrowseFileTags(byPath)
    } catch { setBrowseFileTags({}) }
  }, [fileIdByPath])

  // Load browse tags when directory entries change
  useEffect(() => {
    if (dirEntries.length > 0) loadBrowseTagsForDir(dirEntries)
  }, [dirEntries, loadBrowseTagsForDir])

  const handleScan = useCallback(async () => {
    setScanning(true)
    setFeedback('')
    setScanProgress(null)
    try {
      const entries = await invoke<FileEntry[]>('scan_files')
      _setFiles(entries)
      setSearchResults([])
      setIsSearchMode(false)
      setFileCount(entries.length)
      setScanning(false)
      setScanProgress(null)
      showFeedback(`扫描完成，共 ${entries.length} 个文件`)
      await invoke<string>('get_device_root').then(setDeviceRoot)
      loadDirectory('')
    } catch (err) {
      showFeedback(`扫描失败: ${err}`, 'error')
      setScanning(false)
      setScanProgress(null)
    }
  }, [loadDirectory, showFeedback])

  const handleSearch = async () => {
    const hasMainQuery = query.trim() !== ''
    const hasTags = searchTags.some(t => t.trim() !== '')
    if (!hasMainQuery && !hasTags) {
      setIsSearchMode(false)
      loadDirectory(currentPath || '')
      return
    }
    setSearching(true)
    setFeedback('')
    setCollapsedFolders(new Set())
    try {
      const bilingualSearch = useSettingsStore.getState().bilingualSearch
      const results = await invoke<SearchResult[]>('search_files', {
        query: query.trim(),
        tags: searchTags.filter(t => t.trim() !== ''),
        maxResults: 50,
        bilingual: bilingualSearch,
      })
      setSearchResults(results)
      setIsSearchMode(true)
      if (results.length > 0) {
        try {
          const ftags = await invoke<Record<string, string[]>>('get_files_custom_tags_batch', {
            fileIds: results.map(r => r.file_id),
          })
          setFileTags(ftags)
        } catch { setFileTags({}) }
      } else {
        setFileTags({})
        showFeedback('未找到匹配的文件，请尝试其他关键词', 'info')
      }
    } catch (err) {
      showFeedback(`搜索失败: ${err}`, 'error')
    } finally {
      setSearching(false)
    }
  }

  // Directory navigation
  const handleNavigate = useCallback((path: string) => {
    loadDirectory(path)
    setIsSearchMode(false)
    setQuery('')
  }, [loadDirectory])

  const handleNavigateUp = useCallback(() => {
    const parent = currentPath.includes('/')
      ? currentPath.slice(0, currentPath.lastIndexOf('/'))
      : ''
    loadDirectory(parent)
    setIsSearchMode(false)
    setQuery('')
  }, [currentPath, loadDirectory])

  const handleRefresh = useCallback(() => {
    loadDirectory(currentPath)
  }, [currentPath, loadDirectory])

  const handleCreateDirectory = useCallback(async () => {
    const name = prompt('请输入文件夹名称')
    if (!name || !name.trim()) return
    const newPath = currentPath ? `${currentPath}/${name.trim()}` : name.trim()
    try {
      await invoke('create_directory', { dirPath: newPath })
      showFeedback(`已创建文件夹: ${name.trim()}`)
      loadDirectory(currentPath)
    } catch (err) {
      showFeedback(`创建文件夹失败: ${err}`, 'error')
    }
  }, [currentPath, loadDirectory, showFeedback])

  const handleImport = async () => {
    try {
      const selected = await open({ multiple: false, title: '从电脑选择文件导入到U盘' })
      if (!selected) return
      const name = selected.split('\\').pop()?.split('/').pop() || 'imported_file'
      const dest = currentPath ? `${currentPath}/${name}` : name
      await invoke('import_file', { source: selected, destination: dest })
      showFeedback(`已导入: ${name}`)
      loadDirectory(currentPath)
    } catch (err) {
      showFeedback(`导入失败: ${err}`, 'error')
    }
  }

  // File operations
  const handleOpenLocation = async (filePath: string) => {
    try {
      await invoke('open_file_location', { filePath })
    } catch (err) {
      showFeedback(`无法打开文件位置: ${err}`, 'error')
    }
  }

  const handleOpenFile = async (filePath: string) => {
    try {
      await invoke('open_file', { filePath })
    } catch (err) {
      showFeedback(`无法打开文件: ${err}`, 'error')
    }
  }

  const handleRename = async (filePath: string, currentName: string) => {
    const newName = prompt('输入新文件名:', currentName)
    if (!newName || newName === currentName) return
    try {
      await invoke('rename_file', { oldPath: filePath, newName })
      showFeedback(`已重命名为: ${newName}`)
      loadDirectory(currentPath)
    } catch (err) {
      showFeedback(`重命名失败: ${err}`, 'error')
    }
  }

  const handleDelete = async (filePath: string) => {
    if (!confirm(`确定要删除 "${filePath}" 吗？此操作不可撤销。`)) return
    try {
      await invoke('delete_file', { filePath })
      showFeedback(`已删除: ${filePath}`)
      loadDirectory(currentPath)
    } catch (err) {
      showFeedback(`删除失败: ${err}`, 'error')
    }
  }

  const handleCopy = (filePath: string) => {
    setClipboard({ action: 'copy', paths: [filePath] })
    showFeedback(`已复制: ${filePath}`)
  }

  const handleCut = (filePath: string) => {
    setClipboard({ action: 'cut', paths: [filePath] })
    showFeedback(`已剪切: ${filePath}`)
  }

  const handlePaste = async () => {
    if (!clipboard) return
    let pasted = 0
    for (const src of clipboard.paths) {
      const name = src.split('/').pop() || src
      const dst = currentPath ? `${currentPath}/${name}` : name
      try {
        if (clipboard.action === 'copy') {
          await invoke('copy_file', { source: src, destination: dst })
        } else {
          await invoke('move_file', { source: src, destination: dst })
        }
        pasted++
      } catch (err) {
        showFeedback(`粘贴失败: ${err}`, 'error')
      }
    }
    setClipboard(null)
    if (pasted > 0) {
      showFeedback(`已${clipboard.action === 'copy' ? '复制' : '移动'} ${pasted} 项`)
      loadDirectory(currentPath)
    }
  }

  // Custom tag operations
  const handleAddTag = async (fileId: string, tag: string) => {
    const trimmed = tag.trim()
    if (!trimmed) return
    const current = fileTags[fileId] || []
    if (current.includes(trimmed)) return
    const newTags = [...current, trimmed]
    try {
      await invoke('set_file_custom_tags', { fileId, tags: newTags })
      setFileTags(prev => ({ ...prev, [fileId]: newTags }))
      setEditingTagFileId(null)
      setTagInputText('')
      const updated = await invoke<[string, number][]>('get_top_user_tags', { limit: 50 })
      setTopUserTags(updated)
    } catch (err) {
      showFeedback(`添加标签失败: ${err}`, 'error')
    }
  }

  const handleRemoveTag = async (fileId: string, tag: string) => {
    const current = fileTags[fileId] || []
    const newTags = current.filter(t => t !== tag)
    try {
      await invoke('set_file_custom_tags', { fileId, tags: newTags })
      setFileTags(prev => ({ ...prev, [fileId]: newTags }))
    } catch (err) {
      showFeedback(`删除标签失败: ${err}`, 'error')
    }
  }

  // ── Tag browse mode handlers ──

  const enterTagBrowseMode = useCallback(async () => {
    setTagBrowseMode(true)
    setIsSearchMode(false)
    setSelectedBrowseTag(null)
    setQuery('')
    try {
      await invoke<number>('cleanup_orphan_tags')
      const tags = await invoke<[string, number][]>('get_all_tags_with_counts')
      setAllTags(tags)
      if (tags.length === 0) showFeedback('暂无标签，请先在搜索或浏览中为文件添加标签', 'info')
    } catch { setAllTags([]) }
  }, [showFeedback])

  const exitTagBrowseMode = useCallback(() => {
    setTagBrowseMode(false)
    setSelectedBrowseTag(null)
    setTagBrowseResults([])
    setBrowseFileTags({})
  }, [])

  const handleTagBrowseSelect = useCallback(async (tag: string) => {
    setSelectedBrowseTag(tag)
    try {
      const files = await invoke<FileEntry[]>('get_files_by_custom_tag', { tag })
      const results: SearchResult[] = files.map(f => ({
        file_id: f.id,
        file_name: f.name,
        file_path: f.path,
        score: 1.0,
        match_type: '标签匹配',
        snippet: '',
        file_size: f.size,
        modified: f.modified,
      }))
      setTagBrowseResults(results)
      // Load custom tags for these files
      if (files.length > 0) {
        const ftags = await invoke<Record<string, string[]>>('get_files_custom_tags_batch', {
          fileIds: files.map(f => f.id),
        })
        setFileTags(ftags)
      } else { setFileTags({}) }
    } catch { setTagBrowseResults([]); showFeedback('加载标签文件失败', 'error') }
  }, [showFeedback])

  // Browse mode tag editing (uses path → id mapping)
  const handleBrowseAddTag = useCallback(async (filePath: string, tag: string) => {
    const id = fileIdByPath[filePath]
    if (!id) { showFeedback('文件未在索引中，请先扫描', 'error'); return }
    const trimmed = tag.trim()
    if (!trimmed) return
    const current = browseFileTags[filePath] || []
    if (current.includes(trimmed)) return
    const newTags = [...current, trimmed]
    try {
      await invoke('set_file_custom_tags', { fileId: id, tags: newTags })
      // Update browse tags cache
      setBrowseFileTags(prev => ({ ...prev, [filePath]: newTags }))
      // Also update fileTags if this file is in search results
      setFileTags(prev => ({ ...prev, [id]: newTags }))
      const updated = await invoke<[string, number][]>('get_top_user_tags', { limit: 50 })
      setTopUserTags(updated)
    } catch (err) { showFeedback(`添加标签失败: ${err}`, 'error') }
  }, [fileIdByPath, browseFileTags, showFeedback])

  const handleBrowseRemoveTag = useCallback(async (filePath: string, tag: string) => {
    const id = fileIdByPath[filePath]
    if (!id) return
    const current = browseFileTags[filePath] || []
    const newTags = current.filter(t => t !== tag)
    try {
      await invoke('set_file_custom_tags', { fileId: id, tags: newTags })
      setBrowseFileTags(prev => ({ ...prev, [filePath]: newTags }))
      setFileTags(prev => ({ ...prev, [id]: newTags }))
    } catch (err) { showFeedback(`删除标签失败: ${err}`, 'error') }
  }, [fileIdByPath, browseFileTags, showFeedback])

  // Group search results by parent directory
  const groupByParentDir = (results: SearchResult[]): { dir: string; files: SearchResult[] }[] => {
    const groups: Record<string, SearchResult[]> = {}
    for (const r of results) {
      const parent = r.file_path.includes('/')
        ? r.file_path.slice(0, r.file_path.lastIndexOf('/'))
        : '/'
      if (!groups[parent]) groups[parent] = []
      groups[parent].push(r)
    }
    return Object.entries(groups)
      .map(([dir, files]) => ({ dir, files }))
      .sort((a, b) => a.dir.localeCompare(b.dir))
  }

  const toggleFolder = (dir: string) => {
    const next = new Set(collapsedFolders)
    if (next.has(dir)) next.delete(dir)
    else next.add(dir)
    setCollapsedFolders(next)
  }

  const pct = scanProgress && scanProgress.files_found > 0
    ? Math.round((scanProgress.files_processed / scanProgress.files_found) * 100)
    : 0

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="p-6 pb-4">
        <div className="flex items-center justify-between mb-1">
          <h2 className="text-xl font-semibold">智能搜索</h2>
          <div className="flex items-center gap-3 text-xs text-silver-400">
            <span className="flex items-center gap-1">
              <HardDrive size={14} />
              {deviceRoot || '加载中...'}
            </span>
            <span>{fileCount} 个文件已索引</span>
          </div>
        </div>
        <p className="text-sm text-silver-400 mb-4">
          用自然语言描述你想找的文件，例如"上周的Excel"、"2024年的照片"、"财务报表"
        </p>

        <div className="flex gap-3">
          {searchTags.length === 0 ? (
            <div className="flex-1 relative">
              <Search
                size={18}
                className="absolute left-3 top-1/2 -translate-y-1/2 text-silver-400"
              />
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
                placeholder="例：上周的Excel文件、2024年的照片、合同..."
                className="w-full glass rounded-xl pl-10 pr-4 py-3 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/50"
              />
            </div>
          ) : (
            <div className="flex-1 flex items-center gap-2 glass rounded-xl px-4 py-2">
              <span className="text-xs text-accent-cyan/60 font-medium shrink-0">条件搜索</span>
              <div className="flex-1 flex items-center gap-1.5 overflow-x-auto">
                {searchTags.map((tag, i) => (
                  <span
                    key={i}
                    className="inline-flex items-center gap-1 px-2 py-1 rounded-md bg-accent-cyan/10 text-xs text-accent-cyan border border-accent-cyan/20 whitespace-nowrap"
                  >
                    {tag || `条件 ${i + 1}`}
                  </span>
                ))}
              </div>
              <button
                onClick={() => { setSearchTags([]); setQuery(''); setIsSearchMode(false); loadDirectory(currentPath || ''); }}
                className="p-1 rounded text-silver-500 hover:text-red-400 hover:bg-white/10 transition-all shrink-0"
                title="退出条件搜索"
              >
                <X size={14} />
              </button>
            </div>
          )}
          <button
            onClick={() => setSearchTags([...searchTags, ''])}
            className="glass px-3 py-3 rounded-xl text-silver-400 hover:text-accent-cyan hover:bg-accent-cyan/10 transition-all"
            title="添加搜索条件"
          >
            <Plus size={18} />
          </button>
          <button
            onClick={handleScan}
            disabled={scanning}
            className="glass px-4 py-3 rounded-xl text-sm font-medium text-accent-teal hover:bg-accent-teal/10 disabled:opacity-40 transition-all flex items-center gap-2"
          >
            <RefreshCw size={16} className={scanning ? 'animate-spin' : ''} />
            {scanning ? '扫描中...' : '扫描'}
          </button>
          {searching ? (
            <button
              onClick={async () => {
                try { await invoke('cancel_search') } catch (error) { console.debug('Search cancellation failed', error) }
                setSearching(false)
              }}
              className="glass px-4 py-3 rounded-xl text-sm font-medium text-red-400 hover:bg-red-500/10 transition-all flex items-center gap-2"
            >
              <X size={16} />
              取消
            </button>
          ) : (
            <button
              onClick={handleSearch}
              disabled={searching}
              className="glass px-5 py-3 rounded-xl text-sm font-medium text-accent-cyan hover:bg-accent-cyan/10 disabled:opacity-40 transition-all"
            >
              搜索
            </button>
          )}
          <button className="glass p-3 rounded-xl text-silver-400 hover:text-accent-cyan transition-colors">
            <Mic size={18} />
          </button>
        </div>
        <p className="text-xs text-silver-500/60 mt-3">
          提示：目录浏览器直接读取文件系统无需等待；点击"扫描"会提取文件内容+AI分析，首次耗时较长，后续只处理新增/变更的文件。
        </p>

        {/* Search tags — independent search conditions */}
        {searchTags.length > 0 && (
          <div className="mt-2 space-y-1.5">
            {searchTags.map((tag, i) => (
              <div key={i} className="flex gap-2 items-center">
                <div className="flex-1 relative">
                  <span className="absolute left-3 top-1/2 -translate-y-1/2 text-[11px] text-accent-cyan/60 font-medium">
                    {i + 1}
                  </span>
                  <input
                    type="text"
                    value={tag}
                    onChange={(e) => {
                      const next = [...searchTags]
                      next[i] = e.target.value
                      setSearchTags(next)
                    }}
                    onFocus={() => {/* trigger autocomplete */}}
                    onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
                    placeholder="搜索条件..."
                    className="w-full glass-light rounded-lg pl-7 pr-4 py-2 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/30 border border-white/5"
                  />
                  {/* Autocomplete dropdown for stored tags */}
                  {tag.trim() && (
                    <div className="absolute left-0 top-full mt-1 z-20 glass rounded-lg p-1.5 shadow-lg border border-white/10 min-w-[130px] max-h-36 overflow-y-auto">
                      {topUserTags
                        .filter(([name]) => name.includes(tag.trim()) && name !== tag.trim())
                        .slice(0, 8)
                        .map(([name, freq]) => (
                          <button
                            key={name}
                            onMouseDown={() => {
                              const next = [...searchTags]
                              next[i] = name
                              setSearchTags(next)
                            }}
                            className="block w-full text-left px-2 py-1 rounded text-[11px] text-silver-300 hover:bg-white/10 hover:text-white transition-colors"
                          >
                            {name}
                            <span className="text-silver-500 ml-1">({freq})</span>
                          </button>
                        ))
                      }
                    </div>
                  )}
                </div>
                <button
                  onClick={() => setSearchTags(searchTags.filter((_, j) => j !== i))}
                  className="p-1.5 rounded-lg text-silver-500 hover:text-red-400 hover:bg-white/10 transition-all"
                  title="移除条件"
                >
                  <X size={14} />
                </button>
              </div>
            ))}
          </div>
        )}

	      </div>

	      {/* Results */}
      <div className="flex-1 overflow-auto px-6 pb-6">
        {/* Feedback bar */}
        {feedback && (
          <div
            className={`glass rounded-xl p-3 mb-4 text-sm border ${
              feedbackType === 'error'
                ? 'text-red-400 border-red-500/20'
                : 'text-accent-teal border-accent-teal/20'
            }`}
          >
            {feedback}
          </div>
        )}

        {/* Scan progress */}
        {scanning && scanProgress && (
          <div className="glass rounded-xl p-6 mb-4">
            <div className="flex items-center justify-between mb-3">
              <span className="text-sm font-medium">
                {scanProgress.done
                  ? '正在保存...'
                  : `正在分析文件 ${scanProgress.files_processed} / ${scanProgress.files_found}`}
              </span>
              <span className="text-xs text-silver-400">{pct}%</span>
            </div>
            <div className="w-full h-2 rounded-full bg-white/10 overflow-hidden">
              <div
                className="h-full rounded-full bg-gradient-to-r from-accent-cyan to-accent-teal transition-all duration-300 ease-out"
                style={{ width: `${pct}%` }}
              />
            </div>
            {scanProgress.current_file && (
              <p className="text-xs text-silver-500 mt-2 truncate">
                {scanProgress.current_file}
              </p>
            )}
          </div>
        )}

        {/* Initial scanning — no progress data yet */}
        {scanning && !scanProgress && (
          <div className="flex flex-col items-center justify-center h-64 gap-3">
            <RefreshCw size={32} className="text-accent-cyan animate-spin" />
            <p className="text-sm text-silver-400">正在扫描文件...</p>
            <p className="text-xs text-silver-500">列出目录结构中...</p>
          </div>
        )}

        {/* Searching indicator */}
        {!scanning && searching && (
          <div className="flex flex-col items-center justify-center h-64 gap-3">
            <RefreshCw size={32} className="text-accent-cyan animate-spin" />
            <p className="text-sm text-silver-400">正在搜索...</p>
          </div>
        )}

        {/* Search results */}
        {!scanning && !searching && isSearchMode && searchResults.length > 0 && (
          <>
            {(() => {
              const groups = groupByParentDir(searchResults)
              return (
                <>
                  <p className="text-xs text-silver-500 mb-3">
                    找到 {searchResults.length} 个匹配结果，来自 {groups.length} 个文件夹
                  </p>
                  <div className="space-y-3">
                    {groups.map(({ dir, files }, gIdx) => {
                      const isCollapsed = collapsedFolders.has(dir)
                      const displayDir = dir === '/' ? '根目录' : dir
                      return (
                        <div key={dir} className="glass rounded-xl overflow-hidden animate-slide-up" style={{ animationDelay: `${gIdx * 80}ms` }}>
                          <button
                            onClick={() => toggleFolder(dir)}
                            className="w-full flex items-center gap-2 px-4 py-2.5 bg-white/[0.03] hover:bg-white/[0.06] transition-colors text-left"
                          >
                            <ChevronDown
                              size={14}
                              className={`text-accent-cyan/60 transition-transform duration-200 ${
                                isCollapsed ? '-rotate-90' : ''
                              }`}
                            />
                            <span className="text-sm font-medium text-silver-300 truncate">
                              📁 {displayDir}
                            </span>
                            <span className="text-[11px] text-silver-500 shrink-0">
                              {files.length} 个文件
                            </span>
                          </button>
                          {!isCollapsed && (
                            <div className="divide-y divide-white/[0.04]">
                              {files.map((r, fIdx) => (
                                <div
                                  key={r.file_id}
                                  className="p-3 hover:bg-white/5 transition-colors animate-slide-up"
                                  style={{ animationDelay: `${(gIdx * 80) + ((fIdx + 1) * 40)}ms` }}
                                >
                                  <div className="flex items-start justify-between">
                                    <div className="flex items-start gap-3 min-w-0 flex-1">
                                      <span className="text-lg shrink-0 mt-0.5">
                                        {getFileIcon(r.file_name)}
                                      </span>
                                      <div className="min-w-0 flex-1">
                                        <h3 className="text-sm font-medium truncate">{r.file_name}</h3>
                                        <p className="text-xs text-silver-500 truncate">{r.file_path}</p>
                                        {r.snippet && (
                                          <p className="text-xs text-silver-500/80 mt-1 line-clamp-2 italic">
                                            {r.snippet.slice(0, 200)}
                                          </p>
                                        )}
                                        <div className="flex items-center gap-3 mt-1.5">
                                          <span className="text-[11px] px-1.5 py-0.5 rounded-full bg-accent-cyan/10 text-accent-cyan/80">
                                            {r.match_type}
                                          </span>
                                          <span className="text-[11px] text-silver-500">
                                            匹配度 {(r.score * 100).toFixed(0)}%
                                          </span>
                                        </div>
                                      </div>
                                    </div>
                                    <div className="flex flex-col items-end gap-1.5 shrink-0 ml-4">
                                      <span className="text-xs text-silver-500 whitespace-nowrap">
                                        {formatSize(r.file_size)}
                                      </span>
                                      <span className="text-[11px] text-silver-600 whitespace-nowrap">
                                        {r.modified.slice(0, 10)}
                                      </span>
                                      <div className="flex gap-1 mt-1">
                                        <button
                                          onClick={() => handleOpenLocation(r.file_path)}
                                          className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                                          title="打开文件所在位置"
                                        >
                                          <FolderOpen size={14} />
                                        </button>
                                        <button
                                          onClick={() => handleOpenFile(r.file_path)}
                                          className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                                          title="打开文件"
                                        >
                                          <ExternalLink size={14} />
                                        </button>
                                      </div>
                                    </div>
                                  </div>

                                  {/* Custom tags section */}
                                  <div className="mt-2.5 pt-2 border-t border-white/5">
                                    <div className="flex items-center gap-1.5 flex-wrap min-h-[28px]">
                                      {(fileTags[r.file_id] || []).map(tag => (
                                        <span
                                          key={tag}
                                          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-accent-teal/10 text-[11px] text-accent-teal border border-accent-teal/20"
                                        >
                                          {tag}
                                          <button
                                            onClick={() => handleRemoveTag(r.file_id, tag)}
                                            className="hover:text-red-400 transition-colors"
                                          >
                                            <X size={10} />
                                          </button>
                                        </span>
                                      ))}
                                      {editingTagFileId === r.file_id ? (
                                        <div className="relative inline-flex items-center gap-1">
                                          <input
                                            type="text"
                                            value={tagInputText}
                                            onChange={e => { setTagInputText(e.target.value); setShowTagSuggestions(true); }}
                                            onKeyDown={e => {
                                              if (e.key === 'Enter') handleAddTag(r.file_id, tagInputText);
                                              if (e.key === 'Escape') { setEditingTagFileId(null); setTagInputText(''); }
                                            }}
                                            placeholder="标签名..."
                                            className="w-28 glass-light rounded px-2 py-0.5 text-[11px] text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/30 border border-white/5"
                                            autoFocus
                                          />
                                          <button
                                            onClick={() => handleAddTag(r.file_id, tagInputText)}
                                            className="p-0.5 rounded text-accent-cyan hover:bg-accent-cyan/10 transition-colors"
                                          >
                                            <Check size={12} />
                                          </button>
                                          {showTagSuggestions && (
                                            <div className="absolute left-0 top-full mt-1 z-20 glass rounded-lg p-1.5 shadow-lg border border-white/10 min-w-[130px] max-h-40 overflow-y-auto">
                                              {[
                                                ...topUserTags
                                                  .filter(([name]) => name.includes(tagInputText) && !(fileTags[r.file_id] || []).includes(name))
                                                  .slice(0, 8)
                                                  .map(([name, freq]) => (
                                                    <button
                                                      key={name}
                                                      onMouseDown={() => handleAddTag(r.file_id, name)}
                                                      className="block w-full text-left px-2 py-1 rounded text-[11px] text-silver-300 hover:bg-white/10 hover:text-white transition-colors"
                                                    >
                                                      {name}
                                                      <span className="text-silver-500 ml-1">({freq})</span>
                                                    </button>
                                                  )),
                                                ...(tagInputText.trim() && !(fileTags[r.file_id] || []).includes(tagInputText.trim()) && !topUserTags.some(([name]) => name === tagInputText.trim())
                                                  ? [(
                                                    <button
                                                      key="__new__"
                                                      onMouseDown={() => handleAddTag(r.file_id, tagInputText)}
                                                      className="block w-full text-left px-2 py-1 rounded text-[11px] text-accent-cyan hover:bg-accent-cyan/10 transition-colors border-t border-white/5 mt-1 pt-1"
                                                    >
                                                      新建"{tagInputText}"
                                                    </button>
                                                  )]
                                                  : []),
                                              ].filter(Boolean)}
                                            </div>
                                          )}
                                        </div>
                                      ) : (
                                        <button
                                          onClick={() => { setEditingTagFileId(r.file_id); setTagInputText(''); setShowTagSuggestions(true); }}
                                          className="inline-flex items-center gap-0.5 px-2 py-0.5 rounded-md text-[11px] text-silver-500 hover:text-accent-cyan hover:bg-white/5 border border-dashed border-white/10 transition-all"
                                        >
                                          <Plus size={10} /> 标签
                                        </button>
                                      )}
                                    </div>
                                  </div>
                                </div>
                              ))}
                            </div>
                          )}
                        </div>
                      )
                    })}
                  </div>
                </>
              )
            })()}
          </>
        )}

        {/* Empty search results */}
        {!scanning && !searching && isSearchMode && searchResults.length === 0 && (
          <div className="flex flex-col items-center justify-center h-64 text-silver-500">
            <Search size={40} className="mb-3 opacity-30" />
            <p className="text-sm">未找到匹配的文件</p>
          </div>
        )}

        {/* Browse mode — directory browser + tag browsing */}
        {!scanning && !searching && !isSearchMode && (
          <>
            <div className="flex items-center gap-2 mb-3">
              <button
                onClick={handleImport}
                className="glass px-3 py-1.5 rounded-lg text-xs flex items-center gap-1.5 text-accent-teal hover:bg-accent-teal/10 transition-all"
              >
                <Upload size={14} />
                从电脑导入
              </button>
              <div className="flex-1" />
              <div className="flex glass rounded-lg p-0.5">
                <button
                  onClick={() => { if (tagBrowseMode) exitTagBrowseMode() }}
                  className={`px-3 py-1 rounded-md text-xs font-medium transition-all ${
                    !tagBrowseMode
                      ? 'bg-accent-cyan/20 text-accent-cyan shadow-sm'
                      : 'text-silver-400 hover:text-white'
                  }`}
                >
                  浏览目录
                </button>
                <button
                  onClick={() => { if (!tagBrowseMode) enterTagBrowseMode() }}
                  className={`px-3 py-1 rounded-md text-xs font-medium transition-all flex items-center gap-1 ${
                    tagBrowseMode
                      ? 'bg-accent-cyan/20 text-accent-cyan shadow-sm'
                      : 'text-silver-400 hover:text-white'
                  }`}
                >
                  <Tags size={12} />
                  按标签浏览
                </button>
              </div>
            </div>

            {tagBrowseMode ? (
              /* ═══ Tag browsing mode ═══ */
              <>
                {selectedBrowseTag ? (
                  /* Selected tag → show files */
                  <>
                    <div className="flex items-center gap-2 mb-3">
                      <button
                        onClick={() => setSelectedBrowseTag(null)}
                        className="glass px-3 py-1.5 rounded-lg text-xs flex items-center gap-1.5 text-accent-cyan hover:bg-accent-cyan/10 transition-all"
                      >
                        ← 返回全部标签
                      </button>
                      <span className="text-xs text-silver-400">
                        标签: <span className="text-accent-teal font-medium">{selectedBrowseTag}</span>
                        {' — '}{tagBrowseResults.length} 个文件
                      </span>
                    </div>
                    {tagBrowseResults.length > 0 ? (
                      <div className="space-y-2">
                        {tagBrowseResults.map((r, idx) => (
                          <div key={r.file_id} className="glass rounded-xl p-3 hover:bg-white/5 transition-colors animate-slide-up" style={{ animationDelay: `${idx * 40}ms` }}>
                            <div className="flex items-start justify-between">
                              <div className="flex items-start gap-3 min-w-0 flex-1">
                                <span className="text-lg shrink-0 mt-0.5">{getFileIcon(r.file_name)}</span>
                                <div className="min-w-0 flex-1">
                                  <h3 className="text-sm font-medium truncate">{r.file_name}</h3>
                                  <p className="text-xs text-silver-500 truncate">{r.file_path}</p>
                                  <div className="flex items-center gap-3 mt-1.5">
                                    <span className="text-[11px] px-1.5 py-0.5 rounded-full bg-accent-teal/10 text-accent-teal/80">
                                      {r.match_type}
                                    </span>
                                  </div>
                                </div>
                              </div>
                              <div className="flex flex-col items-end gap-1.5 shrink-0 ml-4">
                                <span className="text-xs text-silver-500">{formatSize(r.file_size)}</span>
                                <span className="text-[11px] text-silver-600">{r.modified.slice(0, 10)}</span>
                                <div className="flex gap-1 mt-1">
                                  <button onClick={() => handleOpenLocation(r.file_path)} className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all" title="打开文件所在位置"><FolderOpen size={14} /></button>
                                  <button onClick={() => handleOpenFile(r.file_path)} className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all" title="打开文件"><ExternalLink size={14} /></button>
                                </div>
                              </div>
                            </div>
                            {/* Custom tags for tag-browse files */}
                            <div className="mt-2 pt-2 border-t border-white/5">
                              <div className="flex items-center gap-1.5 flex-wrap min-h-[28px]">
                                {(fileTags[r.file_id] || []).map(tag => (
                                  <span key={tag} className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-accent-teal/10 text-[11px] text-accent-teal border border-accent-teal/20">
                                    {tag}
                                    <button onClick={() => handleRemoveTag(r.file_id, tag)} className="hover:text-red-400 transition-colors"><X size={10} /></button>
                                  </span>
                                ))}
                                {editingTagFileId === r.file_id ? (
                                  <div className="relative inline-flex items-center gap-1">
                                    <input type="text" value={tagInputText} onChange={e => { setTagInputText(e.target.value); setShowTagSuggestions(true); }} onKeyDown={e => { if (e.key === 'Enter') handleAddTag(r.file_id, tagInputText); if (e.key === 'Escape') { setEditingTagFileId(null); setTagInputText(''); }}} placeholder="标签名..." className="w-24 glass-light rounded px-2 py-0.5 text-[11px] text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/30 border border-white/5" autoFocus />
                                    <button onClick={() => handleAddTag(r.file_id, tagInputText)} className="p-0.5 rounded text-accent-cyan hover:bg-accent-cyan/10 transition-colors"><Check size={12} /></button>
                                  </div>
                                ) : (
                                  <button onClick={() => { setEditingTagFileId(r.file_id); setTagInputText(''); setShowTagSuggestions(true); }} className="inline-flex items-center gap-0.5 px-2 py-0.5 rounded-md text-[11px] text-silver-500 hover:text-accent-cyan hover:bg-white/5 border border-dashed border-white/10 transition-all"><Plus size={10} /> 标签</button>
                                )}
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>
                    ) : (
                      <div className="flex flex-col items-center justify-center h-48 text-silver-500">
                        <Tags size={40} className="mb-3 opacity-30" />
                        <p className="text-sm">该标签下暂无文件</p>
                      </div>
                    )}
                  </>
                ) : (
                  /* Tag cloud */
                  <>
                    <p className="text-xs text-silver-500 mb-3">
                      共 {allTags.length} 个标签，点击筛选文件
                    </p>
                    {allTags.length > 0 ? (
                      <div className="glass rounded-xl p-4">
                        <div className="flex flex-wrap gap-2">
                          {allTags.map(([name, count]) => (
                            <button
                              key={name}
                              onClick={() => handleTagBrowseSelect(name)}
                              className="group relative px-3 py-1.5 rounded-lg text-sm transition-all duration-200 hover:scale-105"
                              style={{
                                backgroundColor: `rgba(0, 200, 200, ${Math.min(0.05 + (count / Math.max(...allTags.map(([,c]) => c))) * 0.2, 0.25)})`,
                                borderColor: `rgba(0, 200, 200, ${Math.min(0.1 + (count / Math.max(...allTags.map(([,c]) => c))) * 0.3, 0.4)})`,
                                borderWidth: '1px',
                                borderStyle: 'solid',
                              }}
                            >
                              <span className="text-accent-cyan/90 group-hover:text-white transition-colors">
                                {name}
                              </span>
                              <span className="ml-1.5 text-[11px] text-silver-500 group-hover:text-accent-cyan/70">
                                {count}
                              </span>
                            </button>
                          ))}
                        </div>
                      </div>
                    ) : (
                      <div className="flex flex-col items-center justify-center h-48 text-silver-500">
                        <Tags size={40} className="mb-3 opacity-30" />
                        <p className="text-sm">暂无标签</p>
                        <p className="text-xs text-silver-600 mt-1">在搜索或浏览中为文件添加标签</p>
                      </div>
                    )}
                  </>
                )}
              </>
            ) : (
              /* ═══ Directory browsing mode ═══ */
              <FileBrowser
                entries={dirEntries}
                currentPath={currentPath}
                clipboard={clipboard}
                onNavigate={handleNavigate}
                onNavigateUp={handleNavigateUp}
                onOpenFile={handleOpenFile}
                onOpenLocation={handleOpenLocation}
                onRename={handleRename}
                onDelete={handleDelete}
                onCopy={handleCopy}
                onCut={handleCut}
                onPaste={handlePaste}
                onRefresh={handleRefresh}
                onCreateDirectory={handleCreateDirectory}
                browseFileTags={browseFileTags}
                onBrowseAddTag={handleBrowseAddTag}
                onBrowseRemoveTag={handleBrowseRemoveTag}
              />
            )}
          </>
        )}
      </div>
    </div>
  )
}
