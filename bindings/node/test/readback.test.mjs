import { test } from 'node:test'
import assert from 'node:assert/strict'

import { Readback, checkCleanup, version } from '../index.js'

test('a dropped negation is held and restored', () => {
  const rb = new Readback()
  const verdict = rb.check({
    text: 'never merge a change like this',
    cleaned: 'Merge a change like this.',
  })

  assert.equal(verdict.action, 'hold')
  assert.match(verdict.text.toLowerCase(), /never/)
  assert.equal(verdict.flags[0].kind, 'dropped_negation')
  assert.equal(verdict.flags[0].severity, 'critical')
  assert.ok(verdict.provenance.cleanupReverted)
})

test('harmless polish passes through byte for byte', () => {
  const rb = new Readback()
  const cleaned = 'We need to ship it tomorrow.'
  const verdict = rb.check({ text: 'um so we need to ship it tomorrow', cleaned })

  assert.equal(verdict.action, 'pass')
  assert.equal(verdict.text, cleaned)
  assert.equal(verdict.flags.length, 0)
})

test('text alone is enough to get a verdict', () => {
  const verdict = new Readback().check({ text: 'hey are you around' })
  assert.equal(verdict.action, 'pass')
  assert.equal(verdict.risk, 0)
})

test('flag spans are byte offsets into the returned text', () => {
  const verdict = new Readback().check({
    text: 'do not deploy this',
    cleaned: 'Deploy this.',
  })
  const flag = verdict.flags[0]
  const slice = Buffer.from(verdict.text, 'utf8').subarray(flag.start, flag.end).toString('utf8')
  assert.match(slice.toLowerCase(), /not/)
})

test('word confidence raises suspicion without inventing stakes', () => {
  const rb = new Readback()
  const smallTalk = rb.check({
    text: 'lol sounds good',
    words: [
      { text: 'lol', confidence: 0.41 },
      { text: 'sounds', confidence: 0.98 },
      { text: 'good', confidence: 0.99 },
    ],
  })

  assert.equal(smallTalk.action, 'pass')
  assert.ok(smallTalk.suspicion > 0.5, 'the shaky word is still noticed')
  assert.ok(smallTalk.stakes < 0.2, 'but nothing here is worth blocking')
  assert.equal(smallTalk.flags[0].kind, 'low_confidence')
})

test('the same evidence escalates in a terminal', () => {
  const rb = new Readback({ recommended: true })
  const input = {
    text: 'never delete production',
    words: [
      { text: 'never', confidence: 0.41 },
      { text: 'delete', confidence: 0.98 },
      { text: 'production', confidence: 0.99 },
    ],
  }

  const terminal = rb.check({ ...input, app: 'Ghostty' })
  const notes = rb.check({ ...input, app: 'Obsidian' })

  assert.equal(terminal.action, 'hold')
  assert.notEqual(notes.action, 'hold')
  assert.equal(terminal.risk, notes.risk, 'same evidence, same risk')
})

test('per-call vocabulary is protected', () => {
  const verdict = new Readback().check({
    text: 'ask Harpawan to check it',
    cleaned: 'Ask Harpreet to check it.',
    vocabulary: ['Harpawan'],
  })

  assert.match(verdict.text, /Harpawan/)
  assert.equal(verdict.flags[0].kind, 'changed_protected_term')
})

test('hinglish loads only when asked for', () => {
  const withoutHinglish = new Readback().check({ text: 'abhi mat bhejo', cleaned: 'Bhejo abhi.' })
  assert.equal(withoutHinglish.action, 'pass')

  const withHinglish = new Readback({ locales: ['en', 'hinglish'] }).check({
    text: 'abhi mat bhejo',
    cleaned: 'Bhejo abhi.',
  })
  assert.notEqual(withHinglish.action, 'pass')
  assert.match(withHinglish.text.toLowerCase(), /mat/)
})

test('voice-activity regions surface a dropped word', () => {
  const verdict = new Readback({ recommended: true }).check({
    text: 'merge production',
    words: [
      { text: 'merge', startMs: 1000, endMs: 1400, confidence: 0.96 },
      { text: 'production', startMs: 1400, endMs: 2000, confidence: 0.95 },
    ],
    speech: [{ startMs: 500, endMs: 2000 }],
    app: 'Ghostty',
  })

  assert.ok(verdict.provenance.omissionCheckRan)
  assert.ok(verdict.flags.some((f) => f.kind === 'possible_omission'))
  assert.notEqual(verdict.action, 'pass')
})

test('custom app policies take precedence', () => {
  const rb = new Readback({
    recommended: true,
    apps: [{ pattern: 'Ghostty', hold: 1.01, highlight: 0.9 }],
  })
  const verdict = rb.check({
    text: 'never merge this',
    cleaned: 'Merge this.',
    app: 'Ghostty',
  })

  assert.notEqual(verdict.action, 'hold', 'the explicit rule should override the preset')
})

test('an unknown locale throws a readable error', () => {
  assert.throws(() => new Readback({ locales: ['klingon'] }), /unknown locale/)
})

test('checkCleanup runs the guard on its own', () => {
  const outcome = checkCleanup('deploy this to staging', 'Deploy this to production.')
  assert.equal(outcome.reverted, true)
  assert.equal(outcome.text, 'Deploy this to staging.')
  assert.equal(outcome.flags[0].kind, 'changed_environment')

  const quiet = checkCleanup('um we should ship it', 'We should ship it.')
  assert.equal(quiet.reverted, false)
  assert.equal(quiet.flags.length, 0)
})

test('version reports the crate version', () => {
  assert.match(version(), /^\d+\.\d+\.\d+$/)
})
