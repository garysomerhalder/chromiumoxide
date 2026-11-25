# Goals

This folder holds the goals for chromiumoxide. A goal is a result worth
having, written so that anyone (a person or a Claude session) can tell
whether it is met. Milestones, PRs and commits serve goals.

## The standard

Each goal gets **one file**, named for what it achieves, in lowercase with
dashes: `electron-apps-are-automatable.md`, not `m10.md` or `electron.md`.

Every file has the same sections, in this order:

1. **Header**: status, owner, dates (proposed, started, finished), and the
   repos and environments it touches.
2. **Outcome**: the result, in one or two sentences.
3. **Why**: who needs it and what it unblocks.
4. **Scope**: what is in, and what is explicitly out.
5. **Rules**: constraints the work must keep (contracts, how to work).
6. **Work**: the steps, in order, with links to PRs as they land.
7. **Evidence**: where the proof lives (tests, CI runs, recordings).
8. **Done when**: a checklist where anyone can mark each line true or
   false, without judgement calls.
9. **Stop rule**: when to stop and ask instead of going on.
10. **Status**: a dated log, newest last, one line per change.
11. **Set it**: the exact `/goal` line that runs the goal.

[`TEMPLATE.md`](TEMPLATE.md) is a blank goal in this shape; copy it for new
goals.

## Statuses

| Status | Meaning |
|---|---|
| `proposed` | Written down, not started. |
| `active` | Being worked on. |
| `met` | Every "Done when" line is true. |
| `stopped` | Its stop rule fired; waiting for a decision. |
| `dropped` | Decided against; the Status log says why. |

## Rules for the folder

- Finished goals stay in the folder (`met`, `stopped` or `dropped`), so the
  history stays readable.
- If the "Done when" list changes, the "Set it" line must change in the same
  commit, so the `/goal` that runs a goal always checks the current list.
- A status change gets a dated line in the goal's Status log.

## Goals

| Goal | Status |
|---|---|
| [Electron apps are automatable](electron-apps-are-automatable.md) | met |
| [Android devices are automatable](android-devices-are-automatable.md) | met |
| [Every CDP domain has a tested API](every-cdp-domain-has-a-tested-api.md) | met |
| [Firefox runs the Playwright API](firefox-runs-the-playwright-api.md) | met |
| [WebKit runs the Playwright API](webkit-runs-the-playwright-api.md) | met |
| [Tauri apps are automatable](tauri-apps-are-automatable.md) | met |
| [Firefox and WebKit offer every Playwright method Chromium does](firefox-and-webkit-match-the-playwright-api.md) | met |
| [Your own browser is automatable through an extension](your-own-browser-is-automatable.md) | met |
| [Playwright 1.66 is matched](playwright-1-66-is-matched.md) | proposed |
