import { create } from 'zustand'

interface SettingsState {
  sidebarOpen: boolean
  bilingualSearch: boolean
  toggleSidebar: () => void
  setBilingualSearch: (val: boolean) => void
}

export const useSettingsStore = create<SettingsState>((set) => ({
  sidebarOpen: false,
  bilingualSearch: (localStorage.getItem('sd-bilingual') ?? 'true') === 'true',
  toggleSidebar: () =>
    set((s) => ({ sidebarOpen: !s.sidebarOpen })),
  setBilingualSearch: (val) => {
    localStorage.setItem('sd-bilingual', val ? 'true' : 'false')
    set({ bilingualSearch: val })
  },
}))
