import type { PagePort } from '../ports/page';
import type { Tab } from '../ports/tabs';

type Outcome = { ok: true; value: unknown } | { ok: false; error: string };

/** Runs `func(...args)` in the tab; its outcome. */
async function run(tabId: number, world: 'MAIN' | 'ISOLATED', func: (...a: string[]) => unknown, args: string[]) {
  // The typings cannot follow `args` through to `func`'s parameters.
  const injection = { target: { tabId }, world, func, args } as unknown as Parameters<
    typeof browser.scripting.executeScript
  >[0];
  const [first] = await browser.scripting.executeScript(injection);
  const injected = first as { result?: unknown; error?: unknown } | undefined;
  if (injected?.error) throw new Error(String(injected.error));
  const out = injected?.result as Outcome | undefined;
  if (!out) throw new Error('the script did not run in this tab');
  if (!out.ok) throw new Error(out.error);
  return out.value;
}

/** `browser.scripting` and `browser.tabs` (Firefox, Safari). */
export class BrowserPages implements PagePort {
  navigate(tabId: number, url: string): Promise<Tab> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        browser.tabs.onUpdated.removeListener(listener);
        reject(new Error(`timed out loading ${url}`));
      }, 30000);
      const listener = (id: number, change: { status?: string }, tab: { url?: string; title?: string; windowId?: number }) => {
        if (id !== tabId || change.status !== 'complete') return;
        if (tab.url === 'about:blank' && url !== 'about:blank') return;
        clearTimeout(timer);
        browser.tabs.onUpdated.removeListener(listener);
        resolve({ id, url: tab.url ?? url, title: tab.title ?? '', windowId: tab.windowId ?? -1 });
      };
      browser.tabs.onUpdated.addListener(listener);
      browser.tabs.update(tabId, { url }).catch((e: unknown) => {
        clearTimeout(timer);
        browser.tabs.onUpdated.removeListener(listener);
        reject(e instanceof Error ? e : new Error(String(e)));
      });
    });
  }

  evaluate(tabId: number, expression: string) {
    return run(
      tabId,
      'MAIN',
      async (expr: string) => {
        try {
          const value = await (0, eval)(expr);
          return { ok: true, value: value === undefined ? null : JSON.parse(JSON.stringify(value)) };
        } catch (e) {
          return { ok: false, error: e instanceof Error ? `${e.name}: ${e.message}` : String(e) };
        }
      },
      [expression],
    );
  }

  async click(tabId: number, selector: string) {
    await run(
      tabId,
      'ISOLATED',
      (sel: string) => {
        const el = document.querySelector(sel) as HTMLElement | null;
        if (!el) return { ok: false, error: `no element matches ${sel}` };
        el.scrollIntoView({ block: 'center' });
        el.click();
        return { ok: true, value: null };
      },
      [selector],
    );
  }

  async fill(tabId: number, selector: string, value: string) {
    await run(
      tabId,
      'ISOLATED',
      (sel: string, text: string) => {
        const el = document.querySelector(sel) as HTMLInputElement | HTMLTextAreaElement | null;
        if (!el) return { ok: false, error: `no element matches ${sel}` };
        el.focus();
        el.value = text;
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
        return { ok: true, value: null };
      },
      [selector, value],
    );
  }

  async screenshot(tabId: number) {
    const tab = await browser.tabs.update(tabId, { active: true });
    if (tab?.windowId !== undefined) await browser.windows.update(tab.windowId, { focused: true });
    const url = await browser.tabs.captureVisibleTab(tab?.windowId ?? browser.windows.WINDOW_ID_CURRENT, {
      format: 'png',
    });
    return url.replace(/^data:image\/png;base64,/, '');
  }
}
