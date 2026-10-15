import { invoke } from '@tauri-apps/api/core'
import type { ThemeName } from './themes'

export interface Settings {
  schemaVersion: number
  theme: ThemeName
  fontFamily: string
  fontSize: number
  scrollback: number
  copyOnSelect: boolean
  rightClickPaste: boolean
  pasteMultilineWarning: boolean
}

export const defaultSettings: Settings = {
  schemaVersion: 1,
  theme: 'dark',
  fontFamily: 'Consolas, "DejaVu Sans Mono", monospace',
  fontSize: 14,
  scrollback: 10000,
  copyOnSelect: true,
  rightClickPaste: true,
  pasteMultilineWarning: true,
}

/** Impostazioni correnti, condivise da tutta la UI. */
export const settings: Settings = $state({ ...defaultSettings })

/** Il tema "light" cambia anche l'interfaccia, gli altri restano scuri. */
function applyChrome() {
  document.documentElement.dataset.theme = settings.theme === 'light' ? 'light' : 'dark'
}

export async function loadSettings() {
  Object.assign(settings, await invoke<Settings>('settings_get'))
  applyChrome()
}

export async function saveSettings(next: Settings) {
  Object.assign(settings, await invoke<Settings>('settings_set', { settings: next }))
  applyChrome()
}
