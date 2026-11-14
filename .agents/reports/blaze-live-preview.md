# Blaze live reload with browser preview — 2026-11-13

## Change

Removed the CLI argument guard rejecting desktop `--preview` when `--live` or
`--compiler blaze` selects Blaze. Web, platform and external-reloader restrictions
remain in place. The existing supervisor owns the HTTP server, Swift capture helper
and app preview environment through patches and checkpointed restarts; no engine,
toolchain, SDK or preview-runtime change was needed.

Updated help and development docs. This is a source-CLI fix for PR #71: the already
published `rake-app-cli` alpha.4 archive remains unchanged and still rejects the
combination. A later CLI release is needed to distribute it through crates.io.

## Regression coverage

- Pinned `--live --preview` and explicit `--compiler blaze --preview` parse on macOS.
  The new test failed with the old guard before the fix.
- Preview references retain their epoch across a body patch, then invalidate and
  rebind on a new runtime, including reuse of a native window ID.
- The supervisor keeps the original PID for an Applied outcome and passes
  `RAKE_DEV_PREVIEW=1` on both initial launch and checkpointed replacement. This
  exercises fully registered, partially registered and standalone checkpoint cases.
- Existing Web rejection, strict-mode blocking and restart-notice tests remain intact.

CLI unit/integration suites: **402 passed, two ignored**. CLI Clippy, Rake format/lint
and the CI spacing lints for the affected CLI package passed. Supervisor contracts:
**15 passed**. Preview contracts: **18 passed**.
The full workspace was tested for the release baseline; this follow-up reran the
changed CLI's complete suite and related contracts, with full-platform CI on push.

## Real preview/live check

A disposable app generated from the published CLI uses registry-only SDK alpha.4.
Its Home view registers a `ReloadState` counter through `Entry::on_reload`, and
restores that counter when mounted. Only test-fixture source was changed.

Command: patched source CLI `rake dev --live --preview --require-state --bin preview71
--port 0 --no-open --json`. Used the SHA-verified published Blaze rake.2 archive in
an isolated short HOME, with the normal hybrid settings, sandbox and memory policy.
No existing engine/toolchain or installed compiler was changed.

Headless system Chrome ran the real preview HTML/JavaScript, Swift capture and video
decoder. The same browser page stayed open for the complete successful sequence:

| Step | App PID | Counter | Result |
|---|---:|---:|---|
| Browser clicks before edits | 24018 | 6 | Native input delivered through preview |
| `view!` text edit | 24018 | 6 | New text appeared in inspected tree and video pixels |
| Helper-body edit | 24018 | 6 | New text appeared in tree and video pixels |
| Add field to Home | 28512 | 6 | Strict checkpointed restart; generation 4, restored phase |
| Helper edit after restart | 28512 | 6 | Live patch and video update still worked |
| Browser click after restart | 28512 | 7 | Input targets the restored window |

Field-change save to restored preview: **5.763 s**. The preview changed its runtime
and stream epoch automatically, opened a second video connection, and returned to
Live without a page reload. Both HTTP video responses were successful; no browser
page errors or capture errors remained. The browser briefly shows the existing
“Native capture disconnected” notice while the old capture closes; it reconnects
and the notice expires automatically.

Single text/helper/post-restart checks took 0.887/0.553/0.738 s including browser
capture and screenshot overhead. These are smoke observations with a debug CLI,
not benchmark medians or native save-to-frame measurements.

The first harness attempt queried the replacement before its first painted tree;
an independent subsequent query confirmed the counter had restored. The harness
was corrected to wait for that first frame and then reran the full sequence above.
No product fix was needed for that test synchronization issue.

## Evidence and cleanup

Raw JSON, browser harness, screenshots and logs:
`/Users/yuriihulyk/Documents/Codex/2026-11-13/rake71-live-preview/`.
The original checkout, fork and system toolchains are unchanged. No package was
republished. The app, preview service, Chrome and private Blaze daemon were stopped
after acceptance. Temporary build/cache cleanup is recorded with the evidence.
