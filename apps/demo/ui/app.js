/*
 * The demo window. Everything interesting happens in Rust; this file renders a
 * verdict and sends edits back.
 */

const invoke = window.__TAURI__.core.invoke

const el = (id) => document.getElementById(id)
const encoder = new TextEncoder()
const decoder = new TextDecoder()

const SEVERITY_ORDER = { none: 0, low: 1, medium: 2, high: 3, critical: 4 }

/** Rust reports byte offsets; the DOM wants UTF-16. Convert through bytes. */
function sliceBytes(bytes, start, end) {
  return decoder.decode(bytes.subarray(start, end))
}

/** snake_cases the Rust enum name so it reads like the CLI and the JSON. */
function kindName(kind) {
  return kind.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase()
}

function worstSeverityAt(flags, start, end) {
  let worst = null
  for (const flag of flags) {
    if (flag.end <= flag.start) continue
    if (flag.start < end && flag.end > start) {
      if (!worst || SEVERITY_ORDER[flag.severity] > SEVERITY_ORDER[worst]) worst = flag.severity
    }
  }
  return worst
}

/**
 * Draws the text with flagged spans marked, and a caret where a word appears to
 * have gone missing.
 */
function renderMarked(container, text, flags) {
  container.textContent = ''
  const bytes = encoder.encode(text)

  const boundaries = new Set([0, bytes.length])
  const carets = new Set()
  for (const flag of flags) {
    if (flag.end > flag.start) {
      boundaries.add(flag.start)
      boundaries.add(flag.end)
    } else {
      carets.add(flag.start)
      boundaries.add(flag.start)
    }
  }

  const points = [...boundaries].sort((a, b) => a - b)
  for (let i = 0; i < points.length; i += 1) {
    const start = points[i]
    if (carets.has(start)) {
      const caret = document.createElement('span')
      caret.className = 'caret-marker'
      caret.textContent = '‸'
      caret.title = 'a word may be missing here'
      container.append(caret)
    }
    const end = points[i + 1]
    if (end === undefined || end <= start) continue

    const chunk = sliceBytes(bytes, start, end)
    const severity = worstSeverityAt(flags, start, end)
    if (!severity) {
      container.append(document.createTextNode(chunk))
      continue
    }
    const mark = document.createElement('mark')
    mark.dataset.severity = severity
    mark.textContent = chunk
    container.append(mark)
  }
}

function renderFlags(verdict) {
  const list = el('flags')
  list.textContent = ''
  el('empty').hidden = verdict.flags.length > 0

  for (const flag of verdict.flags) {
    const item = document.createElement('div')
    item.className = 'flag'
    item.dataset.severity = flag.severity

    const head = document.createElement('div')
    head.className = 'head'

    const severity = document.createElement('span')
    severity.className = 'severity'
    severity.textContent = flag.severity

    const kind = document.createElement('span')
    kind.className = 'kind'
    kind.textContent = kindName(flag.kind)

    head.append(severity, kind)

    const evidence = document.createElement('div')
    evidence.className = 'evidence'
    evidence.textContent = flag.evidence

    item.append(head, evidence)
    list.append(item)
  }
}

const EXPLANATIONS = {
  pass: 'Insert immediately.',
  highlight: 'Insert, but mark the flagged words.',
  hold: "Don't insert until the user confirms.",
}

function setMeter(key, value) {
  el(`${key}-value`).textContent = value.toFixed(2)
  el(`${key}-fill`).style.width = `${Math.min(value, 1) * 100}%`
}

function renderVerdict(verdict) {
  el('banner').dataset.action = verdict.action
  el('action').textContent = verdict.action.toUpperCase()
  el('what').textContent = EXPLANATIONS[verdict.action] ?? ''

  renderMarked(el('output'), verdict.text, verdict.flags)

  const baseline = el('baseline')
  baseline.textContent = ''
  if (verdict.baseline !== verdict.text) {
    baseline.append(document.createTextNode('without Readback: '))
    const struck = document.createElement('s')
    struck.textContent = verdict.baseline
    baseline.append(struck)
  }

  setMeter('risk', verdict.risk)
  setMeter('stakes', verdict.stakes)
  setMeter('suspicion', verdict.suspicion)

  renderFlags(verdict)
}

/* ------------------------------------------------------------------ state */

let scenarios = []
let selectedId = null
/** Speech regions from a loaded clip or the selected scenario. */
let speech = []

function currentRequest() {
  const vocabulary = el('vocabulary')
    .value.split(',')
    .map((term) => term.trim())
    .filter(Boolean)

  return {
    heard: el('heard').value,
    polished: el('polished').value,
    app: el('app').value.trim() || null,
    vocabulary,
    locales: el('hinglish').checked ? ['en', 'hinglish'] : ['en'],
    words: [],
    speech,
  }
}

async function run() {
  renderVerdict(await invoke('check', { request: currentRequest() }))
}

function selectScenario(id) {
  const scenario = scenarios.find((s) => s.id === id)
  if (!scenario) return

  selectedId = id
  speech = []
  el('clip-status').textContent = ''
  el('heard').value = scenario.heard
  el('polished').value = scenario.polished ?? ''
  el('app').value = scenario.app ?? ''
  el('vocabulary').value = scenario.vocabulary.join(', ')
  if (scenario.category === 'hinglish') el('hinglish').checked = true

  for (const button of document.querySelectorAll('.scenario')) {
    button.setAttribute('aria-current', String(button.dataset.id === id))
  }
  run()
}

function renderScenarios(filter) {
  const list = el('scenarios')
  list.textContent = ''
  const needle = filter.trim().toLowerCase()

  const matches = scenarios.filter(
    (s) =>
      !needle ||
      s.id.includes(needle) ||
      s.category.includes(needle) ||
      s.spoken.toLowerCase().includes(needle),
  )

  for (const [label, benign] of [
    ['Something went wrong', false],
    ['Controls', true],
  ]) {
    const group = matches.filter((s) => s.benign === benign)
    if (group.length === 0) continue

    const heading = document.createElement('div')
    heading.className = 'group-label'
    heading.textContent = `${label} (${group.length})`
    list.append(heading)

    for (const scenario of group) {
      const button = document.createElement('button')
      button.className = 'scenario'
      button.type = 'button'
      button.dataset.id = scenario.id
      button.dataset.benign = String(scenario.benign)
      button.setAttribute('aria-current', String(scenario.id === selectedId))

      const dot = document.createElement('span')
      dot.className = 'dot'

      const id = document.createElement('span')
      id.className = 'id'
      id.textContent = scenario.id

      const spoken = document.createElement('span')
      spoken.className = 'spoken'
      spoken.textContent = scenario.spoken

      button.append(dot, id, spoken)
      button.addEventListener('click', () => selectScenario(scenario.id))
      list.append(button)
    }
  }
}

async function loadClip() {
  const path = el('clip').value.trim()
  const status = el('clip-status')
  if (!path) return

  try {
    const clip = await invoke('load_clip', { path })
    speech = clip.speech.map((region) => [region.startMs ?? region.start_ms, region.endMs ?? region.end_ms])
    status.dataset.error = 'false'
    status.textContent = `${clip.durationMs ?? clip.duration_ms} ms at ${clip.sampleRate ?? clip.sample_rate} Hz — ${speech.length} speech region(s). Word timings are still needed to find gaps.`
    run()
  } catch (error) {
    speech = []
    status.dataset.error = 'true'
    status.textContent = String(error)
  }
}

/* -------------------------------------------------------------------- init */

let pending = null
function scheduleRun() {
  // Debounced so every keystroke does not redraw the overlay, but short enough
  // that it still feels live.
  clearTimeout(pending)
  pending = setTimeout(run, 120)
}

async function init() {
  el('version').textContent = `v${await invoke('version')}`
  scenarios = await invoke('scenarios')
  renderScenarios('')

  for (const id of ['heard', 'polished', 'app', 'vocabulary']) {
    el(id).addEventListener('input', scheduleRun)
  }
  el('hinglish').addEventListener('change', run)
  el('search').addEventListener('input', (event) => renderScenarios(event.target.value))
  el('load-clip').addEventListener('click', loadClip)

  selectScenario(scenarios[0]?.id)
}

init()
