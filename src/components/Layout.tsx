import { Search, FolderTree, Sparkles, Shield, Sun, Moon, Menu, X, MessageSquare, Settings } from 'lucide-react'
import { useAppStore, type NavPage } from '../store/appStore'
import { useThemeStore } from '../store/themeStore'
import { useSettingsStore } from '../store/settingsStore'

const navItems: { id: NavPage; label: string; icon: React.ReactNode }[] = [
  { id: 'chat', label: '智能助手', icon: <MessageSquare size={20} /> },
  { id: 'search', label: '智能搜索', icon: <Search size={20} /> },
  { id: 'classify', label: '文件分类', icon: <FolderTree size={20} /> },
  { id: 'organize', label: '整理建议', icon: <Sparkles size={20} /> },
  { id: 'vault', label: '安全空间', icon: <Shield size={20} /> },
  { id: 'settings', label: '设置', icon: <Settings size={20} /> },
]

export default function Layout({ children }: { children: React.ReactNode }) {
  const { currentPage, setPage } = useAppStore()
  const { theme, toggle } = useThemeStore()

  const { sidebarOpen, bilingualSearch, toggleSidebar, setBilingualSearch } = useSettingsStore()

  return (
    <div className="flex h-full">
      {/* Sidebar */}
      <aside className="bg-surface flex w-60 flex-col shrink-0 h-full border-r border-subtle">
        <div className="px-5 py-6 border-b border-subtle">
          <div className="flex items-center justify-between">
            <div>
              <h1 className="text-sm font-semibold tracking-wider text-accent-cyan uppercase">
                Semantic Drive
              </h1>
              <p className="text-xs text-muted mt-1">语义智能文件管家</p>
            </div>
            <button
              onClick={toggleSidebar}
              className="p-1.5 rounded-lg text-secondary hover:text-primary hover:bg-hover transition-all"
              title="设置"
            >
              <Menu size={18} />
            </button>
          </div>
        </div>

        <nav className="flex-1 py-4 px-3 space-y-1 overflow-y-auto">
          {navItems.map((item) => (
            <button
              key={item.id}
              onClick={() => setPage(item.id)}
              className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition-all duration-200 border-l-2 ${
                currentPage === item.id
                  ? 'bg-deepsea-500/60 text-white shadow-sm border-accent-cyan'
                  : 'text-secondary hover:text-primary hover:bg-hover border-transparent'
              }`}
            >
              {item.icon}
              {item.label}
            </button>
          ))}
        </nav>

        <div className="px-3 py-4 border-t border-subtle">
          <button
            onClick={toggle}
            className="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm text-secondary hover:text-primary hover:bg-hover transition-all duration-200"
          >
            {theme === 'dark' ? <Sun size={18} /> : <Moon size={18} />}
            {theme === 'dark' ? '浅色模式' : '暗夜模式'}
          </button>
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-auto relative">{children}

        {/* Settings sidebar overlay */}
        {sidebarOpen && (
          <div
            className="fixed inset-0 bg-black/40 z-40 transition-opacity"
            onClick={toggleSidebar}
          />
        )}

        <div
          className={`fixed top-0 right-0 h-full w-72 bg-surface border-l border-subtle z-50 shadow-2xl transform transition-transform duration-300 ${
            sidebarOpen ? 'translate-x-0' : 'translate-x-full'
          }`}
        >
          <div className="flex items-center justify-between px-5 py-5 border-b border-subtle">
            <h2 className="text-sm font-semibold text-primary">⚙️ 设置</h2>
            <button
              onClick={toggleSidebar}
              className="p-1.5 rounded-lg text-secondary hover:text-primary hover:bg-hover transition-all"
            >
              <X size={16} />
            </button>
          </div>

          <div className="px-5 py-6 space-y-6">
            {/* Bilingual search toggle */}
            <div className="flex items-center justify-between">
              <div>
                <div className="text-sm font-medium text-primary">双语搜索</div>
                <div className="text-xs text-muted mt-1">
                  开启后搜索时同时匹配中英文内容
                </div>
              </div>
              <button
                onClick={() => setBilingualSearch(!bilingualSearch)}
                className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${
                  bilingualSearch ? 'bg-accent-cyan' : 'bg-hover'
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white transition-transform duration-200 ${
                    bilingualSearch ? 'translate-x-5' : 'translate-x-0'
                  }`}
                />
              </button>
            </div>
          </div>
        </div>
      </main>
    </div>
  )
}
