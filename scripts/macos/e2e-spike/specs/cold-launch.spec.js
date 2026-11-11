// Task 2.4 CI-runner spike — the minimal cold-launch proof THROUGH the
// embedded WebDriver session: the driver connects and exposes the app
// window, the real UI renders under #root (first usable state), and the
// evidence screenshot lands in the run's evidence directory. The harness
// (scripts/macos/cold-launch-spike.sh) verifies the clean quit afterwards
// and writes the verdict status the workflow gates on.
import assert from 'node:assert/strict'
import { existsSync, statSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'

const evidenceDir = process.env.DRIFT_EVIDENCE_DIR

describe('built Drift app — macOS cold launch via the Tauri-service embedded driver', () => {
  it('the embedded driver connects and exposes the app window', async () => {
    const handles = await browser.getWindowHandles()
    assert.ok(
      handles.length >= 1,
      `expected at least one window handle, got ${handles.length}`,
    )
    const title = await browser.getTitle()
    assert.equal(title, 'Drift', `document title was ${JSON.stringify(title)}`)
  })

  it('reaches first usable state (rendered UI under #root)', async () => {
    const root = await browser.$('#root')
    await root.waitForExist({ timeout: 30000 })
    const html = await root.getHTML()
    assert.ok(
      html.length > 100,
      `#root rendered ${html.length} chars — expected a rendered UI, not an empty mount point`,
    )
  })

  it('captures the first-usable-state screenshot evidence', async () => {
    const shot = path.join(evidenceDir, 'first-usable-state.png')
    await browser.saveScreenshot(shot)
    assert.ok(existsSync(shot), 'screenshot file missing after saveScreenshot')
    const size = statSync(shot).size
    assert.ok(size > 4096, `screenshot suspiciously small (${size} bytes) — likely blank`)
  })
})
