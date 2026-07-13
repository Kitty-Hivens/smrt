// Drive the panel with a headless Chromium and capture screenshots, including
// authenticated views (logs in via the form). Reusable across phases.
// Env: CHROME (browser path), BASE (default localhost:9000), TOKEN, OUT (dir),
//      WIDTHS (comma-separated px for the responsive sweep; see BP_WIDTHS).
import puppeteer from 'puppeteer-core';

const EXE = process.env.CHROME;
const BASE = process.env.BASE ?? 'http://127.0.0.1:9000';
const TOKEN = process.env.TOKEN ?? '';
const OUT = process.env.OUT ?? '/tmp';

// Viewport widths for the responsive breakpoint sweep. Defaults straddle both
// layout breaks: 320/375/560 -> phone drawer, 768 -> tablet strip, 1024/1440
// -> desktop sidebar (see --bp-sm 560 / --bp-md 768 in app.css).
const BP_WIDTHS = (process.env.WIDTHS ?? '320,375,560,768,1024,1440')
  .split(',')
  .map((s) => Number(s.trim()))
  .filter((n) => n > 0);

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

// Shoot the current view at every breakpoint width. CSS media queries relay out
// on resize client-side, so no reload is needed -- capture whatever is on screen
// (a list, the editor, or an open modal) as it reflows. deviceScaleFactor 1 keeps
// the sweep's file count cheap; the single-viewport shots above stay at 2.
async function sweep(page, name, height = 900) {
  for (const w of BP_WIDTHS) {
    await page.setViewport({ width: w, height, deviceScaleFactor: 1 });
    await sleep(300);
    await page.screenshot({ path: `${OUT}/bp-${name}-${w}.png` });
  }
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

  // ── responsive breakpoint sweeps ─────────────────────────────────────────
  // Navigate at desktop width (the rail collapses to a drawer on phones and is
  // no longer clickable there), then resize down and shoot. Files land as
  // bp-<view>-<width>.png next to the single-viewport shots above.
  const desktop = () => page.setViewport({ width: 1440, height: 900, deviceScaleFactor: 1 });

  // overview: rail (sidebar -> strip -> drawer) and the stat readout grid
  await desktop();
  {
    const nav = await page.$$('.rail .item');
    if (nav[0]) {
      await nav[0].click();
      await sleep(300);
    }
  }
  await sweep(page, 'overview');

  // packs: the wide operator table inside its .tablewrap scroll box
  await desktop();
  {
    const nav = await page.$$('.rail .item');
    if (nav[1]) {
      await nav[1].click();
      await sleep(300);
    }
  }
  await sweep(page, 'packs');

  // drawer open on a phone: the burger toggles the off-canvas rail over a scrim
  await page.setViewport({ width: 375, height: 812, deviceScaleFactor: 1 });
  await sleep(300);
  {
    const burger = await page.$('.burger');
    if (burger) {
      await burger.click();
      await sleep(350);
      await page.screenshot({ path: `${OUT}/bp-drawer-open-375.png` });
      await page.keyboard.press('Escape');
      await sleep(250);
    }
  }

  // pack editor config: the 8-column mod row and the 3-column basics grid reflow
  await desktop();
  {
    const nav = await page.$$('.rail .item');
    if (nav[1]) {
      await nav[1].click();
      await sleep(300);
    }
    const row = await page.$('tr.clickable');
    if (row) {
      await row.click();
      await sleep(600);
    }
  }
  await sweep(page, 'pack-config');

  // mirror picker modal: the filter row wraps on phones. Best-effort -- opens
  // from the mods section, skipped if that button is absent in this locale/pack.
  await desktop();
  if (
    (await clickByText(page, '.sm', 'From mirror')) ||
    (await clickByText(page, '.sm', 'С зеркала'))
  ) {
    try {
      await page.waitForSelector('.picker', { timeout: 4000 });
      await sleep(300);
      await sweep(page, 'mirror-picker');
    } catch {
      // picker did not open -- skip the modal sweep
    }
  }

  console.log('shots ->', OUT, '| breakpoints:', BP_WIDTHS.join(', '));
} finally {
  await browser.close();
}
