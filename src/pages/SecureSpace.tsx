import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { Shield, Lock, Eye, EyeOff, Plus, Trash2, ExternalLink, RefreshCw } from 'lucide-react'

interface VaultFile {
  id: string
  original_name: string
  original_path: string
  encrypted_name: string
  size: number
  stored_at: string
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}

export default function SecureSpace() {
  const [unlocked, setUnlocked] = useState(false)
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [files, setFiles] = useState<VaultFile[]>([])
  const [loading, setLoading] = useState(false)
  const [feedback, setFeedback] = useState('')
  const [feedbackType, setFeedbackType] = useState<'info' | 'error'>('info')
  const [configuring, setConfiguring] = useState<boolean | null>(null)
  const [confirmPassword, setConfirmPassword] = useState('')

  const showFeedback = (msg: string, type: 'info' | 'error' = 'info') => {
    setFeedback(msg)
    setFeedbackType(type)
    setTimeout(() => setFeedback(''), 5000)
  }

  useEffect(() => {
    invoke<boolean>('vault_is_configured')
      .then(configured => setConfiguring(!configured))
      .catch(() => setConfiguring(true))
  }, [])

  const handleConfigure = async () => {
    if (!password.trim() || password !== confirmPassword) {
      showFeedback('两次输入的密码不一致', 'error')
      return
    }
    if (password.length < 4) {
      showFeedback('密码至少需要4个字符', 'error')
      return
    }
    try {
      await invoke('vault_configure', { password })
      showFeedback('安全空间已创建')
      setConfiguring(false)
    } catch (err) {
      showFeedback(`创建失败: ${err}`, 'error')
    }
  }

  const handleUnlock = async () => {
    if (!password.trim()) return
    try {
      await invoke('vault_unlock', { password })
      setUnlocked(true)
      loadFiles()
    } catch (err) {
      showFeedback(`解锁失败: ${err}`, 'error')
    }
  }

  const loadFiles = async () => {
    setLoading(true)
    try {
      const result = await invoke<VaultFile[]>('vault_list_files', { password })
      setFiles(result)
    } catch (err) {
      showFeedback(`加载文件列表失败: ${err}`, 'error')
    } finally {
      setLoading(false)
    }
  }

  const handleAddFile = async () => {
    try {
      const selected = await open({ multiple: false, title: '选择要加密的文件（仅限设备内文件）' })
      if (!selected) return
      await invoke('vault_add_external_file', { path: selected, password })
      showFeedback('文件已加密添加')
      loadFiles()
    } catch (err) {
      showFeedback(`添加失败: ${err}`, 'error')
    }
  }

  const handleOpenFile = async (file: VaultFile) => {
    try {
      await invoke('vault_open_file', {
        encryptedName: file.encrypted_name,
        originalName: file.original_name,
        password,
      })
    } catch (err) {
      showFeedback(`打开失败: ${err}`, 'error')
    }
  }

  const handleDeleteFile = async (file: VaultFile) => {
    if (!confirm(`确定要从加密空间删除 "${file.original_name}" 吗？`)) return
    try {
      await invoke('vault_delete_file', {
        encryptedName: file.encrypted_name,
        password,
      })
      showFeedback(`已删除: ${file.original_name}`)
      loadFiles()
    } catch (err) {
      showFeedback(`删除失败: ${err}`, 'error')
    }
  }

  const handleLock = () => {
    setUnlocked(false)
    setPassword('')
    setFiles([])
  }

  // Loading config state
  if (configuring === null) {
    return (
      <div className="h-full flex flex-col">
        <div className="flex-1 flex items-center justify-center">
          <RefreshCw size={24} className="text-accent-cyan animate-spin" />
        </div>
      </div>
    )
  }

  // Setup form
  if (configuring && !unlocked) {
    return (
      <div className="h-full flex flex-col">
        <div className="p-6 pb-4">
          <h2 className="text-xl font-semibold mb-1">安全空间</h2>
          <p className="text-sm text-silver-400">初始化加密存储区域</p>
        </div>
        <div className="flex-1 flex items-center justify-center">
          <div className="glass rounded-2xl p-8 w-96 text-center">
            <Shield size={48} className="mx-auto mb-4 text-accent-teal opacity-60" />
            <h3 className="text-lg font-medium mb-2">创建加密空间</h3>
            <p className="text-sm text-silver-400 mb-4">设置主密码以保护您的敏感文件</p>
            {feedback && (
              <p className={`text-xs mb-3 ${feedbackType === 'error' ? 'text-red-400' : 'text-accent-teal'}`}>{feedback}</p>
            )}
            <div className="relative mb-3">
              <Lock size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-silver-400" />
              <input
                type={showPassword ? 'text' : 'password'}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="设置主密码..."
                className="w-full glass rounded-xl pl-10 pr-10 py-2.5 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-teal/50"
              />
              <button
                onClick={() => setShowPassword(!showPassword)}
                className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-silver-400 hover:text-white"
              >
                {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
              </button>
            </div>
            <div className="relative mb-4">
              <Lock size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-silver-400" />
              <input
                type={showPassword ? 'text' : 'password'}
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && handleConfigure()}
                placeholder="确认主密码..."
                className="w-full glass rounded-xl pl-10 pr-10 py-2.5 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-teal/50"
              />
            </div>
            <button
              onClick={handleConfigure}
              disabled={!password.trim() || !confirmPassword.trim()}
              className="w-full py-2.5 rounded-xl text-sm font-medium bg-accent-teal/20 text-accent-teal hover:bg-accent-teal/30 disabled:opacity-40 transition-all"
            >
              创建安全空间
            </button>
          </div>
        </div>
      </div>
    )
  }

  // Lock screen
  if (!unlocked) {
    return (
      <div className="h-full flex flex-col">
        <div className="p-6 pb-4">
          <h2 className="text-xl font-semibold mb-1">安全空间</h2>
          <p className="text-sm text-silver-400">输入密码解锁加密存储区域</p>
        </div>
        <div className="flex-1 flex items-center justify-center">
          <div className="glass rounded-2xl p-8 w-96 text-center">
            <Shield size={48} className="mx-auto mb-4 text-accent-teal opacity-60" />
            <h3 className="text-lg font-medium mb-2">加密存储区域</h3>
            <p className="text-sm text-silver-400 mb-4">此区域的内容经过加密保护，解锁后可见</p>
            {feedback && (
              <p className={`text-xs mb-3 ${feedbackType === 'error' ? 'text-red-400' : 'text-accent-teal'}`}>{feedback}</p>
            )}
            <div className="relative mb-4">
              <Lock size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-silver-400" />
              <input
                type={showPassword ? 'text' : 'password'}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && handleUnlock()}
                placeholder="输入主密码..."
                className="w-full glass rounded-xl pl-10 pr-10 py-2.5 text-sm text-white placeholder:text-silver-500 focus:outline-none focus:ring-1 focus:ring-accent-teal/50"
              />
              <button
                onClick={() => setShowPassword(!showPassword)}
                className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-silver-400 hover:text-white"
              >
                {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
              </button>
            </div>
            <button
              onClick={handleUnlock}
              disabled={!password.trim()}
              className="w-full py-2.5 rounded-xl text-sm font-medium bg-accent-teal/20 text-accent-teal hover:bg-accent-teal/30 disabled:opacity-40 transition-all"
            >
              解锁
            </button>
          </div>
        </div>
      </div>
    )
  }

  // Unlocked — file list
  return (
    <div className="h-full flex flex-col">
      <div className="p-6 pb-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold mb-1">安全空间</h2>
            <p className="text-sm text-silver-400">已解锁 — 加密保护中的文件</p>
          </div>
          <div className="flex gap-2">
            <button
              onClick={handleAddFile}
              className="glass px-3 py-2 rounded-lg text-sm flex items-center gap-2 text-accent-cyan hover:bg-accent-cyan/10 transition-all"
            >
              <Plus size={16} />
              添加文件
            </button>
            <button
              onClick={handleLock}
              className="glass px-3 py-2 rounded-lg text-sm text-silver-400 hover:text-white transition-all"
              title="锁定"
            >
              <Lock size={16} />
            </button>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-auto px-6 pb-6">
        {feedback && (
          <div className={`glass rounded-xl p-3 mb-4 text-sm border ${
            feedbackType === 'error'
              ? 'text-red-400 border-red-500/20'
              : 'text-accent-teal border-accent-teal/20'
          }`}>
            {feedback}
          </div>
        )}

        {loading && (
          <div className="flex items-center justify-center h-32">
            <RefreshCw size={24} className="text-accent-cyan animate-spin" />
          </div>
        )}

        {!loading && files.length === 0 && (
          <div className="flex flex-col items-center justify-center h-64 text-silver-500">
            <Shield size={40} className="mb-3 opacity-30" />
            <p className="text-sm">加密空间为空</p>
            <p className="text-xs mt-1 opacity-60">点击"添加文件"将敏感文件移入加密保护</p>
          </div>
        )}

        {!loading && files.length > 0 && (
          <div className="space-y-2">
            {files.map((f, idx) => (
              <div key={f.id} className="glass rounded-xl p-3 flex items-center justify-between animate-slide-up" style={{ animationDelay: `${idx * 60}ms` }}>
                <div className="flex items-center gap-3 min-w-0 flex-1">
                  <Shield size={16} className="text-accent-teal shrink-0" />
                  <div className="min-w-0">
                    <p className="text-sm truncate">{f.original_name}</p>
                    <p className="text-xs text-silver-500">{formatSize(f.size)}</p>
                  </div>
                </div>
                <div className="flex items-center gap-1 shrink-0 ml-4">
                  <button
                    onClick={() => handleOpenFile(f)}
                    className="p-1.5 rounded text-silver-400 hover:text-accent-cyan hover:bg-white/10 transition-all"
                    title="打开文件"
                  >
                    <ExternalLink size={14} />
                  </button>
                  <button
                    onClick={() => handleDeleteFile(f)}
                    className="p-1.5 rounded text-silver-400 hover:text-red-400 hover:bg-white/10 transition-all"
                    title="删除"
                  >
                    <Trash2 size={14} />
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
