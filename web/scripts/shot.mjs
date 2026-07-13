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

const BASE = (process.env.BASE ?? 'http://127.0.0.1:9000').replace(/\/+$/, '');
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
async function sweep(page, name, opts = {}) {
  const { height = 900, scrollTo = null } = opts;
  for (const w of BP_WIDTHS) {
    await page.setViewport({ width: w, height, deviceScaleFactor: 1 });
    await sleep(300);
    if (scrollTo) {
      // bring a below-the-fold element into view before the shot
      await page.evaluate((sel) => {
        document.querySelector(sel)?.scrollIntoView({ block: 'center' });
      }, scrollTo);
      await sleep(150);
    }
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

// Ask the API who we are from the page's own origin (so the cookie rides along).
// Returns { ok, detail } -- ok true only for an authenticated identity.
async function whoAmI(page) {
  return page.evaluate(async (base) => {
    try {
      const r = await fetch(`${base}/v1/me`, { credentials: 'include' });
      if (!r.ok) return { ok: false, detail: String(r.status) };
      const j = await r.json();
      return { ok: !!j.authenticated, detail: `${j.login} / ${j.role}` };
    } catch (e) {
      return { ok: false, detail: `fetch failed: ${e}` };
    }
  }, BASE);
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
    // We are on the origin (login page loaded). Put the session in the cookie
    // jar, then confirm against the API. If the jar entry is not honoured (some
    // driver/browser combos are finicky about it), fall back to document.cookie
    // on the origin -- the server only checks the cookie's value, so a plain,
    // non-HttpOnly cookie authenticates just as well.
    await injectSession(page, SESSION);
    let auth = await whoAmI(page);
    if (!auth.ok) {
      await page.evaluate((v) => {
        const secure = location.protocol === 'https:' ? '; Secure' : '';
        document.cookie = `smrt_session=${v}; path=/; SameSite=Strict${secure}`;
      }, SESSION);
      auth = await whoAmI(page);
    }
    if (!auth.ok) {
      await page.screenshot({ path: `${OUT}/bp-auth-failed.png` }).catch(() => {});
      console.error('\n-- not signed in --');
      console.error(`  /v1/me -> ${auth.detail}`);
      console.error('  401 => the SESSION value is stale or mistyped; copy a fresh smrt_session.');
      console.error('  also check BASE is the exact origin you copied the cookie from.');
      console.error(`  wrote ${OUT}/bp-auth-failed.png (what rendered instead of the panel).`);
      throw new Error('authentication failed; see diagnosis above');
    }
    console.log('signed in as', auth.detail);

    // Land on the app root: the SPA authenticates by cookie, so any served path
    // renders the shell (OAuth itself lands on "/"), and "/admin" is not its own
    // page -- the route lives in a store, not the URL. Drive it in English so
    // the rail labels are deterministic.
    await page.goto(`${BASE}/`, { waitUntil: 'networkidle0' });
    await page.waitForSelector('.rail .item', { timeout: 12000 });
    await clickByText(page, '.loc', 'EN');
    await sleep(300);

    // Navigate by rail label (English), not by a brittle index.
    const go = (label) => clickByText(page, '.item', label);

    // single-viewport reference shots
    await page.setViewport({ width: 1180, height: 760, deviceScaleFactor: DSF });
    await go('Overview');
    await page.screenshot({ path: `${OUT}/smrt-overview.png` });
    await go('Packs');
    await page.screenshot({ path: `${OUT}/smrt-packs.png` });

    // ── responsive breakpoint sweeps ───────────────────────────────────────
    // Navigate at desktop width (the rail collapses to a drawer on phones and is
    // no longer clickable there), then resize down and shoot. Files land as
    // bp-<view>-<width>.png next to the single-viewport shots above.
    const desktop = () => page.setViewport({ width: 1440, height: 900, deviceScaleFactor: 1 });

    // overview: rail (sidebar -> strip -> drawer) and the stat readout grid
    await desktop();
    await go('Overview');
    await sweep(page, 'overview');

    // packs: the wide operator table inside its .tablewrap scroll box
    await desktop();
    await go('Packs');
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

    // pack editor config: the 8-column mod row and the 3-column basics grid
    // reflow. Open the first pack row; skipped if there are no packs yet.
    await desktop();
    await go('Packs');
    const packRow = await page.$('tr.clickable');
    if (packRow) {
      await packRow.click();
      await sleep(700);
      await page.screenshot({ path: `${OUT}/smrt-pack-config.png` });
      await sweep(page, 'pack-config');
      // the mod row sits below the fold; scroll it into view at each width
      await sweep(page, 'modrow', { scrollTo: '.modrow' });

      // mirror picker modal: its filter row wraps on phones. Opens from the mods
      // section of the editor we are already in; best-effort.
      await desktop();
      if (await clickByText(page, '.sm', 'From mirror')) {
        try {
          await page.waitForSelector('.picker', { timeout: 4000 });
          await sleep(300);
          await sweep(page, 'mirror-picker');
        } catch {
          // picker did not open -- skip the modal sweep
        }
      }
    } else {
      console.log('no packs found -- skipped pack-config and picker sweeps.');
    }
  }

  console.log('shots ->', OUT, '| breakpoints:', BP_WIDTHS.join(', '));
} finally {
  await browser.close();
}
