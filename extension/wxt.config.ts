import { defineConfig } from 'wxt';

export default defineConfig({
  manifest: ({ browser }) => ({
    name: 'chromiumoxide',
    description: 'Lets chromiumoxide drive the browser you already use.',
    permissions: browser === 'firefox' || browser === 'safari'
      ? ['tabs', 'scripting', 'activeTab', '<all_urls>']
      : ['debugger', 'tabs'],
    host_permissions: ['<all_urls>'],
  }),
});
