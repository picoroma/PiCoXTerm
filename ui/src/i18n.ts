type Dict = Record<string, string>

const it: Dict = {
  host: 'Host',
  port: 'Porta',
  user: 'Utente',
  password: 'Password',
  connect: 'Connetti',
  connecting: 'Connessione in corso…',
  disconnect: 'Disconnetti',
  closed: 'Connessione chiusa.',
  hostKeyTitle: 'Host key sconosciuta',
  hostKeyBody: 'Il server non è mai stato visto prima. Verifica la fingerprint prima di continuare.',
  accept: 'Accetta e salva',
  cancel: 'Annulla',
  fingerprint: 'Fingerprint',
  keyType: 'Tipo di chiave',
  injectCwd: 'Attiva tracciamento cwd',
  cwd: 'Directory',
  pasteTitle: 'Incollare più righe?',
  pasteBody: 'Stai incollando {n} righe: ogni riga verrà eseguita dalla shell.',
  paste: 'Incolla',
}

const en: Dict = {
  host: 'Host',
  port: 'Port',
  user: 'User',
  password: 'Password',
  connect: 'Connect',
  connecting: 'Connecting…',
  disconnect: 'Disconnect',
  closed: 'Connection closed.',
  hostKeyTitle: 'Unknown host key',
  hostKeyBody: 'This server has never been seen before. Check the fingerprint before continuing.',
  accept: 'Accept and save',
  cancel: 'Cancel',
  fingerprint: 'Fingerprint',
  keyType: 'Key type',
  injectCwd: 'Enable cwd tracking',
  cwd: 'Directory',
  pasteTitle: 'Paste multiple lines?',
  pasteBody: 'You are pasting {n} lines: the shell will run each one.',
  paste: 'Paste',
}

const dicts: Record<string, Dict> = { it, en }
const lang = navigator.language.toLowerCase().startsWith('it') ? 'it' : 'en'

export function t(key: string): string {
  return dicts[lang][key] ?? key
}
