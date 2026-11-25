import { defineConfig } from 'wxt';

export default defineConfig({
  srcDir: 'src',
  modules: ['@wxt-dev/module-react'],
  manifest: ({ browser, manifestVersion }) => ({
    name: 'chromiumoxide',
    description: 'Lets chromiumoxide drive the browser you already use.',
    // Chrome and Edge: the debugger. Firefox and Safari have none for
    // extensions: scripts in tabs, and pictures of the visible tab.
    permissions:
      browser === 'firefox' || browser === 'safari'
        ? ['tabs', 'scripting', 'activeTab', ...(manifestVersion === 2 ? ['<all_urls>'] : [])]
        : ['debugger', 'tabs'],
    host_permissions: ['<all_urls>'],
    browser_specific_settings:
      browser === 'firefox' ? { gecko: { id: 'chromiumoxide@garysomerhalder', strict_min_version: '128.0' } } : undefined,
  }),
});
