import fs from 'node:fs'
const SRC = (process.argv[2] ?? '../soundgineer') + '/src'
const params = await import(SRC + '/shared/params.ts')
const messages = await import(SRC + '/shared/messages.ts')

const out = { params: [], sources: [], presets: [] }
for (const d of params.PARAMS) {
  let unit = ''
  if (d.fmt) {
    const sample = d.fmt(d.choices ? 0 : (d.min + d.max) / 2 || 1)
    const m = /[-\d.]+\s*([^\d\s.-][^\d]*)$/.exec(String(sample))
    unit = m ? m[1].trim() : ''
    if (unit === 'kHz') unit = 'Hz'
    if (unit === 'ms') unit = 's'
    if (unit === '%') unit = ''
  }
  out.params.push({
    id: d.id, name: d.name, group: d.group, min: d.min, max: d.max, def: d.def,
    curve: d.curve ?? 'lin', step: d.step ?? 0, choices: d.choices ?? null,
    moddable: !!d.moddable, unit
  })
}
out.sources = messages.MOD_SOURCES.map(s => ({ id: s.id, name: s.name }))
out.syncDivisions = params.SYNC_DIVISIONS
out.divisionBeats = params.SYNC_DIVISIONS.map((_, i) => params.divisionToBeats(i))

// ---- presets: pull the FACTORY literal out of presets.ts (the file touches the DOM, so it
// can't be imported) and evaluate it with the same P() helper.
const text = fs.readFileSync(SRC + '/ui/presets.ts', 'utf8')
const start = text.indexOf('const FACTORY: Partial<PresetData>[] = [')
const open = text.indexOf('= [', start) + 2
let depth = 0, end = -1
for (let i = open; i < text.length; i++) {
  if (text[i] === '[') depth++
  else if (text[i] === ']') { depth--; if (depth === 0) { end = i; break } }
}
const literal = text.slice(open, end + 1)
const P = raw => raw
const factory = eval(literal)
// categories from the divider comments between entries
const names = [...literal.matchAll(/name: '([^']+)'/g)].map(m => ({ name: m[1], at: m.index }))
const dividers = [...literal.matchAll(/\/\/ -{10,}\s*([A-Za-z\/ &]+?)\s*$/gm)].map(m => ({ cat: m[1].trim(), at: m.index }))
let cat = 'Featured'
let di = 0
const cats = {}
for (const n of names) {
  while (di < dividers.length && dividers[di].at < n.at) cat = dividers[di++].cat
  cats[n.name] = cat
}
for (const p of factory) {
  out.presets.push({ name: p.name, category: cats[p.name], params: p.params ?? {}, mods: p.mods ?? [] })
}
fs.writeFileSync(process.argv[3] ?? 'soundgineer.json', JSON.stringify(out))
console.log(out.params.length, 'params,', out.sources.length, 'sources,', out.presets.length, 'presets')
console.log([...new Set(out.presets.map(p => p.category))], dividers.map(d => d.cat))
const fmts = {}; out.params.forEach(p => fmts[p.unit] = (fmts[p.unit] || 0) + 1); console.log(fmts)
console.log(out.presets.reduce((a, p) => a + p.mods.length, 0), 'mod routes, max per preset', Math.max(...out.presets.map(p => p.mods.length)))
console.log(Object.keys(messages), out.presets.some(p => p.lfoShapes))
