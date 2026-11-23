# Working rules for this repository

This is Gary Somerhalder's fork of `mattsse/chromiumoxide`, being rewritten as a
Chrome DevTools Protocol (CDP) client with Playwright-for-Chromium parity.
Gary owns the repo and is not a developer: keep status updates short and plain.

## Scope

- Chromium (Chrome, Chrome for Testing, chrome-headless-shell), driven over CDP,
  is the base and the device-mockup contract.
- Since 2026-11-23 also in scope (Gary's request): Electron, Android, the rest
  of CDP, Firefox (WebDriver BiDi), WebKit and Tauri, in the order and with
  the plan in `docs/BACKLOG-PLATFORMS.md`. None of it may change Chromium's
  default discovery, launch or rendering.
- Checklists live in `docs/PARITY.md`; the design lives in `docs/ARCHITECTURE.md`;
  open work lives in `docs/BACKLOG.md` and `docs/BACKLOG-PLATFORMS.md`.
- Goals live in `goals/`, one file per goal, in the shape `goals/README.md`
  sets out; keep each goal's Status log and "Done when" boxes current.

## Hard compatibility contract (garysomerhalder/device-mockup)

device-mockup must stay pixel-identical to Playwright. Never change without
messaging the device-mockup session first:

- browser discovery order (see `domain::discovery`);
- the default launch argument preset (see `domain::launch`) and the unique
  temporary user-data-dir per launch;
- contexts with emulation + basic auth, goto/setContent waiting, waitForFunction,
  addStyleTag, evaluate, viewport resize, screenshots (viewport, clip, fullPage);
- the pixel-parity integration test (`tests/pixel_parity.rs`) must stay green.

## How to work

- **TDD, visibly.** For each behaviour:
  1. commit the failing test first: `test(red): …`, with the failure output in the message;
  2. commit the smallest code that passes: `feat(green): …` (or `fix(green): …`);
  3. optionally `refactor: …`.
  Never write code before its test. Never weaken a test to get green.
- **DDD + hexagonal.** `domain` (pure, no I/O, no tokio, no protocol crate) ←
  `ports` (traits) ← `application` (use cases, public API) ← `adapters` (I/O).
  Dependencies point inward only. `tests/architecture.rs` enforces this.
- Tests: unit tests for the domain; application tests through in-memory port
  adapters; integration tests against real Chromium (no transport mocks).
- Keep the PDL code generator as its own crate (`chromiumoxide_pdl`).
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`
  must be clean. Keep `CHANGELOG.md` updated.
- Work on a `claude/…` branch; open a draft PR into
  `garysomerhalder/chromiumoxide` `main` per milestone. Never push to or open
  anything against `mattsse/chromiumoxide`. Never commit secrets.

## Local environment notes

- Playwright 1.63 is installed globally (`/opt/node-tools/node_modules/playwright`),
  browsers in `/opt/pw-browsers`. Integration tests find Chromium via discovery.
- Run integration tests: `cargo test --test integration -- --test-threads=2`.
