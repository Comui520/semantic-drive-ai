// ===== API-first settings types =====
export interface ApiEndpointConfig {
  base_url: string
  api_key: string
  model: string
  enabled: boolean
  timeout_secs: number
}

export interface AppConfig {
  ai_mode: 'api'
  embedding_api: ApiEndpointConfig
  chat_api: ApiEndpointConfig
  scan_root?: string | null
}
