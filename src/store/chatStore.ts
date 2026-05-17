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

interface ChatState {
  sessions: ChatSession[]
  currentSessionId: string | null
  messages: ChatMessage[]
  streaming: boolean
  loading: boolean
  error: string | null

  loadSessions: () => Promise<void>
  createSession: () => Promise<string>
  switchSession: (id: string) => Promise<void>
  deleteSession: (id: string) => Promise<void>
  renameSession: (id: string, title: string) => Promise<void>
  sendMessage: (content: string, attachedFiles?: { file_id: string; file_name: string; file_path: string }[]) => Promise<void>
  stopGeneration: () => Promise<void>
  clearError: () => void
}

let unlistenStream: (() => void) | null = null

export const useChatStore = create<ChatState>((set, get) => ({
  sessions: [],
  currentSessionId: null,
  messages: [],
  streaming: false,
  loading: false,
  error: null,

  clearError: () => set({ error: null }),

  stopGeneration: async () => {
    try {
      await invoke('stop_chat')
      set({ streaming: false })
    } catch (err) {
      set({ error: `停止失败: ${err}` })
    }
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

  sendMessage: async (content: string, attachedFiles?: { file_id: string; file_name: string; file_path: string }[]) => {
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

    // Clean up previous listener if any, then create a fresh one
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
        set({ streaming: false })
        get().switchSession(get().currentSessionId || event.payload.session_id)
        if (unlistenStream) { unlistenStream(); unlistenStream = null }
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
    const thinkingMsg: ChatMessage = {
      id: '__thinking__',
      session_id: sessionId,
      role: 'assistant',
      content: '思考中...',
      file_refs: null,
      created_at: now,
    }

    set((s) => ({ messages: [...s.messages, userMsg, thinkingMsg], streaming: true, error: null }))

    try {
      const fileIds = attachedFiles && attachedFiles.length > 0
        ? attachedFiles.map(f => f.file_id)
        : undefined
      const invokeArgs: Record<string, unknown> = { sessionId, message: content }
      if (fileIds && fileIds.length > 0) {
        invokeArgs.fileIds = fileIds
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
