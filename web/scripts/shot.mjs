// Drive the panel with a headless Chromium and capture screenshots, including
// the authenticated operator views.
//
// Sign-in is GitHub OAuth now; the old token form is deprecated (it answers 410
// and opens no session), so it cannot log a headless browser in. Instead inject
// an existing session: sign in to the panel in a real browser, copy the
// `smrt_session` cookie value (DevTools > Application > Cookies -- it is
// HttpOnly, so document.cookie will not show it), and pass it as SESSION.
//
// Env: FIREFOX or CHROME (browser binary path -- one is required), BASE (default
//      localhost:9000), SESSION (the smrt_session cookie value; required for the
//      authenticated views), OUT (dir), WIDTHS (comma-separated px for the
//      responsive sweep; see BP_WIDTHS).
import puppeteer from 'puppeteer-core';
import { mkdirSync } from 'node:fs';

// Firefox is driven over WebDriver BiDi in its own throwaway profile, so it
// neither touches nor needs you to close a Firefox you already have open.
const FIREFOX = process.env.FIREFOX;
const CHROME = process.env.CHROME;
const useFirefox = !!FIREFOX;
const EXE = FIREFOX ?? CHROME;
if (!EXE) {
  console.error('set FIREFOX=/usr/bin/firefox (or CHROME=/path/to/chromium)');
  process.exit(1);
}
// Firefox over BiDi is most reliable at a 1x device pixel ratio; Chrome renders 2x.
const DSF = useFirefox ? 1 : 2;

const BASE = process.env.BASE ?? 'http://127.0.0.1:9000';
const SESSION = process.env.SESSION ?? '';
const OUT = process.env.OUT ?? '/tmp';
// page.screenshot() writes but does not create the directory -- ensure it exists.
mkdirSync(OUT, { recursive: true });

// Viewport widths for the responsive breakpoint sweep. Defaults straddle both
// layout breaks: 320/375/560 -> phone drawer, 768 -> tablet strip, 1024/1440
// -> desktop sidebar (see --bp-sm 560 / --bp-md 768 in app.css).
const BP_WIDTHS = (process.env.WIDTHS ?? '320,375,560,768,1024,1440')
  .split(',')
  .map((s) => Number(s.trim()))
  .filter((n) => n > 0);

const browser = await puppeteer.launch({
  executablePath: EXE,
  browser: useFirefox ? 'firefox' : 'chrome',
  headless: true,
  // Chrome-only flags; Firefox gets a fresh profile and needs none of them.
  args: useFirefox ? [] : ['--no-sandbox', '--disable-gpu', '--hide-scrollbars'],
  defaultViewport: { width: 1180, height: 760, deviceScaleFactor: DSF },
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
// the sweep's file count cheap; the single-viewport shots stay at 2.
async function sweep(page, name, height = 900) {
  for (const w of BP_WIDTHS) {
    await page.setViewport({ width: w, height, deviceScaleFactor: 1 });
    await sleep(300);
    await page.screenshot({ path: `${OUT}/bp-${name}-${w}.png` });
  }
}

// Inject the copied session cookie so the SPA (which authenticates purely by the
// `smrt_session` cookie via credentials:'include') comes up signed in. Firefox's
// BiDi backend wants an explicit `domain` (it will not derive one from `url` the
// way Chrome's CDP does), so pass the host directly.
async function injectSession(page, value) {
  const cookie = {
    name: 'smrt_session',
    value,
    domain: new URL(BASE).hostname,
    path: '/',
    httpOnly: true,
    secure: BASE.startsWith('https'),
    sameSite: 'Strict',
  };
  if (typeof browser.setCookie === 'function') await browser.setCookie(cookie);
  else await page.setCookie(cookie);
}

try {
  const page = await browser.newPage();

  // Login page first (unauthenticated): the GitHub button and locale chrome.
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle0' });
  await page.waitForSelector('.gh', { timeout: 6000 }).catch(() => {});
  await page.screenshot({ path: `${OUT}/smrt-login.png` });
  await clickByText(page, '.loc', 'EN');
  await page.screenshot({ path: `${OUT}/smrt-login-en.png` });
  await clickByText(page, '.loc', 'RU');
  await sweep(page, 'login');

  if (!SESSION) {
    console.log('SESSION not set -- captured the login page only.');
    console.log(
      'Sign in to the panel, copy the smrt_session cookie (DevTools > Application >',
    );
    console.log('Cookies), and re-run with SESSION=<value> for the authenticated views.');
  } else {
    await injectSession(page, SESSION);
    await page.goto(`${BASE}/admin`, { waitUntil: 'networkidle0' });
    try {
      await page.waitForSelector('.rail .item', { timeout: 8000 });
    } catch {
      throw new Error(
        'session cookie did not authenticate -- is SESSION current? sessions expire after 24h',
      );
    }
    await sleep(600);
    // back to the crisp default viewport for the single-shot captures
    await page.setViewport({ width: 1180, height: 760, deviceScaleFactor: DSF });
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

    // pack editor: the calmer checkboxes + workspace
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

    // ── responsive breakpoint sweeps ───────────────────────────────────────
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
  }

  console.log('shots ->', OUT, '| breakpoints:', BP_WIDTHS.join(', '));
} finally {
  await browser.close();
}
