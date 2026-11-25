// The extension's layers point inward, as in the Rust library:
// domain <- ports <- application <- adapters / ui <- entrypoints.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(__dirname, '../src');

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : /\.tsx?$/.test(name) ? [path] : [];
  });
}

function imports(path: string): string[] {
  const src = readFileSync(path, 'utf8');
  return [...src.matchAll(/(?:import|export)[^'"]*from\s+['"]([^'"]+)['"]/g)].map((m) => m[1]);
}

/** What each layer may import (by folder under src/, or package). */
const RULES: Record<string, { layers: string[]; packages: RegExp | null; globals?: RegExp }> = {
  domain: { layers: ['domain'], packages: null, globals: /\b(chrome|browser|WebSocket|document|window)\./ },
  ports: { layers: ['domain', 'ports'], packages: null, globals: /\b(chrome|browser|WebSocket)\./ },
  application: { layers: ['domain', 'ports', 'application'], packages: null, globals: /\b(chrome|browser|WebSocket)\./ },
  adapters: { layers: ['domain', 'ports', 'adapters'], packages: null },
  ui: { layers: ['domain', 'ports', 'ui'], packages: /^(react|react-dom|@mui\/|@emotion\/)/, globals: /\b(chrome|browser)\./ },
};

describe('architecture', () => {
  for (const [layer, rule] of Object.entries(RULES)) {
    it(`${layer} depends only on ${rule.layers.join(', ')}`, () => {
      const bad: string[] = [];
      for (const file of files(join(SRC, layer))) {
        const rel = relative(SRC, file);
        for (const spec of imports(file)) {
          if (spec.startsWith('.')) {
            const target = relative(SRC, join(file, '..', spec)).split(/[\\/]/)[0];
            if (!rule.layers.includes(target)) bad.push(`${rel} imports ${spec}`);
          } else if (!rule.packages?.test(spec)) {
            bad.push(`${rel} imports package ${spec}`);
          }
        }
        if (rule.globals && rule.globals.test(readFileSync(file, 'utf8').replace(/\/\/.*$/gm, ''))) {
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
          const target = relative(join(SRC, 'ui'), join(file, '..', spec)).split(/[\\/]/)[0];
          const j = order.indexOf(target);
          if (j >= i) bad.push(`${relative(SRC, file)} (${level}) imports ${spec}`);
        }
      }
    }
    expect(bad).toEqual([]);
  });
});
