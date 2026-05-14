import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Search, Mic, FolderOpen, RefreshCw, HardDrive } from 'lucide-react'

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

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}

function getFileIcon(ext: string): string {
  const icons: Record<string, string> = {
    pdf: '📄', doc: '📝', docx: '📝', xls: '📊', xlsx: '📊',
    ppt: '📽️', pptx: '📽️', txt: '📃', jpg: '🖼️', jpeg: '🖼️',
    png: '🖼️', gif: '🖼️', mp3: '🎵', wav: '🎵', mp4: '🎬',
    avi: '🎬', zip: '📦', rar: '📦', '7z': '📦',
  }
  return icons[ext] || '📁'
}

export default function SmartSearch() {
  const [query, setQuery] = useState('')
  const [searching, setSearching] = useState(false)
  const [files, setFiles] = useState<FileEntry[]>([])
  const [filteredFiles, setFilteredFiles] = useState<FileEntry[]>([])
  const [fileCount, setFileCount] = useState(0)
  const [scanning, setScanning] = useState(false)
  const [deviceRoot, setDeviceRoot] = useState('')

  // Load device root and file count on mount
  useEffect(() => {
    invoke<string>('get_device_root')
      .then(setDeviceRoot)
      .catch(() => setDeviceRoot('未知'))
    invoke<number>('get_file_count')
      .then(setFileCount)
      .catch(() => setFileCount(0))
  }, [])

  // Client-side filtering when query changes
  useEffect(() => {
    if (!query.trim()) {
      setFilteredFiles(files.slice(0, 50))
      return
    }
    const q = query.toLowerCase()
    setFilteredFiles(
      files.filter(
        (f) =>
          f.name.toLowerCase().includes(q) ||
          f.path.toLowerCase().includes(q) ||
          f.extension.toLowerCase().includes(q)
      ).slice(0, 50)
    )
  }, [query, files])

  const handleScan = useCallback(async () => {
    setScanning(true)
    try {
      const entries = await invoke<FileEntry[]>('scan_files')
      setFiles(entries)
      setFileCount(entries.length)
      await invoke<string>('get_device_root').then(setDeviceRoot)
    } catch (err) {
      console.error('Scan failed:', err)
    } finally {
      setScanning(false)
    }
  }, [])

  const handleSearch = () => {
    if (!query.trim()) {
      // If empty, load all files
      handleLoadFiles()
      return
    }
    setSearching(true)
    // For now, client-side filtering. Full semantic search in Phase 4.
    setTimeout(() => setSearching(false), 300)
  }

  const handleLoadFiles = useCallback(async () => {
    try {
      const entries = await invoke<FileEntry[]>('get_files')
      setFiles(entries)
      setFilteredFiles(entries.slice(0, 50))
    } catch (err) {
      console.error('Load files failed:', err)
    }
  }, [])

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
          用自然语言描述你想找的文件
        </p>

        <div className="flex gap-3">
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
              placeholder="描述你要找的文件..."
              className="w-full glass rounded-xl pl-10 pr-4 py-3 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/50"
            />
          </div>
          <button
            onClick={handleScan}
            disabled={scanning}
            className="glass px-4 py-3 rounded-xl text-sm font-medium text-accent-teal hover:bg-accent-teal/10 disabled:opacity-40 transition-all flex items-center gap-2"
          >
            <RefreshCw size={16} className={scanning ? 'animate-spin' : ''} />
            {scanning ? '扫描中...' : '扫描'}
          </button>
          <button
            onClick={handleSearch}
            disabled={searching}
            className="glass px-5 py-3 rounded-xl text-sm font-medium text-accent-cyan hover:bg-accent-cyan/10 disabled:opacity-40 transition-all"
          >
            {searching ? '搜索中...' : '搜索'}
          </button>
          <button className="glass p-3 rounded-xl text-silver-400 hover:text-accent-cyan transition-colors">
            <Mic size={18} />
          </button>
        </div>
      </div>

      {/* Results */}
      <div className="flex-1 overflow-auto px-6 pb-6">
        {files.length === 0 && !scanning && (
          <div className="flex flex-col items-center justify-center h-64 text-silver-500">
            <Search size={40} className="mb-3 opacity-30" />
            <p className="text-sm">点击"扫描"按钮开始索引设备上的文件</p>
            <p className="text-xs mt-1 opacity-60">
              扫描范围仅限于当前存储设备
            </p>
          </div>
        )}

        {scanning && (
          <div className="flex flex-col items-center justify-center h-64 gap-3">
            <RefreshCw size={32} className="text-accent-cyan animate-spin" />
            <p className="text-sm text-silver-400">正在扫描文件...</p>
          </div>
        )}

        {filteredFiles.length > 0 && (
          <p className="text-xs text-silver-500 mb-3">
            {query.trim()
              ? `找到 ${filteredFiles.length} 个匹配文件`
              : `显示 ${filteredFiles.length} 个文件`}
          </p>
        )}

        <div className="space-y-2">
          {filteredFiles.map((f) => (
            <div
              key={f.id}
              className="glass rounded-xl p-3 hover:bg-white/5 transition-colors cursor-pointer"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3 min-w-0">
                  <span className="text-lg shrink-0">{getFileIcon(f.extension)}</span>
                  <div className="min-w-0">
                    <h3 className="text-sm font-medium truncate">{f.name}</h3>
                    <p className="text-xs text-silver-500 truncate">{f.path}</p>
                  </div>
                </div>
                <div className="flex items-center gap-3 shrink-0 ml-4">
                  <span className="text-xs text-silver-500">{formatSize(f.size)}</span>
                  <span className="text-xs text-silver-600">{f.modified.slice(0, 10)}</span>
                  <button className="p-1.5 rounded-lg text-silver-400 hover:text-white hover:bg-white/5">
                    <FolderOpen size={14} />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
