// Open a pack -> Curator section (Structured view is default), screenshot it.
import puppeteer from 'puppeteer-core';

const EXE = process.env.CHROME;
const BASE = process.env.BASE ?? 'http://127.0.0.1:9000';
const TOKEN = process.env.TOKEN ?? '';
const OUT = process.env.OUT ?? '/tmp';

const browser = await puppeteer.launch({
  executablePath: EXE,
  headless: true,
  args: ['--no-sandbox', '--disable-gpu', '--hide-scrollbars'],
  defaultViewport: { width: 1280, height: 940, deviceScaleFactor: 2 },
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

try {
  const page = await browser.newPage();
  await page.goto(`${BASE}/admin`, { waitUntil: 'networkidle0' });
  await page.waitForSelector('input[type=password]', { timeout: 6000 });
  await page.type('input[type=password]', TOKEN);
  await Promise.all([
    page.click('button[type=submit]'),
    page.waitForSelector('.tiles', { timeout: 8000 }),
  ]);
  await page.evaluate(() =>
    [...document.querySelectorAll('.tab')].find((b) => b.textContent.trim() === 'Packs')?.click(),
  );
  await sleep(400);
  await page.click('td.actions button');
  await sleep(600);
  await page.evaluate(() =>
    [...document.querySelectorAll('.seg')].find((b) => b.textContent.trim() === 'Curator')?.click(),
  );
  await sleep(700);
  await page.screenshot({ path: `${OUT}/smrt-curator-structured.png` });
  const rows = await page.$$eval('tbody tr', (els) => els.length);
  console.log('per-mod rows:', rows);
} finally {
  await browser.close();
}
