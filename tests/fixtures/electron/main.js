// A minimal Electron app for the integration tests.
const { app, BrowserWindow } = require('electron');

app.whenReady().then(() => {
  const win = new BrowserWindow({ width: 400, height: 300 });
  win.loadURL('data:text/html,<title>Hello</title><h1>Electron</h1>');
});

app.on('window-all-closed', () => app.quit());
