import { useEffect } from 'react'
import Layout from './components/Layout'
import SmartSearch from './pages/SmartSearch'
import SmartAssistant from './pages/SmartAssistant'
import FileClassify from './pages/FileClassify'
import OrganizeSuggestions from './pages/OrganizeSuggestions'
import SecureSpace from './pages/SecureSpace'
import { useAppStore } from './store/appStore'
import { useThemeStore } from './store/themeStore'

const pages = {
  chat: <SmartAssistant />,
  search: <SmartSearch />,
  classify: <FileClassify />,
  organize: <OrganizeSuggestions />,
  vault: <SecureSpace />,
}

export default function App() {
  const currentPage = useAppStore((s) => s.currentPage)
  const theme = useThemeStore((s) => s.theme)

  useEffect(() => {
    document.body.className = theme
  }, [theme])

  return <Layout><div key={currentPage} className="h-full animate-fade-in">{pages[currentPage]}</div></Layout>
}
