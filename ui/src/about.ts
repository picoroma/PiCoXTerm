// Contenuti della finestra "Informazioni su PiCoXterm" scelti dall'autore.

export const ABOUT = {
  author: 'Francesco Cocco',
  /** Licenza del progetto. Vuota = "da definire". */
  license: '',
  /** Frase sotto l'immagine (in corsivo). Vuota = non mostrata. */
  incipit: '',
}

// Immagine della finestra: basta mettere un file chiamato `about.jpg` (o .jpeg, .png, .webp, .svg)
// nella cartella `ui/src/assets/`. Viene incorporata nell'eseguibile. Se manca, si mostra un segnaposto.
const found = import.meta.glob('./assets/about.{jpg,jpeg,png,webp,svg}', {
  eager: true,
  query: '?url',
  import: 'default',
}) as Record<string, string>

export const aboutImage: string | null = Object.values(found)[0] ?? null
