// The extension's layers point inward, as in the Rust library:
// domain <- ports <- application <- adapters / ui <- entrypoints.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(__dirname, '../src');

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name: string) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : /\.tsx?$/.test(name) ? [path] : [];
  });
}

/** The source without comments. */
function code(path: string): string {
  return readFileSync(path, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
}

function imports(path: string): string[] {
  const src = readFileSync(path, 'utf8');
  return [...src.matchAll(/(?:import|export)[^'"]*from\s+['"]([^'"]+)['"]/g)].map((m) => m[1]!);
}

/** What each layer may import (by folder under src/, or package). */
/** Calls to the browser's extension APIs, or opening sockets. */
const BROWSER_API = /\b(chrome|browser)\.(tabs|debugger|windows|runtime|scripting|storage|action)\b|\bnew WebSocket\b/;

const RULES: Record<string, { layers: string[]; packages: RegExp | null; globals?: RegExp }> = {
  domain: { layers: ['domain'], packages: null, globals: BROWSER_API },
  ports: { layers: ['domain', 'ports'], packages: null, globals: BROWSER_API },
  application: { layers: ['domain', 'ports', 'application'], packages: null, globals: BROWSER_API },
  adapters: { layers: ['domain', 'ports', 'adapters'], packages: null },
  ui: { layers: ['domain', 'ports', 'ui'], packages: /^(react|react-dom|@mui\/|@emotion\/)/, globals: BROWSER_API },
};

describe('architecture', () => {
  for (const [layer, rule] of Object.entries(RULES)) {
    it(`${layer} depends only on ${rule.layers.join(', ')}`, () => {
      const bad: string[] = [];
      for (const file of files(join(SRC, layer))) {
        const rel = relative(SRC, file);
        for (const spec of imports(file)) {
          if (spec.startsWith('.')) {
            const target = relative(SRC, join(file, '..', spec)).split(/[\\/]/)[0]!;
            if (!rule.layers.includes(target)) bad.push(`${rel} imports ${spec}`);
          } else if (!rule.packages?.test(spec)) {
            bad.push(`${rel} imports package ${spec}`);
          }
        }
        if (rule.globals && rule.globals.test(code(file))) {
          bad.push(`${rel} uses a browser API directly`);
        }
      }
      expect(bad).toEqual([]);
    });
  }

  it('the UI follows atomic design: each level uses only lower ones', () => {
    const order = ['atoms', 'molecules', 'organisms', 'templates', 'pages'];
    const bad: string[] = [];
    for (const [i, level] of order.entries()) {
      for (const file of files(join(SRC, 'ui', level))) {
        for (const spec of imports(file).filter((s) => s.startsWith('.'))) {
          const target = relative(join(SRC, 'ui'), join(file, '..', spec)).split(/[\\/]/)[0]!;
          const j = order.indexOf(target);
          if (j >= i) bad.push(`${relative(SRC, file)} (${level}) imports ${spec}`);
        }
      }
    }
    expect(bad).toEqual([]);
  });
});
