export const STORAGE_KEY = 'semanticdrive_welcome_seen'

export function hasSeenWelcome(): boolean {
  return localStorage.getItem(STORAGE_KEY) === 'true'
}
