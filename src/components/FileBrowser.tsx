import { useState } from 'react'
import { Virtuoso } from 'react-virtuoso'
import { Folder, ChevronRight, Home, RotateCw, Pencil, Trash2, FolderOpen, ExternalLink, Copy, Scissors, ClipboardPaste, Plus, X, Check } from 'lucide-react'

export interface DirEntry {
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: string
}

export interface ClipboardState {
  action: 'copy' | 'cut'
  paths: string[]
}

interface FileBrowserProps {
  entries: DirEntry[]
  currentPath: string
  clipboard: ClipboardState | null
  onNavigate: (path: string) => void
  onNavigateUp: () => void
  onOpenFile: (path: string) => void
  onOpenLocation: (path: string) => void
  onRename: (path: string, currentName: string) => void
  onDelete: (path: string) => void
  onCopy: (path: string) => void
  onCut: (path: string) => void
  onPaste: () => void
  onRefresh: () => void
  onCreateDirectory?: () => void
  // Optional tag editing in browse mode — keyed by DirEntry.path
  browseFileTags?: Record<string, string[]>
  onBrowseAddTag?: (filePath: string, tag: string) => void
  onBrowseRemoveTag?: (filePath: string, tag: string) => void
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}

function getFileIcon(name: string): string {
  const e = name.split('.').pop()?.toLowerCase() || ''
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

function getBreadcrumbs(path: string): string[] {
  if (!path || path === '.') return []
  return path.split('/').filter(Boolean)
}

export default function FileBrowser({
  entries, currentPath, clipboard, onNavigate, onNavigateUp,
  onOpenFile, onOpenLocation, onRename, onDelete,
  onCopy, onCut, onPaste, onRefresh, onCreateDirectory,
  browseFileTags, onBrowseAddTag, onBrowseRemoveTag,
}: FileBrowserProps) {
  const breadcrumbs = getBreadcrumbs(currentPath)
  const [editingTagPath, setEditingTagPath] = useState<string | null>(null)
  const [tagInput, setTagInput] = useState('')

  return (
    <div>
      {/* Breadcrumb */}
      <div className="flex items-center gap-1 mb-3 text-sm">
        <button
          onClick={() => onNavigate('')}
          className="flex items-center gap-1 px-2 py-1 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/5 transition-all"
        >
          <Home size={14} />
          根目录
        </button>
        {breadcrumbs.map((part, i) => {
          const path = breadcrumbs.slice(0, i + 1).join('/')
          return (
            <span key={path} className="flex items-center gap-1">
              <ChevronRight size={14} className="text-silver-600" />
              <button
                onClick={() => onNavigate(path)}
                className="px-2 py-1 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/5 transition-all"
              >
                {part}
              </button>
            </span>
          )
        })}
        <div className="flex-1" />
        {clipboard && (
          <button
            onClick={onPaste}
            className="flex items-center gap-1 px-2 py-1.5 rounded-lg text-accent-teal bg-accent-teal/10 hover:bg-accent-teal/20 transition-all text-xs font-medium"
            title="粘贴"
          >
            <ClipboardPaste size={14} />
            粘贴 {clipboard.paths.length} 项
          </button>
        )}
        <button
          onClick={onCreateDirectory}
          className="p-1.5 rounded-lg text-silver-400 hover:text-accent-teal hover:bg-white/10 transition-all"
          title="新建文件夹"
        >
          <Plus size={14} />
        </button>
        <button
          onClick={onRefresh}
          className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
          title="刷新"
        >
          <RotateCw size={14} />
        </button>
      </div>

      {/* Parent directory link */}
      {currentPath && (
        <div
          onClick={onNavigateUp}
          className="glass rounded-xl p-3 mb-1 hover:bg-white/5 transition-colors cursor-pointer"
        >
          <div className="flex items-center gap-3">
            <Folder size={20} className="text-silver-400 shrink-0" />
            <span className="text-sm text-silver-400">..</span>
          </div>
        </div>
      )}

      {/* Virtual-scrolled entry list */}
      {entries.length > 0 ? (
        <Virtuoso
          style={{ height: 'calc(100vh - 280px)' }}
          data={entries}
          computeItemKey={(_, entry) => entry.path}
          itemContent={(_index, entry) => (
            <div className="group glass rounded-xl p-3 mb-1 hover:bg-white/5 transition-colors">
              {entry.is_dir ? (
                <div
                  onClick={() => onNavigate(entry.path)}
                  className="flex items-center gap-3 cursor-pointer"
                >
                  <Folder size={20} className="text-accent-cyan shrink-0" />
                  <span className="text-sm font-medium">{entry.name}</span>
                  <span className="text-xs text-silver-500 ml-auto">
                    {entry.modified.slice(0, 10)}
                  </span>
                </div>
              ) : (
                <div className="flex items-center justify-between">
                  <div
                    className="flex items-center gap-3 min-w-0 flex-1 cursor-pointer"
                    onClick={() => onOpenFile(entry.path)}
                  >
                    <span className="text-lg shrink-0">{getFileIcon(entry.name)}</span>
                    <div className="min-w-0 flex-1">
                      <h3 className="text-sm font-medium truncate">{entry.name}</h3>
                      <p className="text-xs text-silver-500 truncate">{entry.path}</p>
                    </div>
                  </div>
                  <div className="flex items-center gap-2 shrink-0 ml-4">
                    <span className="text-xs text-silver-500">{formatSize(entry.size)}</span>
                    <span className="text-xs text-silver-600">{entry.modified.slice(0, 10)}</span>
                    <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                      <button
                        onClick={() => onOpenLocation(entry.path)}
                        className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                        title="打开文件所在位置"
                      >
                        <FolderOpen size={14} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); onOpenFile(entry.path); }}
                        className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                        title="打开文件"
                      >
                        <ExternalLink size={14} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); onCopy(entry.path); }}
                        className="p-1.5 rounded-lg text-silver-400 hover:text-accent-teal hover:bg-white/10 transition-all"
                        title="复制"
                      >
                        <Copy size={14} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); onCut(entry.path); }}
                        className="p-1.5 rounded-lg text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                        title="剪切"
                      >
                        <Scissors size={14} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); onRename(entry.path, entry.name); }}
                        className="p-1.5 rounded-lg text-silver-400 hover:text-accent-teal hover:bg-white/10 transition-all"
                        title="重命名"
                      >
                        <Pencil size={14} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); onDelete(entry.path); }}
                        className="p-1.5 rounded-lg text-silver-400 hover:text-red-400 hover:bg-white/10 transition-all"
                        title="删除"
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                </div>
              )}

              {/* Custom tags for browse mode */}
              {!entry.is_dir && browseFileTags && onBrowseAddTag && onBrowseRemoveTag && (
                <div className="mt-2 pt-2 border-t border-white/5">
                  <div className="flex items-center gap-1.5 flex-wrap min-h-[28px]">
                    {(browseFileTags[entry.path] || []).map(tag => (
                      <span
                        key={tag}
                        className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-accent-teal/10 text-[11px] text-accent-teal border border-accent-teal/20"
                      >
                        {tag}
                        <button
                          onClick={(e) => { e.stopPropagation(); onBrowseRemoveTag(entry.path, tag); }}
                          className="hover:text-red-400 transition-colors"
                        >
                          <X size={10} />
                        </button>
                      </span>
                    ))}
                    {editingTagPath === entry.path ? (
                      <div className="relative inline-flex items-center gap-1">
                        <input
                          type="text"
                          value={tagInput}
                          onChange={e => setTagInput(e.target.value)}
                          onKeyDown={e => {
                            if (e.key === 'Enter' && tagInput.trim()) {
                              onBrowseAddTag(entry.path, tagInput.trim());
                              setEditingTagPath(null);
                              setTagInput('');
                            }
                            if (e.key === 'Escape') {
                              setEditingTagPath(null);
                              setTagInput('');
                            }
                          }}
                          placeholder="标签名..."
                          className="w-24 glass-light rounded px-2 py-0.5 text-[11px] text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/30 border border-white/5"
                          autoFocus
                        />
                        <button
                          onClick={() => {
                            if (tagInput.trim()) {
                              onBrowseAddTag(entry.path, tagInput.trim());
                              setEditingTagPath(null);
                              setTagInput('');
                            }
                          }}
                          className="p-0.5 rounded text-accent-cyan hover:bg-accent-cyan/10 transition-colors"
                        >
                          <Check size={12} />
                        </button>
                      </div>
                    ) : (
                      <button
                        onClick={(e) => { e.stopPropagation(); setEditingTagPath(entry.path); setTagInput(''); }}
                        className="inline-flex items-center gap-0.5 px-2 py-0.5 rounded-md text-[11px] text-silver-500 hover:text-accent-cyan hover:bg-white/5 border border-dashed border-white/10 transition-all"
                      >
                        <Plus size={10} /> 标签
                      </button>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}
        />
      ) : (
        <div className="flex flex-col items-center justify-center h-48 text-silver-500">
          <Folder size={40} className="mb-3 opacity-30" />
          <p className="text-sm">此文件夹为空</p>
        </div>
      )}
    </div>
  )
}
