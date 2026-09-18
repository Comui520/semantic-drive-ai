import { create } from 'zustand'
import type { AppConfig } from '../types'

interface SettingsState {
  sidebarOpen: boolean
  bilingualSearch: boolean
  toggleSidebar: () => void
  setBilingualSearch: (val: boolean) => void

  embeddingApiEnabled: boolean
  embeddingApiKey: string
  embeddingApiModel: string
  embeddingApiBaseUrl: string
  setEmbeddingApiEnabled: (enabled: boolean) => void
  setEmbeddingApiKey: (key: string) => void
  setEmbeddingApiModel: (model: string) => void
  setEmbeddingApiBaseUrl: (url: string) => void

  chatApiEnabled: boolean
  chatApiKey: string
  chatApiModel: string
  chatApiBaseUrl: string
  scanRoot: string
  setScanRoot: (root: string) => void
  setChatApiEnabled: (enabled: boolean) => void
  setChatApiKey: (key: string) => void
  setChatApiModel: (model: string) => void
  setChatApiBaseUrl: (url: string) => void

  loadConfigFromBackend: (config: AppConfig) => void
  buildConfigForBackend: () => AppConfig
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  sidebarOpen: false,
  bilingualSearch: (localStorage.getItem('sd-bilingual') ?? 'true') === 'true',
  toggleSidebar: () => set((state) => ({ sidebarOpen: !state.sidebarOpen })),
  setBilingualSearch: (val) => {
    localStorage.setItem('sd-bilingual', val ? 'true' : 'false')
    set({ bilingualSearch: val })
  },

  embeddingApiEnabled: true,
  embeddingApiKey: '',
  embeddingApiModel: 'BAAI/bge-m3',
  embeddingApiBaseUrl: 'https://api.siliconflow.cn/v1',
  setEmbeddingApiEnabled: (enabled) => set({ embeddingApiEnabled: enabled }),
  setEmbeddingApiKey: (key) => set({ embeddingApiKey: key }),
  setEmbeddingApiModel: (model) => set({ embeddingApiModel: model }),
  setEmbeddingApiBaseUrl: (url) => set({ embeddingApiBaseUrl: url }),

  chatApiEnabled: true,
  chatApiKey: '',
  chatApiModel: 'deepseek-chat',
  chatApiBaseUrl: 'https://api.deepseek.com/v1',
  scanRoot: '',
  setChatApiEnabled: (enabled) => set({ chatApiEnabled: enabled }),
  setChatApiKey: (key) => set({ chatApiKey: key }),
  setChatApiModel: (model) => set({ chatApiModel: model }),
  setChatApiBaseUrl: (url) => set({ chatApiBaseUrl: url }),
  setScanRoot: (root) => set({ scanRoot: root }),

  loadConfigFromBackend: (config) => set({
    embeddingApiEnabled: config.embedding_api.enabled,
    embeddingApiKey: config.embedding_api.api_key,
    embeddingApiModel: config.embedding_api.model,
    embeddingApiBaseUrl: config.embedding_api.base_url,
    chatApiEnabled: config.chat_api.enabled,
    chatApiKey: config.chat_api.api_key,
    chatApiModel: config.chat_api.model,
    chatApiBaseUrl: config.chat_api.base_url,
    scanRoot: config.scan_root ?? '',
  }),
  buildConfigForBackend: () => {
    const settings = get()
    return {
      ai_mode: 'api',
      embedding_api: { base_url: settings.embeddingApiBaseUrl, api_key: settings.embeddingApiKey, model: settings.embeddingApiModel, enabled: settings.embeddingApiEnabled, timeout_secs: 30 },
      chat_api: { base_url: settings.chatApiBaseUrl, api_key: settings.chatApiKey, model: settings.chatApiModel, enabled: settings.chatApiEnabled, timeout_secs: 60 },
      scan_root: settings.scanRoot || null,
    }
  },
}))
