import { useEffect, useState, lazy, Suspense } from 'react'
import Layout from './components/Layout'
import WelcomeGuide from './components/WelcomeGuide'
import { hasSeenWelcome } from './components/welcomeStorage'
import { useAppStore } from './store/appStore'
import { useThemeStore } from './store/themeStore'

const SmartSearch = lazy(() => import('./pages/SmartSearch'))
const SmartAssistant = lazy(() => import('./pages/SmartAssistant'))
const FileClassify = lazy(() => import('./pages/FileClassify'))
const OrganizeSuggestions = lazy(() => import('./pages/OrganizeSuggestions'))
const SecureSpace = lazy(() => import('./pages/SecureSpace'))
const Settings = lazy(() => import('./pages/Settings'))

const PageLoader = () => (
  <div className="flex items-center justify-center h-64 text-secondary">
    <div className="animate-pulse">加载中...</div>
  </div>
)

const pages: Record<string, React.ReactNode> = {
  chat: <Suspense fallback={<PageLoader />}><SmartAssistant /></Suspense>,
  search: <Suspense fallback={<PageLoader />}><SmartSearch /></Suspense>,
  classify: <Suspense fallback={<PageLoader />}><FileClassify /></Suspense>,
  organize: <Suspense fallback={<PageLoader />}><OrganizeSuggestions /></Suspense>,
  vault: <Suspense fallback={<PageLoader />}><SecureSpace /></Suspense>,
  settings: <Suspense fallback={<PageLoader />}><Settings /></Suspense>,
}

export default function App() {
  const currentPage = useAppStore((s) => s.currentPage)
  const theme = useThemeStore((s) => s.theme)
  const [showWelcome, setShowWelcome] = useState(!hasSeenWelcome())

  useEffect(() => {
    document.body.className = theme
  }, [theme])

  return (
    <>
      {showWelcome && <WelcomeGuide onComplete={() => setShowWelcome(false)} />}
      <Layout>
        <div key={currentPage} className="h-full animate-fade-in">
          {pages[currentPage] ?? pages.chat}
        </div>
      </Layout>
    </>
  )
}
