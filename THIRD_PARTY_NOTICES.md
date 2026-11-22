# Third-party notices

chromiumoxide includes material from **Playwright** (https://github.com/microsoft/playwright),
Copyright (c) Microsoft Corporation, licensed under the Apache License, Version 2.0
(https://www.apache.org/licenses/LICENSE-2.0):

- `src/application/scripts/injected.js` — Playwright's injected script (selector
  engines, actionability and expect helpers), playwright-core 1.65.1, unmodified
  apart from a header comment.
- `src/application/scripts/clock.js`, `storage.js`, `snapshot_streamer.js` and
  `webmcp.js` — Playwright's clock, storage-state, trace snapshot and WebMCP
  scripts, playwright-core 1.65.1, unmodified apart from a header comment.
- `src/application/scripts/prepare_screenshot.js` — Playwright's screenshot
  preparation function.
- `src/application/scripts/websocket_mock.js` — Playwright's WebSocket mock used
  by `routeWebSocket`, playwright-core 1.63.0 (identical in 1.65.1), unmodified.
- `src/domain/data/devices.json` — Playwright's device descriptors.
- `src/domain/data/us_keyboard_layout.json` — Playwright's US keyboard layout.
- `src/domain/data/font_families.json` — Playwright's default font families.
- Behaviour, constants and algorithms ported from Playwright's TypeScript
  sources (launch switches, emulation, screenshots, frames, input, selectors).

The Chrome DevTools Protocol definitions in `chromiumoxide_cdp/pdl` come from
https://github.com/ChromeDevTools/devtools-protocol, Copyright The Chromium
Authors, BSD-style license.
