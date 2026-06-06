// Drive the panel with a headless Chromium and capture screenshots, including
// authenticated views (logs in via the form). Reusable across phases.
// Env: CHROME (browser path), BASE (default localhost:9000), TOKEN, OUT (dir).
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

async function clickByText(page, selector, text) {
  const h = await page.evaluateHandle(
    (sel, t) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim() === t),
    selector,
    text,
  );
  const el = h.asElement();
  if (el) {
    await el.click();
    await sleep(350);
  }
  return !!el;
}

try {
  const page = await browser.newPage();
  await page.goto(`${BASE}/admin`, { waitUntil: 'networkidle0' });
  await page.waitForSelector('input[type=password]', { timeout: 6000 });
  await page.screenshot({ path: `${OUT}/smrt-login.png` });

  await clickByText(page, '.loc', 'EN');
  await page.screenshot({ path: `${OUT}/smrt-login-en.png` });
  await clickByText(page, '.loc', 'RU');

  await page.type('input[type=password]', TOKEN);
  await Promise.all([
    page.click('button[type=submit]'),
    page.waitForSelector('.rail .item', { timeout: 8000 }),
  ]);
  await sleep(600);
  await page.screenshot({ path: `${OUT}/smrt-overview.png` });

  // rail labels are localized; index them so the file names stay ASCII
  const rail = ['packs', 'servers', 'featured', 'cache'];
  for (let i = 0; i < rail.length; i++) {
    const items = await page.$$('.rail .item');
    if (items[i + 1]) {
      await items[i + 1].click();
      await sleep(350);
      await page.screenshot({ path: `${OUT}/smrt-${rail[i]}.png` });
    }
  }

  // pack editor: shows the calmer checkboxes + workspace
  const pItems = await page.$$('.rail .item');
  if (pItems[1]) {
    await pItems[1].click();
    await sleep(350);
  }
  const rowBtn = await page.$('tr.clickable');
  if (rowBtn) {
    await rowBtn.click();
    await sleep(500);
    await page.screenshot({ path: `${OUT}/smrt-pack-config.png` });
  }

  // English chrome on the shell
  await clickByText(page, '.loc', 'EN');
  const items = await page.$$('.rail .item');
  if (items[0]) {
    await items[0].click();
    await sleep(350);
  }
  await page.screenshot({ path: `${OUT}/smrt-overview-en.png` });

  console.log('shots ->', OUT);
} finally {
  await browser.close();
}
