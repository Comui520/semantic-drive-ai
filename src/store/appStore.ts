import { create } from 'zustand'

export type NavPage = 'search' | 'classify' | 'organize' | 'vault' | 'chat'

interface AppState {
  currentPage: NavPage
  setPage: (page: NavPage) => void
  searchQuery: string | null
  setSearchQuery: (query: string | null) => void
}

export const useAppStore = create<AppState>((set) => ({
  currentPage: 'search',
  setPage: (page) => set({ currentPage: page }),
  searchQuery: null,
  setSearchQuery: (query) => set({ searchQuery: query }),
}))
