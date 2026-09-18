import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { Eye, EyeOff, Save, TestTube2, CloudCog, ShieldCheck, FolderOpen } from 'lucide-react'
import { useSettingsStore } from '../store/settingsStore'
import type { AppConfig } from '../types'

type Result = { type: 'success' | 'error'; text: string }

export default function Settings() {
  const store = useSettingsStore()
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState<'embedding' | 'chat' | null>(null)
  const [result, setResult] = useState<Result | null>(null)
  const [showEmbeddingKey, setShowEmbeddingKey] = useState(false)
  const [showChatKey, setShowChatKey] = useState(false)

  useEffect(() => {
    void invoke<AppConfig>('get_config')
      .then((config) => useSettingsStore.getState().loadConfigFromBackend(config))
      .catch((error) => setResult({ type: 'error', text: `无法读取设置：${String(error)}` }))
  }, [])

  const save = async () => {
    setSaving(true)
    setResult(null)
    try {
      await invoke('update_config', { config: store.buildConfigForBackend() })
      setResult({ type: 'success', text: '云端 AI 设置已保存。' })
    } catch (error) {
      setResult({ type: 'error', text: `保存失败：${String(error)}` })
    } finally {
      setSaving(false)
    }
  }

  const test = async (service: 'embedding' | 'chat') => {
    setTesting(service)
    setResult(null)
    try {
      await invoke('update_config', { config: store.buildConfigForBackend() })
      const command = service === 'embedding' ? 'test_embedding_connection' : 'test_chat_connection'
      const response = await invoke<string>(command)
      setResult({ type: 'success', text: response })
    } catch (error) {
      setResult({ type: 'error', text: `连接测试失败：${String(error)}` })
    } finally {
      setTesting(null)
    }
  }

  const selectWorkspace = async () => {
    const selected = await open({ directory: true, multiple: false, title: '选择 Semantic Drive 工作区' })
    if (typeof selected !== 'string') return
    try {
      const canonical = await invoke<string>('set_scan_root', { path: selected })
      store.setScanRoot(canonical)
      setResult({ type: 'success', text: '工作区已切换。建议重新扫描以更新索引。' })
    } catch (error) {
      setResult({ type: 'error', text: `切换工作区失败：${String(error)}` })
    }
  }

  const clearWorkspace = async () => {
    try {
      const fallback = await invoke<string>('set_scan_root', { path: null })
      store.setScanRoot(fallback)
      setResult({ type: 'success', text: '已恢复便携式默认目录。' })
    } catch (error) {
      setResult({ type: 'error', text: `恢复失败：${String(error)}` })
    }
  }

  return (
    <div className="max-w-3xl mx-auto space-y-6 animate-fade-in p-4 md:p-6">
      <header>
        <div className="flex items-center gap-2 text-accent-cyan"><CloudCog size={20} /><span className="text-sm font-medium">API-first</span></div>
        <h2 className="mt-2 text-2xl font-semibold text-primary">云端 AI 设置</h2>
        <p className="mt-2 text-sm text-secondary">Semantic Drive 使用 OpenAI 兼容接口完成语义检索和 Agent 对话；未配置嵌入 API 时，搜索会自动使用轻量本地关键词回退。</p>
      </header>

      <section className="glass rounded-xl p-5 space-y-4">
        <div><h3 className="text-base font-medium text-primary">嵌入 API</h3><p className="text-xs text-secondary mt-1">用于高质量语义搜索与相似文件检索。</p></div>
        <EndpointFields
          baseUrl={store.embeddingApiBaseUrl} model={store.embeddingApiModel} apiKey={store.embeddingApiKey}
          showKey={showEmbeddingKey} onToggleKey={() => setShowEmbeddingKey((value) => !value)}
          onBaseUrl={store.setEmbeddingApiBaseUrl} onModel={store.setEmbeddingApiModel} onApiKey={store.setEmbeddingApiKey}
        />
        <button onClick={() => void test('embedding')} disabled={testing !== null} className="secondary-btn text-xs inline-flex items-center gap-2">
          <TestTube2 size={14} />{testing === 'embedding' ? '测试中…' : '测试嵌入连接'}
        </button>
      </section>

      <section className="glass rounded-xl p-5 space-y-4">
        <div><h3 className="text-base font-medium text-primary">聊天 / Agent API</h3><p className="text-xs text-secondary mt-1">用于文件问答、计划生成和经确认后执行的文件操作。</p></div>
        <EndpointFields
          baseUrl={store.chatApiBaseUrl} model={store.chatApiModel} apiKey={store.chatApiKey}
          showKey={showChatKey} onToggleKey={() => setShowChatKey((value) => !value)}
          onBaseUrl={store.setChatApiBaseUrl} onModel={store.setChatApiModel} onApiKey={store.setChatApiKey}
        />
        <button onClick={() => void test('chat')} disabled={testing !== null} className="secondary-btn text-xs inline-flex items-center gap-2">
          <TestTube2 size={14} />{testing === 'chat' ? '测试中…' : '测试聊天连接'}
        </button>
      </section>

      <section className="glass rounded-xl p-5 space-y-3">
        <div><h3 className="text-base font-medium text-primary">工作区</h3><p className="text-xs text-secondary mt-1">扫描、搜索和 Agent 操作只会发生在这个目录内。默认仍兼容便携式启动方式。</p></div>
        <div className="flex gap-2">
          <input value={store.scanRoot} readOnly className="input-field flex-1" placeholder="尚未选择工作区" />
          <button onClick={() => void selectWorkspace()} className="secondary-btn inline-flex items-center gap-2"><FolderOpen size={15} />选择目录</button>
        </div>
        {store.scanRoot && <button onClick={() => void clearWorkspace()} className="text-xs text-secondary hover:text-primary">恢复便携式默认目录</button>}
      </section>

      <section className="rounded-xl border border-accent-cyan/20 bg-accent-cyan/5 p-4 flex gap-3">
        <ShieldCheck size={20} className="text-accent-cyan shrink-0 mt-0.5" />
        <p className="text-xs leading-5 text-secondary">Agent 只能在已扫描目录内处理带有 <code>file_id</code> 的文件。移动、复制、重命名、删除和加密都会先显示待执行操作，必须由你逐项或批量确认。</p>
      </section>

      {result && <p className={`rounded-lg px-4 py-3 text-sm ${result.type === 'success' ? 'bg-emerald-500/10 text-emerald-300' : 'bg-red-500/10 text-red-300'}`}>{result.text}</p>}
      <button onClick={() => void save()} disabled={saving} className="primary-btn flex items-center gap-2"><Save size={16} />{saving ? '保存中…' : '保存设置'}</button>
    </div>
  )
}

function EndpointFields({ baseUrl, model, apiKey, showKey, onToggleKey, onBaseUrl, onModel, onApiKey }: {
  baseUrl: string; model: string; apiKey: string; showKey: boolean; onToggleKey: () => void
  onBaseUrl: (value: string) => void; onModel: (value: string) => void; onApiKey: (value: string) => void
}) {
  return <div className="grid gap-3 sm:grid-cols-2">
    <label className="text-xs text-secondary">Base URL<input value={baseUrl} onChange={(event) => onBaseUrl(event.target.value)} className="input-field mt-1" placeholder="https://api.example.com/v1" /></label>
    <label className="text-xs text-secondary">模型<input value={model} onChange={(event) => onModel(event.target.value)} className="input-field mt-1" placeholder="模型名称" /></label>
    <label className="text-xs text-secondary sm:col-span-2">API Key
      <span className="relative block mt-1"><input value={apiKey} type={showKey ? 'text' : 'password'} onChange={(event) => onApiKey(event.target.value)} className="input-field pr-10" placeholder="sk-…" />
      <button type="button" onClick={onToggleKey} className="absolute right-2 top-1/2 -translate-y-1/2 text-secondary hover:text-primary" aria-label="显示或隐藏 API Key">{showKey ? <EyeOff size={16} /> : <Eye size={16} />}</button></span>
    </label>
  </div>
}
