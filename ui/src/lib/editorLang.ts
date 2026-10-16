import { StreamLanguage, type LanguageSupport, type StreamParser } from '@codemirror/language'
import { json } from '@codemirror/lang-json'
import { shell } from '@codemirror/legacy-modes/mode/shell'
import { yaml } from '@codemirror/legacy-modes/mode/yaml'
import { properties } from '@codemirror/legacy-modes/mode/properties'
import { toml } from '@codemirror/legacy-modes/mode/toml'
import { nginx } from '@codemirror/legacy-modes/mode/nginx'
import { xml } from '@codemirror/legacy-modes/mode/xml'
import { python } from '@codemirror/legacy-modes/mode/python'
import { standardSQL } from '@codemirror/legacy-modes/mode/sql'
import { dockerFile } from '@codemirror/legacy-modes/mode/dockerfile'
import { diff } from '@codemirror/legacy-modes/mode/diff'
import type { Extension } from '@codemirror/state'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const stream = (p: StreamParser<any>): Extension => StreamLanguage.define(p)

const BY_EXT: Record<string, () => Extension | LanguageSupport> = {
  sh: () => stream(shell),
  bash: () => stream(shell),
  zsh: () => stream(shell),
  ksh: () => stream(shell),
  profile: () => stream(shell),
  bashrc: () => stream(shell),
  env: () => stream(properties),
  json: () => json(),
  yml: () => stream(yaml),
  yaml: () => stream(yaml),
  ini: () => stream(properties),
  conf: () => stream(properties),
  cfg: () => stream(properties),
  properties: () => stream(properties),
  toml: () => stream(toml),
  xml: () => stream(xml),
  html: () => stream(xml),
  htm: () => stream(xml),
  py: () => stream(python),
  sql: () => stream(standardSQL),
  diff: () => stream(diff),
  patch: () => stream(diff),
}

const BY_NAME: Record<string, () => Extension | LanguageSupport> = {
  dockerfile: () => stream(dockerFile),
  'nginx.conf': () => stream(nginx),
  '.bashrc': () => stream(shell),
  '.bash_profile': () => stream(shell),
  '.profile': () => stream(shell),
  '.zshrc': () => stream(shell),
}

/** Evidenziazione della sintassi in base al nome del file (nessuna se non si riconosce il formato). */
export function languageFor(fileName: string): Extension {
  const name = fileName.toLowerCase()
  const byName = BY_NAME[name]
  if (byName) return byName()
  const dot = name.lastIndexOf('.')
  const ext = dot >= 0 ? name.slice(dot + 1) : ''
  return BY_EXT[ext]?.() ?? []
}
