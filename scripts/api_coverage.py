#!/usr/bin/env python3
"""Playwright API coverage per engine, by method name."""
import re,glob,sys
import os
# Usage: python3 scripts/api_coverage.py [path/to/playwright-core/types/types.d.ts]
# Compares Playwright's public methods with each engine's pub fns (by name).
TYPES=sys.argv[1] if len(sys.argv)>1 else os.environ.get('PLAYWRIGHT_TYPES','node_modules/playwright-core/types/types.d.ts')
src=open(TYPES).read()
def methods(iface):
    m=re.search(r'\nexport interface '+iface+r'\b[^{]*\{',src)
    if not m: return set()
    i=m.end(); depth=1; j=i
    while depth:
        c=src[j]; depth+= (c=='{')-(c=='}'); j+=1
    body=src[i:j]
    names=set()
    for line in body.split('\n'):
        mm=re.match(r'  (\w+)(<[^>]*>)?\(',line)
        if mm: names.add(mm.group(1))
    names|=set(re.findall(r'\n  on\(event: \'(\w+)\'',body) and [])
    return names
def snake(n):
    s=re.sub(r'([A-Z]+)([A-Z][a-z])',r'\1_\2',n); s=re.sub(r'([a-z0-9])([A-Z])',r'\1_\2',s); return s.lower()
engines={'firefox':glob.glob('src/application/firefox*.rs'),'webkit':glob.glob('src/application/webkit*.rs'),'chromium':[f for f in glob.glob('src/application/*.rs') if not re.search('firefox|webkit|safari|tauri|android',f)]}
skip={'on','once','off','addListener','removeListener','prependListener','removeAllListeners','waitForEvent','_'}
for iface in ['Page','Locator','BrowserContext','Browser','Frame','Route','Request','Response','ElementHandle','Keyboard','Mouse','Worker','Download','Dialog','FrameLocator','JSHandle','APIRequestContext','Tracing','Clock']:
    ms=sorted(methods(iface)-skip)
    for eng,files in engines.items():
        text=''.join(open(f).read() for f in files)
        fns=set(re.findall(r'pub(?:\(crate\))? (?:async )?fn (\w+)',text))|set(re.findall(r'fn (fmt)\(',text))
        alias={'type':['type_text','kind','dialog_type'],'continue':['continue_'],'move':['move_to'],'toString':['fmt']}
        miss=[m for m in ms if snake(m) not in fns and m.lower() not in fns and not any(a in fns for a in alias.get(m,[]))]
        if 'toString' in miss and re.search(r'impl (std::)?fmt::Display for Locator',text): miss.remove('toString')
        print(f"{iface:18} {eng:8} {len(ms)-len(miss):3}/{len(ms):3} missing: {' '.join(miss)}")

