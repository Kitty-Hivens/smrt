// Drive the panel with chrome-headless-shell and capture screenshots,
// including authenticated views (logs in via the form). Reusable across
// phases. Env: CHROME (browser path), BASE (default localhost:9000), TOKEN,
// OUT (dir).
import puppeteer from 'puppeteer-core';

const EXE = process.env.CHROME;
const BASE = process.env.BASE ?? 'http://127.0.0.1:9000';
const TOKEN = process.env.TOKEN ?? '';
const OUT = process.env.OUT ?? '/tmp';

const browser = await puppeteer.launch({
  executablePath: EXE,
  headless: true,
  args: ['--no-sandbox', '--disable-gpu', '--hide-scrollbars'],
  defaultViewport: { width: 1180, height: 760, deviceScaleFactor: 2 },
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

try {
  const page = await browser.newPage();
  await page.goto(`${BASE}/admin`, { waitUntil: 'networkidle0' });
  await page.waitForSelector('input[type=password]', { timeout: 6000 });
  await page.screenshot({ path: `${OUT}/smrt-login.png` });

  await page.type('input[type=password]', TOKEN);
  await Promise.all([
    page.click('button[type=submit]'),
    page.waitForSelector('.tiles', { timeout: 8000 }),
  ]);
  await sleep(700);
  await page.screenshot({ path: `${OUT}/smrt-overview.png` });

  async function clickTab(label) {
    const h = await page.evaluateHandle(
      (l) => [...document.querySelectorAll('.tab')].find((b) => b.textContent.trim() === l),
      label,
    );
    const el = h.asElement();
    if (el) {
      await el.click();
      await sleep(350);
    }
  }

  await clickTab('Packs');
  await page.screenshot({ path: `${OUT}/smrt-packs.png` });

  await clickTab('Servers');
  await page.screenshot({ path: `${OUT}/smrt-servers.png` });

  const edit = await page.$('td.actions button');
  if (edit) {
    await edit.click();
    await sleep(350);
    await page.screenshot({ path: `${OUT}/smrt-server-editor.png` });
  }

  await clickTab('Featured');
  await page.screenshot({ path: `${OUT}/smrt-featured.png` });

  console.log('shots ->', OUT);
} finally {
  await browser.close();
}
