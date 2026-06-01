// Drive a real cache-jar upload through the panel: login, go to Cache,
// pick a file, confirm it lands. Verifies the client-side SHA-1 + PUT path.
import puppeteer from 'puppeteer-core';

const EXE = process.env.CHROME;
const BASE = process.env.BASE ?? 'http://127.0.0.1:9000';
const TOKEN = process.env.TOKEN ?? '';
const JAR = process.env.JAR;
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
  await page.type('input[type=password]', TOKEN);
  await Promise.all([
    page.click('button[type=submit]'),
    page.waitForSelector('.tiles', { timeout: 8000 }),
  ]);
  const cacheTab = await page.evaluateHandle(() =>
    [...document.querySelectorAll('.tab')].find((b) => b.textContent.trim() === 'Cache'),
  );
  await cacheTab.asElement().click();
  await sleep(300);
  const input = await page.$('input[type=file]');
  await input.uploadFile(JAR);
  await sleep(1400);
  await page.screenshot({ path: `${OUT}/smrt-cache.png` });
  const head = await page.$eval('.cache-head', (e) => e.textContent.trim());
  console.log('cache-head:', head);
} finally {
  await browser.close();
}
