#!/usr/bin/env python3
"""Playwright API coverage per engine and class.

Usage: python3 scripts/api_coverage.py [path/to/playwright-core/types/types.d.ts] [-v]

For each Playwright class, lists the public methods that the engine's Rust
type of the same name has no `pub fn` for (inside `impl <Type>` blocks).
Names compare in snake_case; a few Rust spellings stand in for keywords.
"""
import glob
import os
import re
import sys

args = [a for a in sys.argv[1:] if not a.startswith('-')]
VERBOSE = '-v' in sys.argv
TYPES = args[0] if args else os.environ.get(
    'PLAYWRIGHT_TYPES', 'node_modules/playwright-core/types/types.d.ts')
src = open(TYPES).read()

CLASSES = ['Page', 'Frame', 'Locator', 'FrameLocator', 'ElementHandle', 'JSHandle',
           'BrowserContext', 'Browser', 'Route', 'Request', 'Response', 'Keyboard',
           'Mouse', 'Touchscreen', 'Worker', 'Download', 'Dialog', 'Tracing', 'Clock',
           'APIRequestContext', 'APIResponse', 'ConsoleMessage', 'FileChooser',
           'WebSocket', 'Video']
RUST_NAME = {'JSHandle': 'JsHandle', 'APIRequestContext': 'ApiRequestContext',
             'APIResponse': 'ApiResponse'}
SKIP = {'on', 'once', 'off', 'addListener', 'removeListener', 'prependListener',
        'removeAllListeners', '[Symbol.asyncDispose]'}
ALIAS = {'type': ['type_text', 'kind', 'dialog_type'], 'continue': ['continue_'],
         'move': ['move_to'], 'toString': ['fmt'], 'waitForEvent': ['wait_for_event', 'events']}


def methods(iface):
    m = re.search(r'\nexport interface ' + iface + r'\b[^{]*\{', src)
    if not m:
        return set()
    i = j = m.end()
    depth = 1
    while depth:
        depth += (src[j] == '{') - (src[j] == '}')
        j += 1
    names = set()
    for line in src[i:j].split('\n'):
        mm = re.match(r'  (\w+)(<[^>]*>)?\(', line)
        if mm:
            names.add(mm.group(1))
    return names - SKIP


def snake(n):
    s = re.sub(r'([A-Z]+)([A-Z][a-z])', r'\1_\2', n)
    return re.sub(r'([a-z0-9])([A-Z])', r'\1_\2', s).lower()


def macro_fns():
    """macro name -> pub fn names its body defines (for `name!()` in impls)."""
    out = {}
    for f in glob.glob('src/application/*.rs'):
        text = open(f).read()
        for m in re.finditer(r'macro_rules! (\w+) \{', text):
            i = j = m.end()
            depth = 1
            while depth and j < len(text):
                depth += (text[j] == '{') - (text[j] == '}')
                j += 1
            out[m.group(1)] = set(re.findall(r'pub(?:\(crate\))? (?:async )?fn (\w+)', text[i:j]))
    return out


MACROS = macro_fns()


def impl_fns(files):
    """type name -> set of pub fn names in its impl blocks (and Display)."""
    out = {}
    for f in files:
        text = open(f).read()
        for m in re.finditer(r'\npub struct (\w+)(?:<[^>]*>)?\s*\{([^}]*)\}', text):
            out.setdefault(m.group(1), set()).update(
                re.findall(r'\n    pub (\w+):', m.group(2)))
        for m in re.finditer(r'\nimpl(?:<[^>]*>)?\s+(?:(?:[\w:]+)\s+for\s+)?(?:\w+::)*(\w+)(?:<[^>]*>)?\s*\{', text):
            trait = re.match(r'\nimpl(?:<[^>]*>)?\s+([\w:]+)\s+for', m.group(0))
            i = j = m.end()
            depth = 1
            while depth and j < len(text):
                depth += (text[j] == '{') - (text[j] == '}')
                j += 1
            fns = out.setdefault(m.group(1), set())
            if trait:
                if trait.group(1).endswith('Display'):
                    fns.add('fmt')
                continue
            fns |= set(re.findall(r'\n    pub(?:\(crate\))? (?:async )?fn (\w+)', text[i:j]))
            body = text[i:j]
            for name in re.findall(r'\n    (\w+)!\(\);', body):
                fns |= MACROS.get(name, set())
            # `name! { method(args) -> T; ... }`: one method per entry.
            for inv in re.finditer(r'\n    \w+! \{(.*?)\n    \}', body, re.S):
                fns |= set(re.findall(r'\n\s+(\w+)\(', inv.group(1)))
    return out


app = 'src/application/'
ENGINES = {
    'chromium': [f for f in glob.glob(app + '*.rs')
                 if not re.search(r'firefox|webkit|safari|tauri|android|electron', f)],
    'firefox': glob.glob(app + 'firefox*.rs'),
    # WebKit shares Firefox's Request and Response types.
    'webkit': glob.glob(app + 'webkit*.rs'),
}
# Types an engine re-exports from another module: class -> source files.
BORROWED = {
    'firefox': {'WebSocket': [app + 'websocket.rs'],
                'APIRequestContext': [app + 'request.rs'],
                'APIResponse': [app + 'request.rs']},
}

totals = {}
for engine, files in ENGINES.items():
    fns = impl_fns(files)
    for cls, borrowed in BORROWED.get(engine, {}).items():
        fns[RUST_NAME.get(cls, cls)] = impl_fns(borrowed).get(RUST_NAME.get(cls, cls), set())
    have = total = 0
    for cls in CLASSES:
        ms = sorted(methods(cls))
        rust = RUST_NAME.get(cls, cls)
        if engine == 'chromium' and cls == 'ElementHandle':
            rust = 'JsHandle'  # one handle type, as Playwright's ElementHandle extends JSHandle
        got = fns.get(rust, set())
        miss = [m for m in ms if snake(m) not in got
                and not any(a in got for a in ALIAS.get(m, []))]
        have += len(ms) - len(miss)
        total += len(ms)
        if VERBOSE or (miss and engine != 'chromium') or (miss and '-c' in sys.argv):
            print(f"{cls:18} {engine:8} {len(ms) - len(miss):3}/{len(ms):3} missing: {' '.join(miss)}")
    totals[engine] = (have, total)
for e, (h, t) in totals.items():
    print(f"TOTAL {e}: {h}/{t}")
