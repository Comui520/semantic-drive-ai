import { useState } from 'react'
import { Shield, Lock, Eye, EyeOff, Plus } from 'lucide-react'

export default function SecureSpace() {
  const [unlocked, setUnlocked] = useState(false)
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)

  const handleUnlock = () => {
    if (password.trim()) {
      setUnlocked(true)
    }
  }

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
            <p className="text-sm text-silver-400 mb-4">
              此区域的内容经过加密保护，解锁后可见
            </p>

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

  return (
    <div className="h-full flex flex-col">
      <div className="p-6 pb-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold mb-1">安全空间</h2>
            <p className="text-sm text-silver-400">已解锁 — 加密保护中的文件</p>
          </div>
          <div className="flex gap-2">
            <button className="glass px-3 py-2 rounded-lg text-sm flex items-center gap-2 text-accent-cyan hover:bg-accent-cyan/10 transition-all">
              <Plus size={16} />
              添加文件
            </button>
            <button
              onClick={() => setUnlocked(false)}
              className="glass px-3 py-2 rounded-lg text-sm text-silver-400 hover:text-white transition-all"
            >
              <Lock size={16} />
            </button>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-auto px-6 pb-6">
        <div className="flex flex-col items-center justify-center h-64 text-silver-500">
          <Shield size={40} className="mb-3 opacity-30" />
          <p className="text-sm">加密空间为空</p>
          <p className="text-xs mt-1 opacity-60">点击"添加文件"将敏感文件移入加密保护</p>
        </div>
      </div>
    </div>
  )
}
