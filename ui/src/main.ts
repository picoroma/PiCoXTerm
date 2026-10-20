import { mount } from 'svelte'
import './app.css'
import App from './App.svelte'
import DetachedView from './lib/DetachedView.svelte'
import { currentDetached } from './lib/detach'

// Fuori da Tauri (solo `npm run dev` in un browser) si usa un backend finto.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
  await import('./dev/mock-tauri')
}

// Una scheda staccata gira in una finestra propria, che il backend avvia con i dati della scheda.
const detached = currentDetached()
const target = document.getElementById('app')!
const app = detached ? mount(DetachedView, { target, props: { info: detached } }) : mount(App, { target })

export default app
