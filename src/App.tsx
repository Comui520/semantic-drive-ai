import { useEffect } from 'react'
import Layout from './components/Layout'
import SmartSearch from './pages/SmartSearch'
import FileClassify from './pages/FileClassify'
import OrganizeSuggestions from './pages/OrganizeSuggestions'
import SecureSpace from './pages/SecureSpace'
import { useAppStore } from './store/appStore'
import { useThemeStore } from './store/themeStore'

const pages = {
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

  return <Layout>{pages[currentPage]}</Layout>
}
