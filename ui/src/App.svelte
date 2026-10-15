<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'

  let info = $state('...')
  $effect(() => {
    invoke<string>('app_info').then((v) => (info = v))
  })
</script>

<main>
  <h1>{info}</h1>
  <p>Scheletro M0: frontend Svelte 5 collegato al backend Rust.</p>
</main>
