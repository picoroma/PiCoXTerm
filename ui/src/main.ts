import { mount } from 'svelte'
import './app.css'
import App from './App.svelte'

// Fuori da Tauri (solo `npm run dev` in un browser) si usa un backend finto.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
  await import('./dev/mock-tauri')
}

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
