<script lang="ts">
  import { t } from '../i18n'
  import { subtreeIds, type Folder, type Session, type SessionNode, type Uuid } from './types'

  let {
    nodes,
    onopen,
    onedit,
    onnewsession,
    onnewfolder,
    onrename,
    ondelete,
    onmove,
  }: {
    nodes: SessionNode[]
    onopen: (s: Session) => void
    onedit: (s: Session) => void
    onnewsession: (parentId: Uuid | null) => void
    onnewfolder: (parentId: Uuid | null) => void
    onrename: (f: Folder) => void
    ondelete: (n: SessionNode) => void
    onmove: (id: Uuid, parentId: Uuid | null) => void
  } = $props()

  interface Row {
    node: SessionNode
    depth: number
    expanded: boolean
  }

  let filter = $state('')
  let collapsed = $state(new Set<Uuid>())
  let selected = $state<Uuid | null>(null)
  let dragId = $state<Uuid | null>(null)
  let dropTarget = $state<Uuid | 'root' | null>(null)
  let menu = $state<{ x: number; y: number; node: SessionNode | null } | null>(null)

  function matches(n: SessionNode, q: string): boolean {
    if (n.type === 'folder') return n.name.toLowerCase().includes(q)
    return [n.name, n.host, n.username, ...n.ui.tags].some((v) => v.toLowerCase().includes(q))
  }

  const rows = $derived.by<Row[]>(() => {
    const q = filter.trim().toLowerCase()
    const byParent = new Map<Uuid | null, SessionNode[]>()
    const byId = new Map<Uuid, SessionNode>()
    for (const n of nodes) {
      byId.set(n.id, n)
      const list = byParent.get(n.parentId) ?? []
      list.push(n)
      byParent.set(n.parentId, list)
    }
    for (const list of byParent.values()) {
      list.sort(
        (a, b) =>
          (a.type === b.type ? 0 : a.type === 'folder' ? -1 : 1) || a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
      )
    }
    // Con il filtro attivo restano visibili i nodi che corrispondono e i loro antenati.
    let visible: Set<Uuid> | null = null
    if (q) {
      visible = new Set()
      for (const n of nodes) {
        if (n.type === 'session' && matches(n, q)) {
          let cur: SessionNode | undefined = n
          while (cur) {
            visible.add(cur.id)
            cur = cur.parentId ? byId.get(cur.parentId) : undefined
          }
        }
      }
    }
    const out: Row[] = []
    const walk = (parent: Uuid | null, depth: number) => {
      for (const n of byParent.get(parent) ?? []) {
        if (visible && !visible.has(n.id)) continue
        const expanded = n.type === 'folder' && (visible !== null || !collapsed.has(n.id))
        out.push({ node: n, depth, expanded })
        if (n.type === 'folder' && expanded) walk(n.id, depth + 1)
      }
    }
    walk(null, 0)
    return out
  })

  function toggle(id: Uuid) {
    const next = new Set(collapsed)
    if (!next.delete(id)) next.add(id)
    collapsed = next
  }

  function activate(n: SessionNode) {
    if (n.type === 'folder') toggle(n.id)
    else onopen(n)
  }

  function openMenu(e: MouseEvent, node: SessionNode | null) {
    e.preventDefault()
    e.stopPropagation()
    if (node) selected = node.id
    menu = { x: e.clientX, y: e.clientY, node }
  }

  function pick(action: () => void) {
    menu = null
    action()
  }

  /** Cartella in cui creare: la cartella stessa, il padre di una sessione, o la radice. */
  function parentFor(n: SessionNode | null): Uuid | null {
    if (!n) return null
    return n.type === 'folder' ? n.id : n.parentId
  }

  function canDrop(target: Uuid | null): boolean {
    if (!dragId) return false
    const dragged = nodes.find((n) => n.id === dragId)
    if (!dragged || dragged.parentId === target) return false
    return target === null || !subtreeIds(nodes, dragId).has(target)
  }

  function dragOver(e: DragEvent, target: Uuid | null) {
    if (!canDrop(target)) return
    e.preventDefault()
    e.stopPropagation()
    dropTarget = target ?? 'root'
  }

  function drop(e: DragEvent, target: Uuid | null) {
    e.preventDefault()
    e.stopPropagation()
    if (dragId && canDrop(target)) onmove(dragId, target)
    dragId = null
    dropTarget = null
  }

  function onkey(e: KeyboardEvent, n: SessionNode) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      activate(n)
    } else if (e.key === 'F2') {
      if (n.type === 'folder') onrename(n)
      else onedit(n)
    } else if (e.key === 'Delete') ondelete(n)
  }
</script>

<svelte:window onclick={() => (menu = null)} onkeydown={(e) => e.key === 'Escape' && (menu = null)} />

<div class="tree">
  <div class="head">
    <strong>{t('sessions')}</strong>
    <span class="spacer"></span>
    <button type="button" title={t('newFolder')} aria-label={t('newFolder')} onclick={() => onnewfolder(parentFor(nodes.find((n) => n.id === selected) ?? null))}>
      📁+
    </button>
    <button type="button" title={t('newSession')} aria-label={t('newSession')} onclick={() => onnewsession(parentFor(nodes.find((n) => n.id === selected) ?? null))}>
      +
    </button>
  </div>
  <input class="filter" type="search" placeholder={t('filter')} bind:value={filter} aria-label={t('filter')} />

  <div
    class="list"
    class:drop={dropTarget === 'root'}
    role="tree"
    tabindex="-1"
    oncontextmenu={(e) => openMenu(e, null)}
    ondragover={(e) => dragOver(e, null)}
    ondragleave={() => (dropTarget = null)}
    ondrop={(e) => drop(e, null)}
  >
    {#each rows as { node, depth, expanded } (node.id)}
      <div
        class="row"
        class:selected={selected === node.id}
        class:drop={dropTarget === node.id}
        style:padding-left="{8 + depth * 16}px"
        role="treeitem"
        aria-selected={selected === node.id}
        aria-expanded={node.type === 'folder' ? expanded : undefined}
        tabindex="0"
        draggable="true"
        ondragstart={(e) => {
          dragId = node.id
          e.dataTransfer?.setData('text/plain', node.id)
        }}
        ondragend={() => {
          dragId = null
          dropTarget = null
        }}
        ondragover={(e) => dragOver(e, node.type === 'folder' ? node.id : node.parentId)}
        ondrop={(e) => drop(e, node.type === 'folder' ? node.id : node.parentId)}
        onclick={(e) => {
          e.stopPropagation()
          selected = node.id
          if (node.type === 'folder') toggle(node.id)
        }}
        ondblclick={() => node.type === 'session' && onopen(node)}
        oncontextmenu={(e) => openMenu(e, node)}
        onkeydown={(e) => onkey(e, node)}
      >
        <span class="icon">{node.type === 'folder' ? (expanded ? '▾ 📂' : '▸ 📁') : '🖥'}</span>
        <span class="name">{node.name}</span>
        {#if node.type === 'session'}
          {#each node.ui.tags as tag (tag)}<span class="tag">{tag}</span>{/each}
        {/if}
      </div>
    {/each}
    {#if nodes.length === 0}
      <p class="empty">{t('emptyTree')}</p>
    {/if}
  </div>
</div>

{#if menu}
  {@const target = menu.node}
  <div class="menu" role="menu" style:left="{menu.x}px" style:top="{menu.y}px">
    {#if target?.type === 'session'}
      <button role="menuitem" onclick={() => pick(() => onopen(target))}>{t('open')}</button>
      <button role="menuitem" onclick={() => pick(() => onedit(target))}>{t('edit')}</button>
    {/if}
    {#if target?.type === 'folder'}
      <button role="menuitem" onclick={() => pick(() => onrename(target))}>{t('rename')}</button>
    {/if}
    <button role="menuitem" onclick={() => pick(() => onnewsession(parentFor(target)))}>
      {target?.type === 'folder' ? t('newSessionHere') : t('newSession')}
    </button>
    <button role="menuitem" onclick={() => pick(() => onnewfolder(parentFor(target)))}>
      {target?.type === 'folder' ? t('newFolderHere') : t('newFolder')}
    </button>
    {#if target}
      <button role="menuitem" class="danger-item" onclick={() => pick(() => ondelete(target))}>{t('delete')}</button>
    {/if}
  </div>
{/if}

<style>
  .tree {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px;
  }
  .spacer {
    flex: 1;
  }
  .head button {
    padding: 2px 8px;
  }
  .filter {
    margin: 0 8px 8px;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding-bottom: 16px;
  }
  .list.drop {
    outline: 2px dashed var(--accent);
    outline-offset: -2px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    cursor: default;
    user-select: none;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: #2a3f6b;
  }
  .row.drop {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tag {
    font-size: 11px;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--border);
    color: var(--fg);
  }
  .empty {
    padding: 12px;
    color: var(--fg-dim);
  }
  .menu {
    position: fixed;
    z-index: 60;
    min-width: 180px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.5);
  }
  .menu button {
    text-align: left;
    border: none;
    background: transparent;
  }
  .menu button:hover {
    background: var(--bg-hover);
  }
  .danger-item {
    color: #ff8a80;
  }
</style>
