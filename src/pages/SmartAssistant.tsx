import { useState, useEffect, useRef, useCallback } from 'react'
import { useChatStore, type ChatMessage, type ChatSession, type FileAction } from '../store/chatStore'
import { invoke } from '@tauri-apps/api/core'
import MarkdownRenderer from '../components/MarkdownRenderer'
import { useAppStore } from '../store/appStore'
import {
  MessageSquare, Plus, Trash2, Edit2, Check, X, Send, Bot, User, Loader2,
  FolderOpen, ExternalLink, Paperclip, Search,
} from 'lucide-react'

// ── Helpers ──

interface FileEntry {
  id: string; path: string; name: string; extension: string;
  mime_type: string; size: number; hash: string | null;
  modified: string; created: string; indexed_at: string;
}

const EXT_ICONS: Record<string, string> = {
  pdf: '📄', doc: '📝', docx: '📝', xls: '📊', xlsx: '📊',
  ppt: '📽️', pptx: '📽️', txt: '📃', csv: '📃',
  jpg: '🖼️', jpeg: '🖼️', png: '🖼️', gif: '🖼️', webp: '🖼️',
  mp4: '🎬', avi: '🎬', mkv: '🎬', mov: '🎬',
  mp3: '🎵', wav: '🎵', flac: '🎵',
  zip: '📦', rar: '📦', '7z': '📦', tar: '📦', gz: '📦',
  js: '⚙️', ts: '⚙️', py: '⚙️', rs: '⚙️', java: '⚙️', cpp: '⚙️', c: '⚙️',
  html: '🌐', css: '🌐', json: '📋', xml: '📋', yaml: '📋', toml: '📋',
  md: '📝',
}

function getFileIcon(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase() || ''
  return EXT_ICONS[ext] || '📁'
}

// ── ChatSidebar ──

function ChatSidebar({
  sessions, currentSessionId, onSelect, onCreate, onDelete, onRename,
}: {
  sessions: ChatSession[]
  currentSessionId: string | null
  onSelect: (id: string) => void
  onCreate: () => void
  onDelete: (id: string) => void
  onRename: (id: string, title: string) => void
}) {
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editTitle, setEditTitle] = useState('')

  return (
    <aside className="w-56 shrink-0 h-full bg-surface border-r border-subtle flex flex-col">
      <div className="p-3 border-b border-subtle">
        <button
          onClick={onCreate}
          className="w-full flex items-center justify-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-accent-cyan bg-accent-cyan/10 hover:bg-accent-cyan/20 transition-all"
        >
          <Plus size={16} />
          新建对话
        </button>
      </div>
      <div className="flex-1 overflow-y-auto py-1">
        {sessions.length === 0 && (
          <p className="text-xs text-center text-silver-500 mt-8">暂无对话</p>
        )}
        {sessions.map((s) => (
          <div
            key={s.id}
            className={`group flex items-center gap-1 px-2 py-2 mx-2 rounded-lg cursor-pointer transition-all ${
              s.id === currentSessionId
                ? 'bg-accent-cyan/15 text-white'
                : 'text-silver-400 hover:bg-hover hover:text-silver-200'
            }`}
            onClick={() => onSelect(s.id)}
          >
            <MessageSquare size={14} className="shrink-0" />
            {editingId === s.id ? (
              <div className="flex-1 flex items-center gap-1">
                <input
                  value={editTitle}
                  onChange={(e) => setEditTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') { onRename(s.id, editTitle.trim() || s.title); setEditingId(null) }
                    if (e.key === 'Escape') setEditingId(null)
                  }}
                  className="flex-1 bg-white/10 rounded px-1.5 py-0.5 text-xs text-white outline-none focus:ring-1 focus:ring-accent-cyan/50"
                  autoFocus
                  onClick={(e) => e.stopPropagation()}
                />
                <Check size={12} className="shrink-0 text-accent-teal cursor-pointer" onClick={(e) => { e.stopPropagation(); onRename(s.id, editTitle.trim() || s.title); setEditingId(null) }} />
                <X size={12} className="shrink-0 text-silver-500 cursor-pointer" onClick={(e) => { e.stopPropagation(); setEditingId(null) }} />
              </div>
            ) : (
              <>
                <span className="flex-1 truncate text-xs">{s.title}</span>
                <div className="hidden group-hover:flex items-center gap-0.5 shrink-0">
                  <button
                    onClick={(e) => { e.stopPropagation(); setEditingId(s.id); setEditTitle(s.title) }}
                    className="p-0.5 rounded text-silver-500 hover:text-accent-cyan hover:bg-white/10"
                  >
                    <Edit2 size={12} />
                  </button>
                  <button
                    onClick={(e) => { e.stopPropagation(); if (confirm('确定删除此对话？')) onDelete(s.id) }}
                    className="p-0.5 rounded text-silver-500 hover:text-red-400 hover:bg-white/10"
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              </>
            )}
          </div>
        ))}
      </div>
    </aside>
  )
}

// ── ChatMessageBubble ──

function ChatMessageBubble({ msg }: { msg: ChatMessage }) {
  const isUser = msg.role === 'user'
  const isStreaming = msg.id === '__streaming__'
  const isThinking = msg.id === '__thinking__'

  const handleOpenLocation = useCallback((filePath: string) => {
    invoke('open_file_location', { filePath }).catch(console.error)
  }, [])

  const handleOpenFile = useCallback((filePath: string) => {
    invoke('open_file', { filePath }).catch(console.error)
  }, [])

  const handleSearchView = useCallback((filePath: string) => {
    useAppStore.getState().setPage('search')
    useAppStore.getState().setSearchQuery(filePath)
  }, [])

  return (
    <div className={`flex gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
      <div className={`w-8 h-8 rounded-full flex items-center justify-center shrink-0 ${
        isUser ? 'bg-accent-cyan/20' : 'bg-accent-teal/20'
      }`}>
        {isUser ? <User size={16} className="text-accent-cyan" /> : <Bot size={16} className="text-accent-teal" />}
      </div>
      <div className={`max-w-[75%] ${isUser ? 'items-end' : 'items-start'}`}>
        <div className={`rounded-xl px-4 py-2.5 text-sm leading-relaxed whitespace-pre-wrap ${
          isUser
            ? 'bg-accent-cyan/15 text-white rounded-tr-sm'
            : 'glass text-silver-200 rounded-tl-sm'
        }`}>
          {isThinking ? (
            <span className="flex items-center gap-2 text-silver-400">
              <span className="inline-block w-1.5 h-4 bg-accent-teal/60 rounded-full animate-pulse" />
              思考中...
            </span>
          ) : (
            <>
              {isUser ? (
                <>{msg.content}</>
              ) : (
                <MarkdownRenderer content={msg.content} />
              )}
              {isStreaming && <span className="inline-block w-1.5 h-4 bg-accent-cyan ml-0.5 animate-pulse" />}
            </>
          )}
        </div>
        {msg.file_refs && msg.file_refs.length > 0 && (
          <div className="mt-2 space-y-1.5 border-t border-white/10 pt-2">
            {msg.file_refs.map((ref) => (
              <div key={ref.file_id} className="flex items-start gap-2 bg-white/5 rounded-lg p-2">
                <span className="text-base shrink-0 mt-0.5">{getFileIcon(ref.file_name)}</span>
                <div className="min-w-0 flex-1">
                  <p className="text-xs font-medium text-silver-200 truncate">{ref.file_name}</p>
                  <p className="text-[11px] text-silver-500 truncate">{ref.file_path}</p>
                  {ref.snippet && (
                    <p className="text-[11px] text-silver-500/70 mt-0.5 line-clamp-1 italic">
                      {ref.snippet.slice(0, 150)}
                    </p>
                  )}
                </div>
                <div className="flex gap-0.5 shrink-0">
                  <button
                    onClick={() => handleOpenLocation(ref.file_path)}
                    className="p-1 rounded text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                    title="打开文件所在位置"
                  >
                    <FolderOpen size={12} />
                  </button>
                  <button
                    onClick={() => handleOpenFile(ref.file_path)}
                    className="p-1 rounded text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                    title="打开文件"
                  >
                    <ExternalLink size={12} />
                  </button>
                  <button
                    onClick={() => handleSearchView(ref.file_path)}
                    className="p-1 rounded text-silver-400 hover:text-accent-teal hover:bg-white/10 transition-all"
                    title="在搜索中查看"
                  >
                    <Search size={12} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

// ── FilePickerModal ──

function FilePickerModal({
  files, query, onQueryChange, onSelect, onClose,
}: {
  files: FileEntry[]
  query: string
  onQueryChange: (q: string) => void
  onSelect: (selected: { file_id: string; file_name: string; file_path: string }[]) => void
  onClose: () => void
}) {
  const [selected, setSelected] = useState<Set<string>>(new Set())

  const filtered = files.filter(f =>
    f.name.toLowerCase().includes(query.toLowerCase()) ||
    f.path.toLowerCase().includes(query.toLowerCase())
  )

  const toggleFile = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id); else next.add(id)
      return next
    })
  }

  const handleSelect = () => {
    const result = files.filter(f => selected.has(f.id)).map(f => ({
      file_id: f.id, file_name: f.name, file_path: f.path,
    }))
    onSelect(result)
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 animate-fade-in">
      <div className="bg-surface border border-subtle rounded-xl w-[480px] max-h-[520px] flex flex-col shadow-2xl animate-slide-up" style={{ animationDelay: '50ms' }}>
        <div className="p-3 border-b border-subtle">
          <input
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            placeholder="搜索文件..."
            className="w-full glass rounded-lg px-3 py-2 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/50"
            autoFocus
          />
        </div>
        <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {filtered.map((f) => (
            <button
              key={f.id}
              onClick={() => toggleFile(f.id)}
              className={`w-full flex items-center gap-2 px-3 py-2 rounded-lg text-left transition-all ${
                selected.has(f.id)
                  ? 'bg-accent-cyan/15 text-white'
                  : 'text-silver-300 hover:bg-white/5'
              }`}
            >
              <div className={`w-4 h-4 rounded border-2 flex items-center justify-center shrink-0 ${
                selected.has(f.id) ? 'border-accent-cyan bg-accent-cyan' : 'border-silver-500'
              }`}>
                {selected.has(f.id) && <Check size={10} className="text-white" />}
              </div>
              <span className="text-sm truncate flex-1">{f.name}</span>
              <span className="text-[11px] text-silver-500 shrink-0 truncate max-w-[180px]">{f.path}</span>
            </button>
          ))}
          {filtered.length === 0 && (
            <p className="text-xs text-center text-silver-500 py-8">无匹配文件</p>
          )}
        </div>
        <div className="p-3 border-t border-subtle flex justify-end gap-2">
          <button onClick={onClose} className="px-3 py-1.5 rounded-lg text-sm text-silver-400 hover:text-white hover:bg-white/10 transition-all">取消</button>
          <button
            onClick={handleSelect}
            disabled={selected.size === 0}
            className="px-3 py-1.5 rounded-lg text-sm bg-accent-cyan/20 text-accent-cyan hover:bg-accent-cyan/30 disabled:opacity-30 transition-all"
          >
            附加 ({selected.size})
          </button>
        </div>
      </div>
    </div>
  )
}

// ── ChatInput ──

function ChatInput({ onSend, onStop, streaming }: {
  onSend: (msg: string, attachedFiles?: { file_id: string; file_name: string; file_path: string }[]) => void
  onStop: () => void
  streaming: boolean
}) {
  const [text, setText] = useState('')
  const [attachedFiles, setAttachedFiles] = useState<{ file_id: string; file_name: string; file_path: string }[]>([])
  const [showFilePicker, setShowFilePicker] = useState(false)
  const [allFiles, setAllFiles] = useState<FileEntry[]>([])
  const [filePickerQuery, setFilePickerQuery] = useState('')
  const inputRef = useRef<HTMLTextAreaElement>(null)

  const handleSend = useCallback(() => {
    const trimmed = text.trim()
    if ((!trimmed && attachedFiles.length === 0) || streaming) return
    onSend(trimmed || '(文件附件)', attachedFiles.length > 0 ? attachedFiles : undefined)
    setText('')
    setAttachedFiles([])
  }, [text, streaming, onSend, attachedFiles])

  useEffect(() => {
    if (!streaming && inputRef.current) inputRef.current.focus()
  }, [streaming])

  const handleOpenFilePicker = useCallback(async () => {
    try {
      const files = await invoke<FileEntry[]>('get_files')
      setAllFiles(files)
      setShowFilePicker(true)
    } catch (err) {
      console.error('Failed to load files:', err)
    }
  }, [])

  const handleSelectFiles = useCallback((selected: { file_id: string; file_name: string; file_path: string }[]) => {
    setAttachedFiles((prev) => {
      const existing = new Set(prev.map(f => f.file_id))
      const newFiles = selected.filter(f => !existing.has(f.file_id))
      return [...prev, ...newFiles]
    })
    setShowFilePicker(false)
    setFilePickerQuery('')
  }, [])

  const handleRemoveFile = useCallback((fileId: string) => {
    setAttachedFiles((prev) => prev.filter(f => f.file_id !== fileId))
  }, [])

  return (
    <div className="flex flex-col gap-2">
      {attachedFiles.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {attachedFiles.map(f => (
            <span
              key={f.file_id}
              className="inline-flex items-center gap-1 px-2 py-1 rounded-md bg-accent-cyan/10 text-[11px] text-accent-cyan border border-accent-cyan/20"
            >
              <span>{getFileIcon(f.file_name)}</span>
              <span className="max-w-[120px] truncate">{f.file_name}</span>
              <button onClick={() => handleRemoveFile(f.file_id)} className="hover:text-red-400 transition-colors">
                <X size={10} />
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="flex items-end gap-2">
        <button
          onClick={handleOpenFilePicker}
          className="p-3 rounded-xl text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
          title="附加文件"
        >
          <Paperclip size={18} />
        </button>
        <textarea
          ref={inputRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
          }}
          placeholder="输入消息，Enter 发送，Shift+Enter 换行..."
          rows={1}
          className="flex-1 glass rounded-xl px-4 py-3 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-cyan/50 resize-none max-h-32"
          style={{ minHeight: '44px' }}
        />
        {streaming ? (
          <button
            onClick={onStop}
            className="p-3 rounded-xl bg-red-500/20 text-red-400 hover:bg-red-500/30 transition-all"
            title="停止生成"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
              <rect x="6" y="6" width="12" height="12" rx="2" />
            </svg>
          </button>
        ) : (
          <button
            onClick={handleSend}
            disabled={!text.trim() && attachedFiles.length === 0}
            className="p-3 rounded-xl bg-accent-cyan/20 text-accent-cyan hover:bg-accent-cyan/30 disabled:opacity-30 disabled:cursor-not-allowed transition-all"
          >
            <Send size={18} />
          </button>
        )}
      </div>
      {showFilePicker && (
        <FilePickerModal
          files={allFiles}
          query={filePickerQuery}
          onQueryChange={setFilePickerQuery}
          onSelect={handleSelectFiles}
          onClose={() => { setShowFilePicker(false); setFilePickerQuery('') }}
        />
      )}
    </div>
  )
}

// ── ActionConfirmBar ──

function describeAction(action: FileAction): string {
  switch (action.cmd) {
    case 'rename_file': return `重命名「${action.params.old_path}」→「${action.params.new_name}」`
    case 'delete_file': return `删除「${action.params.file_path}」`
    case 'move_file': return `移动「${action.params.source}」→「${action.params.destination}」`
    case 'copy_file': return `复制「${action.params.source}」→「${action.params.destination}」`
    case 'import_file': return `导入「${action.params.source}」→「${action.params.destination}」`
    case 'vault_add_file': return `加密添加到安全空间: 「${action.params.file_path}」`
    case 'set_file_tags': return `设置标签「${action.params.tags.join('、')}」到文件`
  }
}

function ActionConfirmBar({
  actions, results, onConfirm, onReject,
}: {
  actions: FileAction[]
  results: string[]
  onConfirm: () => void
  onReject: () => void
}) {
  if (actions.length === 0 && results.length === 0) return null

  if (results.length > 0) {
    return (
      <div className="px-4 pb-2">
        <div className="glass rounded-xl p-3 border border-accent-teal/20 animate-fade-in">
          <div className="flex items-center gap-2 text-sm text-accent-teal mb-1">
            <Check size={14} />
            <span className="font-medium">操作已执行</span>
          </div>
          <div className="space-y-0.5">
            {results.map((r, i) => (
              <p key={i} className="text-xs text-silver-400">{r}</p>
            ))}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="px-4 pb-2">
      <div className="glass rounded-xl p-3 border border-accent-cyan/20 animate-slide-up">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-sm">
            <Bot size={14} className="text-accent-cyan" />
            <span className="text-silver-200">AI 建议执行以下操作：</span>
          </div>
          <div className="flex gap-2">
            <button
              onClick={onReject}
              className="px-2.5 py-1 rounded-lg text-xs text-silver-400 hover:text-white hover:bg-white/10 transition-all"
            >
              拒绝
            </button>
            <button
              onClick={onConfirm}
              className="px-2.5 py-1 rounded-lg text-xs font-medium bg-accent-cyan/20 text-accent-cyan hover:bg-accent-cyan/30 transition-all"
            >
              确认执行
            </button>
          </div>
        </div>
        <div className="mt-2 space-y-0.5">
          {actions.map((a, i) => (
            <p key={i} className="text-xs text-silver-400">· {describeAction(a)}</p>
          ))}
        </div>
      </div>
    </div>
  )
}

// ── Main SmartAssistant Page ──

export default function SmartAssistant() {
  const {
    sessions, currentSessionId, messages, streaming, loading, error,
    pendingActions, actionResults,
    loadSessions, createSession, switchSession, deleteSession, renameSession,
    sendMessage, stopGeneration, clearError,
    confirmActions, rejectActions,
  } = useChatStore()

  const messagesEndRef = useRef<HTMLDivElement>(null)

  // Load sessions on mount
  useEffect(() => { loadSessions() }, [loadSessions])

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  const handleCreate = useCallback(async () => {
    await createSession()
  }, [createSession])

  // Welcome screen when no session selected
  const showWelcome = !currentSessionId && sessions.length === 0

  return (
    <div className="h-full flex">
      <ChatSidebar
        sessions={sessions}
        currentSessionId={currentSessionId}
        onSelect={switchSession}
        onCreate={handleCreate}
        onDelete={deleteSession}
        onRename={renameSession}
      />

      {/* Main chat area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Error banner — always visible */}
        {error && (
          <div className="px-4 pt-4">
            <div className="glass rounded-xl p-3 text-sm text-red-400 border border-red-500/20 flex items-center gap-2">
              <span className="flex-1">{error}</span>
              <button onClick={clearError} className="p-0.5 hover:text-white shrink-0">
                <X size={14} />
              </button>
            </div>
          </div>
        )}

        {showWelcome ? (
          <div className="flex-1 flex flex-col items-center justify-center gap-4">
            <div className="w-16 h-16 rounded-full bg-accent-cyan/15 flex items-center justify-center">
              <Bot size={32} className="text-accent-cyan" />
            </div>
            <h2 className="text-lg font-semibold text-primary">Semantic Drive AI 智能助手</h2>
            <p className="text-sm text-muted max-w-md text-center">
              我可以帮助你搜索文件、总结内容、回答问题。<br />
              点击左侧「新建对话」开始，或从已有对话继续。
            </p>
            <div className="flex flex-wrap gap-2 mt-2">
              {[
                { label: '帮我找一下上周的Excel文件', hint: '搜索文件' },
                { label: '总结这个文档的内容', hint: '总结内容' },
                { label: '我可以做什么？', hint: '一般对话' },
              ].map((item) => (
                <button
                  key={item.label}
                  onClick={() => sendMessage(item.label)}
                  className="glass px-3 py-2 rounded-lg text-xs text-silver-300 hover:text-white hover:bg-white/5 transition-all"
                >
                  {item.label}
                </button>
              ))}
            </div>
          </div>
        ) : (
          <>
            {/* Messages area */}
            <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4">
              {currentSessionId && messages.length === 0 && !loading && (
                <div className="flex flex-col items-center justify-center h-48 text-silver-500">
                  <MessageSquare size={32} className="opacity-30 mb-2" />
                  <p className="text-sm">开始对话吧</p>
                </div>
              )}

              {loading && (
                <div className="flex items-center justify-center py-8">
                  <Loader2 size={24} className="text-accent-cyan animate-spin" />
                </div>
              )}

              {messages.map((msg, idx) => (
                <div key={msg.id} className="animate-slide-up" style={{ animationDelay: `${Math.min(idx, 10) * 60}ms` }}>
                  <ChatMessageBubble msg={msg} />
                </div>
              ))}
              <div ref={messagesEndRef} />
            </div>

            {/* AI action confirmation bar */}
            {(pendingActions.length > 0 || actionResults.length > 0) && (
              <ActionConfirmBar
                actions={pendingActions}
                results={actionResults}
                onConfirm={confirmActions}
                onReject={rejectActions}
              />
            )}

            {/* Input area */}
            <div className="px-4 py-3 border-t border-subtle">
              <ChatInput onSend={sendMessage} onStop={stopGeneration} streaming={streaming} />
            </div>
          </>
        )}
      </div>
    </div>
  )
}
