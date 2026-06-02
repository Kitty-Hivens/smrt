// Drive the pack editor: open a pack, screenshot config + curator, run a Build
// and wait for the live log to reach a terminal status. Verifies the whole
// authoring-in-GUI loop.
import puppeteer from 'puppeteer-core';

const EXE = process.env.CHROME;
const BASE = process.env.BASE ?? 'http://127.0.0.1:9000';
const TOKEN = process.env.TOKEN ?? '';
const OUT = process.env.OUT ?? '/tmp';

const browser = await puppeteer.launch({
  executablePath: EXE,
  headless: true,
  args: ['--no-sandbox', '--disable-gpu', '--hide-scrollbars'],
  defaultViewport: { width: 1280, height: 920, deviceScaleFactor: 2 },
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

  async function clickByText(selector, text) {
    const h = await page.evaluateHandle(
      (s, t) => [...document.querySelectorAll(s)].find((e) => e.textContent.trim() === t),
      selector,
      text,
    );
    const el = h.asElement();
    if (el) await el.click();
    return !!el;
  }

  await clickByText('.tab', 'Packs');
  await sleep(400);
  const edit = await page.$('td.actions button');
  await edit.click();
  await sleep(700);
  await page.screenshot({ path: `${OUT}/smrt-pack-config.png` });

  await clickByText('.seg', 'Curator');
  await sleep(400);
  await page.screenshot({ path: `${OUT}/smrt-pack-curator.png` });

  await clickByText('.seg', 'Build');
  await sleep(300);
  await page.evaluate(() => {
    const b = [...document.querySelectorAll('button')].find((x) =>
      x.textContent.trim().startsWith('Build pack'),
    );
    b?.click();
  });
  await page.waitForFunction(
    () => {
      const s = document.querySelector('.st');
      return s && ['done', 'failed'].includes(s.textContent.trim());
    },
    { timeout: 15000 },
  );
  await sleep(300);
  await page.screenshot({ path: `${OUT}/smrt-pack-build.png` });
  const status = await page.$eval('.st', (e) => e.textContent.trim());
  console.log('build status:', status);
} finally {
  await browser.close();
}
