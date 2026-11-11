// Task 2.4 CI-runner spike — minimal WebdriverIO config driving the BUILT
// Drift app on macOS through the @wdio/tauri-service EMBEDDED WebDriver
// server.
//
// Design mandate (design.md "Testing Strategy → Delivery gates and
// brownfield coverage"): "Use the WebdriverIO Tauri service embedded macOS
// server; direct tauri-driver/safaridriver does not drive WKWebView."
// macOS provider decision (from the service's own platform docs, verified
// 2026-11-11): 'embedded' is the default and the only macOS provider that
// needs no paid API key — 'external' (cargo tauri-driver) supports only
// Windows/Linux, and 'crabnebula' requires CN_API_KEY (a secret this
// repository's CI forbids).
//
// Paths arrive via environment from scripts/macos/cold-launch-spike.sh so
// the same config works from any checkout location on the CI runner.
import process from 'node:process'

const appBinary = process.env.DRIFT_APP_BINARY
const evidenceDir = process.env.DRIFT_EVIDENCE_DIR

if (!appBinary || !evidenceDir) {
  throw new Error(
    'DRIFT_APP_BINARY and DRIFT_EVIDENCE_DIR must be set — run through scripts/macos/cold-launch-spike.sh',
  )
}

export const config = {
  runner: 'local',
  specs: ['./specs/**/*.spec.js'],
  maxInstances: 1,
  capabilities: [
    {
      maxInstances: 1,
      browserName: 'tauri',
      'tauri:options': {
        application: appBinary,
      },
    },
  ],
  services: [
    [
      '@wdio/tauri-service',
      {
        appBinaryPath: appBinary,
        // THE macOS decision (design mandate): the service's embedded
        // WebDriver server — never the platform driver, never direct
        // tauri-driver, never a keyed third-party driver.
        driverProvider: 'embedded',
        embeddedPort: 4455,
        // Embedded-server cold start is slow on CI runners; the service's
        // own defaults are 60s start / 2s status poll — both raised per its
        // configuration docs for slow CI environments.
        startTimeout: 180000,
        statusPollTimeout: 5000,
        captureBackendLogs: true,
        captureFrontendLogs: true,
        logLevel: 'debug',
      },
    ],
  ],
  logLevel: 'info',
  bail: 1,
  outputDir: evidenceDir,
  framework: 'mocha',
  mochaOpts: {
    ui: 'bdd',
    timeout: 120000,
  },
  reporters: ['spec'],
}
