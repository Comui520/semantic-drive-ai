import { create } from 'zustand'

export type NavPage = 'search' | 'classify' | 'organize' | 'vault'

interface AppState {
  currentPage: NavPage
  setPage: (page: NavPage) => void
}

export const useAppStore = create<AppState>((set) => ({
  currentPage: 'search',
  setPage: (page) => set({ currentPage: page }),
}))
