// Renders the parity cases with Playwright. Usage: node render.js job.json
// job: { executablePath, args, outDir, cases: [{ name, context, html, style?, resize?, shots: [{ name, options }] }] }
const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

(async () => {
  const job = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
  const browser = await chromium.launch({ executablePath: job.executablePath, ignoreDefaultArgs: true, args: job.args });
  for (const c of job.cases) {
    const context = await browser.newContext(c.context);
    const page = await context.newPage();
    await page.setContent(c.html);
    if (c.style) await page.addStyleTag({ content: c.style });
    if (c.resize) await page.setViewportSize(c.resize);
    for (const s of c.shots) {
      await page.screenshot({ ...s.options, path: path.join(job.outDir, `${c.name}-${s.name}.png`) });
    }
    await context.close();
  }
  await browser.close();
})().catch((e) => { console.error(e); process.exit(1); });
