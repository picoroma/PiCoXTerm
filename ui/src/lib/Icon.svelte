<script lang="ts">
  export type IconName =
    | 'arrow-left'
    | 'arrow-right'
    | 'parent'
    | 'refresh'
    | 'lock'
    | 'unlock'
    | 'eye'
    | 'eye-off'
    | 'arrows-h'
    | 'upload'
    | 'download'
    | 'folder-up'
    | 'folder-down'
    | 'folder-plus'
    | 'plus'
    | 'info'
    | 'terminal'
    | 'target'
    | 'columns'
    | 'folder-transfer'
    | 'swap'
    | 'cloud'
    | 'monitor'
    | 'monitor-cursor'

  let { name, size = 16 }: { name: IconName; size?: number } = $props()

  const FOLDER =
    'M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z'
  const TRAY = 'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4'
  const PADLOCK = 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z'
  const MONITOR = 'M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z'
  const EYE = 'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z'

  // Icone a tratto singolo (stile coerente: 24x24, tratto 2, estremità arrotondate).
  const paths: Record<IconName, string[]> = {
    'arrow-left': ['M19 12H5', 'M12 19l-7-7 7-7'],
    'arrow-right': ['M5 12h14', 'M12 5l7 7-7 7'],
    // Cartella superiore: freccia ad angolo, ben diversa dalla freccia di caricamento.
    parent: ['M14 9L9 4 4 9', 'M20 20h-7a4 4 0 0 1-4-4V4'],
    refresh: [
      'M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8',
      'M21 3v5h-5',
      'M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16',
      'M8 16H3v5',
    ],
    lock: ['M7 11V7a5 5 0 0 1 10 0v4', PADLOCK],
    unlock: ['M7 11V7a5 5 0 0 1 9.9-1', PADLOCK],
    eye: [EYE, 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z'],
    'eye-off': [EYE, 'M3 3l18 18'],
    'arrows-h': ['M8 7l-5 5 5 5', 'M16 7l5 5-5 5', 'M3 12h18'],
    upload: [TRAY, 'M17 8l-5-5-5 5', 'M12 3v12'],
    download: [TRAY, 'M7 10l5 5 5-5', 'M12 15V3'],
    'folder-up': [FOLDER, 'M12 17v-6', 'M9 14l3-3 3 3'],
    'folder-down': [FOLDER, 'M12 11v6', 'M9 14l3 3 3-3'],
    'folder-plus': [FOLDER, 'M12 11v6', 'M9 14h6'],
    plus: ['M12 5v14', 'M5 12h14'],
    info: ['M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z', 'M12 16v-4', 'M12 8h.01'],
    terminal: ['M4 17l6-6-6-6', 'M12 19h8'],
    columns: ['M3 5h18v14H3z', 'M9 5v14', 'M15 5v14'],
    target: ['M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z', 'M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8z', 'M12 12h.01'],
    'folder-transfer': [FOLDER, 'M8 14h8', 'M13 11l3 3-3 3'],
    swap: ['M7 10h14', 'M17 6l4 4-4 4', 'M17 18H3', 'M7 14l-4 4 4 4'],
    cloud: ['M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9z'],
    monitor: [MONITOR, 'M8 21h8', 'M12 17v4'],
    'monitor-cursor': [MONITOR, 'M8 21h8', 'M12 17v4', 'M9 7l5 3-2.2.7-.9 2.1z'],
  }
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  {#each paths[name] as d (d)}
    <path {d} />
  {/each}
</svg>

<style>
  svg {
    display: block;
    flex: none;
  }
</style>
