import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'


export interface FileRef {
  file_id: string
  file_name: string
  file_path: string
  snippet: string
}

export interface ChatMessage {
  id: string
  session_id: string
  role: string
  content: string
  file_refs: FileRef[] | null
  created_at: string
}

export interface ChatSession {
  id: string
  title: string
  created_at: string
  updated_at: string
  message_count: number
}

export interface ChatTokenEvent {
  session_id: string
  token: string
  done: boolean
}

// ── AI File Action types ──

export type FileAction =
  | { cmd: 'rename_file'; params: { old_path: string; new_name: string } }
  | { cmd: 'delete_file'; params: { file_path: string } }
  | { cmd: 'move_file'; params: { source: string; destination: string } }
  | { cmd: 'copy_file'; params: { source: string; destination: string } }
  | { cmd: 'import_file'; params: { source: string; destination: string } }
  | { cmd: 'vault_add_file'; params: { file_path: string; password: string } }
  | { cmd: 'set_file_tags'; params: { file_id: string; tags: string[] } }
  // ── ByFileId variants ──
  | { cmd: 'move_file_by_id'; params: { file_id: string; destination: string } }
  | { cmd: 'copy_file_by_id'; params: { file_id: string; destination: string } }
  | { cmd: 'delete_file_by_id'; params: { file_id: string } }
  | { cmd: 'rename_file_by_id'; params: { file_id: string; new_name: string } }
  | { cmd: 'vault_add_file_by_id'; params: { file_id: string; password: string } }

export interface ChatActionsEvent {
  session_id: string
  actions: FileAction[]
}

function describeAction(action: FileAction): string {
  switch (action.cmd) {
    case 'rename_file': return `重命名「${action.params.old_path}」→「${action.params.new_name}」`
    case 'delete_file': return `删除「${action.params.file_path}」`
    case 'move_file': return `移动「${action.params.source}」→「${action.params.destination}」`
    case 'copy_file': return `复制「${action.params.source}」→「${action.params.destination}」`
    case 'import_file': return `导入「${action.params.source}」→「${action.params.destination}」`
    case 'vault_add_file': return `加密添加到安全空间: 「${action.params.file_path}」`
    case 'set_file_tags': return `设置标签「${action.params.tags.join('、')}」到文件`
    case 'move_file_by_id': return `移动文件（ID: ${action.params.file_id}）→「${action.params.destination}」`
    case 'copy_file_by_id': return `复制文件（ID: ${action.params.file_id}）→「${action.params.destination}」`
    case 'delete_file_by_id': return `删除文件（ID: ${action.params.file_id}）`
    case 'rename_file_by_id': return `重命名文件（ID: ${action.params.file_id}）→「${action.params.new_name}」`
    case 'vault_add_file_by_id': return `加密添加到安全空间（ID: ${action.params.file_id}）`
  }
}

interface ChatState {
  sessions: ChatSession[]
  currentSessionId: string | null
  messages: ChatMessage[]
  streaming: boolean
  loading: boolean
  error: string | null
  pendingActions: FileAction[]
  actionResults: string[]

  loadSessions: () => Promise<void>
  createSession: () => Promise<string>
  switchSession: (id: string) => Promise<void>
  deleteSession: (id: string) => Promise<void>
  renameSession: (id: string, title: string) => Promise<void>
  sendMessage: (content: string, attachedFiles?: { file_id: string; file_name: string; file_path: string }[], folderPaths?: string[]) => Promise<void>
  stopGeneration: () => Promise<void>
  clearError: () => void
  confirmActions: () => Promise<void>
  rejectActions: () => void
}

let unlistenStream: (() => void) | null = null
let unlistenActions: (() => void) | null = null

export const useChatStore = create<ChatState>((set, get) => ({
  sessions: [],
  currentSessionId: null,
  messages: [],
  streaming: false,
  loading: false,
  error: null,
  pendingActions: [],
  actionResults: [],

  clearError: () => set({ error: null }),

  stopGeneration: async () => {
    try {
      await invoke('stop_chat')
      set({ streaming: false })
    } catch (err) {
      set({ error: `停止失败: ${err}` })
    }
  },

  confirmActions: async () => {
    const { pendingActions } = get()
    if (pendingActions.length === 0) return

    const results: string[] = []
    for (const action of pendingActions) {
      try {
        const result = await invoke<string>('execute_file_action', { actionJson: JSON.stringify(action) })
        results.push(result)
      } catch (err) {
        results.push(`操作失败: ${describeAction(action)} — ${err}`)
      }
    }

    set({ pendingActions: [], actionResults: results })
    // Clear results after 8 seconds
    setTimeout(() => set({ actionResults: [] }), 8000)
  },

  rejectActions: () => {
    set({ pendingActions: [] })
  },

  loadSessions: async () => {
    try {
      const sessions = await invoke<ChatSession[]>('list_chat_sessions')
      set({ sessions })
    } catch (err) {
      set({ error: `加载会话失败: ${err}` })
    }
  },

  createSession: async () => {
    try {
      const session = await invoke<ChatSession>('create_chat_session', { title: null })
      set((s) => ({ sessions: [session, ...s.sessions], currentSessionId: session.id, messages: [] }))
      return session.id
    } catch (err) {
      set({ error: `创建会话失败: ${err}` })
      return ''
    }
  },

  switchSession: async (id: string) => {
    set({ currentSessionId: id, loading: true, error: null })
    try {
      const messages = await invoke<ChatMessage[]>('get_chat_messages', { sessionId: id })
      set({ messages, loading: false })
    } catch (err) {
      set({ error: `加载消息失败: ${err}`, loading: false })
    }
  },

  deleteSession: async (id: string) => {
    try {
      await invoke('delete_chat_session', { sessionId: id })
      const { sessions, currentSessionId } = get()
      const filtered = sessions.filter((s) => s.id !== id)
      const nextState: Partial<ChatState> = { sessions: filtered }
      if (currentSessionId === id) {
        nextState.currentSessionId = null
        nextState.messages = []
      }
      set(nextState)
    } catch (err) {
      set({ error: `删除会话失败: ${err}` })
    }
  },

  renameSession: async (id: string, title: string) => {
    try {
      await invoke('rename_chat_session', { sessionId: id, title })
      set((s) => ({
        sessions: s.sessions.map((sess) =>
          sess.id === id ? { ...sess, title } : sess
        ),
      }))
    } catch (err) {
      set({ error: `重命名失败: ${err}` })
    }
  },

  sendMessage: async (content: string, attachedFiles?: { file_id: string; file_name: string; file_path: string }[], folderPaths?: string[]) => {
    const { currentSessionId } = get()
    let sessionId = currentSessionId

    // Auto-create session if none selected
    if (!sessionId) {
      try {
        const session = await invoke<ChatSession>('create_chat_session', { title: null })
        sessionId = session.id
        set((s) => ({ sessions: [session, ...s.sessions], currentSessionId: session.id }))
      } catch (err) {
        set({ error: `创建会话失败: ${err}` })
        return
      }
    }

    // ── Setup event listeners (inside try/catch so failures are visible) ──
    try {
      if (unlistenStream) { unlistenStream(); unlistenStream = null }

      // Token buffer for 80ms batched state updates
      let buffer = ''
      let flushTimer: ReturnType<typeof setTimeout> | null = null
      let streamSessionId = ''

      const flushBuffer = () => {
        flushTimer = null
        if (!buffer) return
        const content = buffer
        buffer = ''
        const state = get()
        const msgs = [...state.messages]
        const last = msgs[msgs.length - 1]
        if (last && last.id === '__thinking__') {
          msgs[msgs.length - 1] = {
            id: '__streaming__',
            session_id: streamSessionId,
            role: 'assistant',
            content,
            file_refs: null,
            created_at: new Date().toISOString(),
          }
        } else if (last && last.role === 'assistant') {
          msgs[msgs.length - 1] = { ...last, content: last.content + content }
        } else {
          msgs.push({
            id: '__streaming__',
            session_id: streamSessionId,
            role: 'assistant',
            content,
            file_refs: null,
            created_at: new Date().toISOString(),
          })
        }
        set({ messages: msgs })
      }

      unlistenStream = await listen<ChatTokenEvent>('chat-token', (event) => {
        if (event.payload.done) {
          if (flushTimer) { clearTimeout(flushTimer); flushTimer = null }
          flushBuffer()
          if (event.payload.token && event.payload.token.startsWith('生成失败:')) {
            set({ streaming: false, error: event.payload.token })
          } else {
            set({ streaming: false })
            get().switchSession(get().currentSessionId || event.payload.session_id)
          }
          if (unlistenStream) { unlistenStream(); unlistenStream = null }
          if (unlistenActions) { unlistenActions(); unlistenActions = null }
        } else {
          if (!streamSessionId) streamSessionId = event.payload.session_id
          const state = get()
          const last = state.messages[state.messages.length - 1]
          if (last && last.id === '__thinking__') {
            // First token — flush immediately to replace thinking indicator
            buffer = event.payload.token
            flushBuffer()
          } else {
            buffer += event.payload.token
            if (!flushTimer) {
              flushTimer = setTimeout(flushBuffer, 80)
            }
          }
        }
      })

      // Listen for AI-suggested file actions
      if (unlistenActions) { unlistenActions(); unlistenActions = null }
      unlistenActions = await listen<ChatActionsEvent>('chat-actions', (event) => {
        set({ pendingActions: event.payload.actions, actionResults: [] })
      })
    } catch (err) {
      set({ streaming: false, error: `消息发送失败: ${err}` })
      return
    }

    // Build optimistic file_refs for user message display
    const optimisticFileRefs = attachedFiles && attachedFiles.length > 0
      ? attachedFiles.map(f => ({ ...f, snippet: '' }))
      : null

    // Add user message optimistically
    const now = new Date().toISOString()
    const userMsg: ChatMessage = {
      id: '__optimistic__',
      session_id: sessionId,
      role: 'user',
      content,
      file_refs: optimisticFileRefs,
      created_at: now,
    }

    // Add thinking indicator so user sees immediate feedback during RAG phase
    const hasFiles = attachedFiles && attachedFiles.length > 0
    const thinkingMsg: ChatMessage = {
      id: '__thinking__',
      session_id: sessionId,
      role: 'assistant',
      content: hasFiles ? '正在分析文件并生成回答...' : '思考中...',
      file_refs: null,
      created_at: now,
    }

    set((s) => ({ messages: [...s.messages, userMsg, thinkingMsg], streaming: true, error: null }))

    try {
      const fileIds = attachedFiles && attachedFiles.length > 0
        ? attachedFiles.filter(f => !f.file_id.startsWith('__folder__') && !f.file_id.includes('/')).map(f => f.file_id)
        : undefined
      const invokeArgs: Record<string, unknown> = { sessionId, message: content }
      if (fileIds && fileIds.length > 0) {
        invokeArgs.fileIds = fileIds
      }
      if (folderPaths && folderPaths.length > 0) {
        invokeArgs.folderPaths = folderPaths
      }
      await invoke('chat_send', invokeArgs)
      // Reload sessions to update timestamp
      const updated = await invoke<ChatSession[]>('list_chat_sessions')
      set({ sessions: updated })
    } catch (err) {
      set({ streaming: false, error: `发送失败: ${err}` })
    }
  },
}))
