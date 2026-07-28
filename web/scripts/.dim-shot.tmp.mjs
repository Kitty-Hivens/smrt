import puppeteer from 'puppeteer-core';
import { mkdirSync } from 'node:fs';

const BASE = process.env.BASE ?? 'http://127.0.0.1:9147';
const OUT = process.env.OUT ?? '/tmp/shots';
mkdirSync(OUT, { recursive: true });

const browser = await puppeteer.launch({
  executablePath: '/usr/bin/firefox',
  browser: 'firefox',
  headless: true,
  defaultViewport: { width: 1400, height: 900, deviceScaleFactor: 2 },
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
try {
  const page = await browser.newPage();
  await page.goto(`${BASE}/`, { waitUntil: 'load' });
  await page.evaluate((v) => {
    document.cookie = `smrt_session=${v}; path=/; SameSite=Strict`;
    localStorage.setItem('smrt.theme', 'light');
  }, process.env.SESSION ?? 'localshot');
  await page.goto(`${BASE}/packs/Testpack`, { waitUntil: 'load' });
  await sleep(2600);
  await page.screenshot({ path: `${OUT}/dim-editor.png` });
} finally {
  await browser.close();
}
