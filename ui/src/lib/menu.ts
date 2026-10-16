/** Posizione di un menu contestuale: il punto del clic, in coordinate della finestra. */
export interface MenuPos {
  x: number
  y: number
}

const MARGIN = 4

/**
 * Azione Svelte per i menu contestuali: li posiziona nel punto del clic e li tiene dentro la finestra.
 * Se non c'è spazio sotto il cursore si apre verso l'alto; se non basta nemmeno così, si incolla al
 * bordo e diventa scorrevole.
 */
export function placeMenu(node: HTMLElement, pos: MenuPos) {
  function place(p: MenuPos) {
    node.style.position = 'fixed'
    node.style.maxHeight = `${window.innerHeight - 2 * MARGIN}px`
    node.style.overflowY = 'auto'
    node.style.left = `${p.x}px`
    node.style.top = `${p.y}px`
    const r = node.getBoundingClientRect()

    let x = p.x
    if (x + r.width > window.innerWidth - MARGIN) x = Math.max(MARGIN, window.innerWidth - r.width - MARGIN)

    let y = p.y
    if (y + r.height > window.innerHeight - MARGIN) {
      // Prima si prova ad aprirlo sopra il cursore, altrimenti lo si porta contro il bordo basso.
      y = p.y - r.height >= MARGIN ? p.y - r.height : Math.max(MARGIN, window.innerHeight - r.height - MARGIN)
    }
    node.style.left = `${x}px`
    node.style.top = `${y}px`
  }

  place(pos)
  return { update: place }
}
