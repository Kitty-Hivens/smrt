// Seed a deliberately rich pack, publish v1, stage a changed v2, then open the
// pack editor -> Preview split-pane and screenshot the launcher-faithful render
// (hero, role groups, dep tree, conflicts, libraries, assets, markdown about)
// plus the version diff. Exercises every Phase 5 code path offline (smrt_cache /
// smrt_static sources -- no Modrinth network needed).
import puppeteer from 'puppeteer-core';
import { createHash } from 'node:crypto';

const EXE = process.env.CHROME;
const BASE = process.env.BASE ?? 'http://127.0.0.1:9000';
const TOKEN = process.env.TOKEN ?? 'dev';
const OUT = process.env.OUT ?? '/tmp';
const PACK = 'Preview';

const auth = { Authorization: `Bearer ${TOKEN}` };
const sha1 = (s) => createHash('sha1').update(s).digest('hex');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function putJar(content) {
  const h = sha1(content);
  const r = await fetch(`${BASE}/v1/admin/cache/${h.slice(0, 2)}/${h}.jar`, {
    method: 'PUT',
    headers: { ...auth, 'Content-Type': 'application/java-archive' },
    body: content,
  });
  if (!r.ok) throw new Error(`putJar ${h}: ${r.status} ${await r.text()}`);
  return h;
}

async function putStatic(rel, content) {
  const r = await fetch(`${BASE}/v1/admin/packs/${PACK}/static/${rel}`, {
    method: 'PUT',
    headers: { ...auth, 'Content-Type': 'application/octet-stream' },
    body: content,
  });
  if (!r.ok) throw new Error(`putStatic ${rel}: ${r.status} ${await r.text()}`);
}

async function putConfig(cfg) {
  const r = await fetch(`${BASE}/v1/admin/packs/${PACK}/config`, {
    method: 'PUT',
    headers: { ...auth, 'Content-Type': 'application/json' },
    body: JSON.stringify(cfg),
  });
  if (!r.ok) throw new Error(`putConfig: ${r.status} ${await r.text()}`);
}

async function putCurator(toml) {
  const r = await fetch(`${BASE}/v1/admin/packs/${PACK}/curator`, {
    method: 'PUT',
    headers: { ...auth, 'Content-Type': 'text/plain; charset=utf-8' },
    body: toml,
  });
  if (!r.ok) throw new Error(`putCurator: ${r.status} ${await r.text()}`);
}

async function build(dryRun) {
  const q = dryRun ? '?dry_run=true' : '';
  const r = await fetch(`${BASE}/v1/admin/packs/${PACK}/build${q}`, { method: 'POST', headers: auth });
  const { job_id } = await r.json();
  for (let i = 0; i < 300; i++) {
    const s = await (await fetch(`${BASE}/v1/admin/jobs/${job_id}`, { headers: auth })).json();
    if (s.status !== 'running') return s;
    await sleep(100);
  }
  throw new Error('build timeout');
}

const mod = (filename, sha, required, defaultEnabled, display) => ({
  filename,
  required,
  default_enabled: defaultEnabled,
  source: { type: 'smrt_cache', sha1: sha },
  display,
});

// ── seed ────────────────────────────────────────────────────────────────────
const J = {};
for (const k of ['JEI', 'JEI2', 'REI', 'JM', 'XA', 'AS', 'MB', 'QK', 'CA', 'CB']) {
  J[k] = await putJar(`jar:${k}:demo-content`);
}
await putStatic('Faithful.zip', 'PK fake resourcepack bytes');

const baseMods = [
  mod('JustEnoughItems.jar', J.JEI, true, true, {
    name: 'Just Enough Items',
    category: 'interface',
    role: 'recipe_viewer',
    description: 'Recipe and usage lookup for every item.',
  }),
  mod('RoughlyEnoughItems.jar', J.REI, false, false, {
    name: 'Roughly Enough Items',
    category: 'interface',
    role: 'recipe_viewer',
    description: 'Alternative recipe viewer.',
  }),
  mod('JourneyMap.jar', J.JM, false, true, {
    name: 'JourneyMap',
    category: 'map',
    role: 'minimap',
    incompatible_with: ['Xaeros.jar'],
  }),
  mod('Xaeros.jar', J.XA, false, false, {
    name: "Xaero's Minimap",
    category: 'map',
    role: 'minimap',
    incompatible_with: ['JourneyMap.jar'],
  }),
  mod('AppleSkin.jar', J.AS, true, true, {
    name: 'AppleSkin',
    category: 'tweaks',
    requires: [{ filename: 'Mixinbooter.jar', version_range: '>=8.0' }],
  }),
  mod('Mixinbooter.jar', J.MB, true, true, { name: 'MixinBooter', category: 'library' }),
  mod('Quark.jar', J.QK, true, true, {
    name: 'Quark',
    category: 'content',
    requires: [{ filename: 'AutoRegLib.jar' }], // intentionally missing -> warning
  }),
];

const cfgBase = {
  pack_id: PACK,
  display_name: 'Preview Pack',
  tagline: 'Exercising the launcher-faithful preview.',
  minecraft_version: '1.12.2',
  loader: { name: 'forge', version: '14.23.5.2860' },
  java_major: 8,
  tags: ['tech', 'demo'],
  featured: true,
  mods: baseMods,
  assets: [
    {
      dest: 'resourcepacks/Faithful.zip',
      required: false,
      source: { type: 'smrt_static', rel_path: 'Faithful.zip' },
      display: { name: 'Faithful 32x' },
    },
  ],
};

await putConfig(cfgBase);
await putCurator(`[pack_meta]
description_md = """
# Preview Pack

A **launcher-faithful** preview, rendered from the resolved manifest.

- role-grouped recipe viewers and minimaps
- a missing dependency and a dependency *cycle*
- an incompatible minimap pair

See \`config.json\` for the authored source.
"""
`);

console.log('build v1:', (await build(false)).status);

// v2: change JEI's jar (updated) + add a dependency-cycle pair (added).
const cfgV2 = {
  ...cfgBase,
  mods: [
    mod('JustEnoughItems.jar', J.JEI2, true, true, baseMods[0].display),
    ...baseMods.slice(1),
    mod('CycleA.jar', J.CA, false, true, {
      name: 'Cycle A',
      requires: [{ filename: 'CycleB.jar' }],
    }),
    mod('CycleB.jar', J.CB, false, true, {
      name: 'Cycle B',
      requires: [{ filename: 'CycleA.jar' }],
    }),
  ],
};
await putConfig(cfgV2);
console.log('seeded; v2 staged (unpublished).');

// ── shot ──────────────────────────────────────────────────────────────────────
const browser = await puppeteer.launch({
  executablePath: EXE,
  headless: true,
  args: ['--no-sandbox', '--disable-gpu', '--hide-scrollbars'],
  defaultViewport: { width: 1440, height: 1200, deviceScaleFactor: 2 },
});

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
  await sleep(500);
  await page.evaluate(() =>
    [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Preview')?.click(),
  );
  await page.waitForSelector('.preview .hero', { timeout: 30000 });
  await sleep(900);
  await page.screenshot({ path: `${OUT}/smrt-preview.png`, fullPage: true });

  // Expand every dependency tree + the diff details for a second shot.
  await page.evaluate(() => {
    document.querySelectorAll('.preview .exp').forEach((b) => b.click());
    document.querySelectorAll('.preview .link').forEach((b) => b.click());
    document.querySelectorAll('.preview .sechead').forEach((b) => b.click());
  });
  await sleep(500);
  await page.screenshot({ path: `${OUT}/smrt-preview-expanded.png`, fullPage: true });

  const sections = await page.$$eval('.preview section', (els) => els.length);
  const warns = await page.$$eval('.preview .warn-line', (els) => els.map((e) => e.textContent.trim()));
  console.log('preview sections:', sections);
  console.log('resolver warnings:', JSON.stringify(warns, null, 2));
} finally {
  await browser.close();
}
