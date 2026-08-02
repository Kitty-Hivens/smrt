// The panel's logic that fails silently, checked here rather than in a
// browser: the merge (#115), and what Java a pack needs (#126).
//
// The merge half:
//
// The editor keeps plain `bind:value` controls over a plain config object, so
// every keystroke arrives here as a whole new config. Turning that into the
// smallest edit is what decides whether two people typing in one paragraph
// merge or overwrite each other -- assigning a whole string to a text node is a
// delete of everything followed by an insert of everything, and the other
// person's sentence is inside the range being deleted.
//
// Plain node, no framework: the panel has no test runner, and this is one file
// of pure functions. `node web/scripts/merge-check.mjs`, or `pnpm merge-check`.
import * as Y from 'yjs';
import { readConfig, textPatch, writeConfig } from '../src/lib/packdoc.ts';
import { JAVA_MAJORS, suggestedJava } from '../src/lib/java.ts';
import { changedPaths } from '../src/lib/touched.svelte.ts';
import { advertisesModList } from '../src/lib/handshake.ts';
import { assetPath, isPackFile, ASSET_PREFIX } from '../src/lib/packassets.ts';
import { nextPageUrl } from '../src/lib/pagelink.ts';
import { diffConfigs } from '../src/lib/configdiff.ts';

let failures = 0;
const check = (name, cond, detail = '') => {
  if (cond) console.log(`ok   ${name}`);
  else {
    failures++;
    console.log(`FAIL ${name} ${detail}`);
  }
};

const base = {
  pack_id: 'Industrial', display_name: 'Industrial', tagline: 'Heavy tech',
  minecraft_version: '1.12.2', loader: { name: 'forge', version: '14.23.5.2860' },
  java_major: 8, version: '0.4', tags: ['tech'], featured: true,
  mods: [{ filename: 'jei.jar', default_enabled: true, source: { type: 'smrt_cache', sha1: 'a'.repeat(40) }, pulled: false }],
  assets: [], pack_meta: { description_md: 'A pack.', gallery_urls: [] },
  owner: 211033194, tier: 'community', visibility: 'draft', fork_of: 'Create',
};

// ── the patch itself ────────────────────────────────────────────────────────
check('append is a patch at the end',
  JSON.stringify(textPatch('abc', 'abcd')) === JSON.stringify({ index: 3, remove: 0, insert: 'd' }));
check('backspace removes one at a point',
  JSON.stringify(textPatch('abcd', 'abd')) === JSON.stringify({ index: 2, remove: 1, insert: '' }));
check('typing in the middle inserts there',
  JSON.stringify(textPatch('ad', 'abcd')) === JSON.stringify({ index: 1, remove: 0, insert: 'bc' }));
check('no change is no patch', textPatch('same', 'same') === null);
{
  // property check: applying the patch must reproduce the target, always
  const words = ['', 'a', 'ab', 'abc', 'hello world', 'hello brave world', 'held', 'hello worlds'];
  let ok = true;
  for (const from of words) for (const to of words) {
    const p = textPatch(from, to);
    const out = p === null ? from : from.slice(0, p.index) + p.insert + from.slice(p.index + p.remove);
    if (out !== to) { ok = false; console.log(`  ${JSON.stringify(from)} -> ${JSON.stringify(to)} gave ${JSON.stringify(out)}`); }
  }
  check('every patch reproduces its target', ok);
}

// ── two editors ─────────────────────────────────────────────────────────────
function editor(seed) {
  const doc = new Y.Doc();
  Y.applyUpdate(doc, seed);
  return doc;
}
const server = new Y.Doc();
writeConfig(server, base, 'seed');
const seed = Y.encodeStateAsUpdate(server);

{
  // both typing into the same paragraph, neither having seen the other
  const ada = editor(seed);
  const bo = editor(seed);
  const a = structuredClone(base);
  a.pack_meta.description_md = 'A pack. Heavy tech.';       // appended
  writeConfig(ada, a, 'local');
  const b = structuredClone(base);
  b.pack_meta.description_md = 'The A pack.';               // prefixed
  writeConfig(bo, b, 'local');

  Y.applyUpdate(ada, Y.encodeStateAsUpdate(bo));
  Y.applyUpdate(bo, Y.encodeStateAsUpdate(ada));
  const text = readConfig(ada, base).pack_meta.description_md;
  check('both people keep their words', text.includes('Heavy tech.') && text.startsWith('The '), `got ${JSON.stringify(text)}`);
  check('and the two editors agree', text === readConfig(bo, base).pack_meta.description_md);
}

{
  // both adding a mod, neither having seen the other
  const ada = editor(seed);
  const bo = editor(seed);
  const a = structuredClone(base);
  a.mods.push({ filename: 'ae2.jar', default_enabled: true, source: { type: 'smrt_cache', sha1: 'b'.repeat(40) }, pulled: false });
  writeConfig(ada, a, 'local');
  const b = structuredClone(base);
  b.mods.push({ filename: 'thermal.jar', default_enabled: true, source: { type: 'smrt_cache', sha1: 'c'.repeat(40) }, pulled: false });
  writeConfig(bo, b, 'local');

  Y.applyUpdate(ada, Y.encodeStateAsUpdate(bo));
  const names = readConfig(ada, base).mods.map((m) => m.filename);
  check('both additions land', names.length === 3 && names.includes('ae2.jar') && names.includes('thermal.jar'), JSON.stringify(names));
}

{
  // one person renames a mod while the other edits the prose: different things,
  // no collision, which is the everyday case the old save refused
  const ada = editor(seed);
  const bo = editor(seed);
  const a = structuredClone(base);
  a.mods[0].filename = 'jei-4.16.jar';
  writeConfig(ada, a, 'local');
  const b = structuredClone(base);
  b.tagline = 'Heavy tech, now heavier';
  writeConfig(bo, b, 'local');

  Y.applyUpdate(ada, Y.encodeStateAsUpdate(bo));
  const out = readConfig(ada, base);
  check('a rename and a retagline both survive',
    out.mods[0].filename === 'jei-4.16.jar' && out.tagline === 'Heavy tech, now heavier',
    JSON.stringify([out.mods[0].filename, out.tagline]));
}

{
  // the server's fields never travel, and the editor keeps what it loaded
  const doc = editor(seed);
  const out = readConfig(doc, base);
  check('server fields come from the loaded config',
    out.owner === base.owner && out.visibility === 'draft' && out.fork_of === 'Create');
  check('and are absent from the document itself',
    !['owner', 'tier', 'visibility', 'fork_of'].some((k) => doc.getMap('config').has(k)));
}

{
  // a full round trip, so the mapping is not quietly lossy
  const doc = editor(seed);
  check('a config round-trips', JSON.stringify(readConfig(doc, base)) === JSON.stringify(base),
    JSON.stringify(readConfig(doc, base)));
}

// ── which Java a pack needs ─────────────────────────────────────────────────
// Every one of these is a pack this mirror actually serves.
check('1.12.2 forge wants 8', suggestedJava('1.12.2', 'forge') === 8);
check('1.21.1 neoforge wants 21', suggestedJava('1.21.1', 'neoforge') === 21);
check('1.7.10 on lwjgl3ify wants 21, not 8',
  suggestedJava('1.7.10', 'lwjgl3ify') === 21,
  'the loader exists to run old Minecraft on new Java; deriving from the version alone gets this wrong');
check('cleanroom is the same kind of loader', suggestedJava('1.12.2', 'cleanroom') === 21);
check('1.17 wants 16', suggestedJava('1.17', 'forge') === 16);
check('1.18.2 wants 17', suggestedJava('1.18.2', 'forge') === 17);
check('1.20.4 still wants 17', suggestedJava('1.20.4', 'forge') === 17);
check('1.20.5 moves to 21', suggestedJava('1.20.5', 'forge') === 21);
check('versions compare piecewise, not as strings',
  suggestedJava('1.9.4', 'forge') === 8,
  'lexically "1.9" > "1.18", which is how a naive compare puts 1.9 on Java 17');
check('an unparseable version suggests nothing', suggestedJava('', 'forge') === null && suggestedJava('snapshot', 'forge') === null);
check('the offered list holds what old packs need', [8, 11, 16, 17, 21].every((v) => JAVA_MAJORS.includes(v)));

// ── who changed what ────────────────────────────────────────────────────────
// A marker is only useful if it points at the thing that moved. Reporting the
// whole config, or the whole mods list, is an address nobody can act on.
{
  const base = {
    display_name: 'Industrial',
    pack_meta: { description_md: 'A pack.', gallery_urls: [] },
    mods: [{ filename: 'jei.jar', default_enabled: true }, { filename: 'ae2.jar', default_enabled: true }],
  };
  const clone = () => JSON.parse(JSON.stringify(base));

  const scalar = clone(); scalar.display_name = 'Industrial II';
  check('a changed scalar is reported at its own path',
    JSON.stringify(changedPaths(base, scalar)) === '["display_name"]',
    JSON.stringify(changedPaths(base, scalar)));

  const nested = clone(); nested.pack_meta.description_md = 'A pack. Heavy tech.';
  check('a nested field is reported at its full path',
    JSON.stringify(changedPaths(base, nested)) === '["pack_meta.description_md"]',
    JSON.stringify(changedPaths(base, nested)));

  const row = clone(); row.mods[1].default_enabled = false;
  check('an edited row is reported as the row, not the whole list',
    JSON.stringify(changedPaths(base, row)) === '["mods.1"]',
    JSON.stringify(changedPaths(base, row)));

  const added = clone(); added.mods.push({ filename: 'thermal.jar' });
  check('an added row is one path, not every field in it',
    JSON.stringify(changedPaths(base, added)) === '["mods.2"]',
    JSON.stringify(changedPaths(base, added)));

  const removed = clone(); removed.mods.pop();
  check('a removed row is reported too', JSON.stringify(changedPaths(base, removed)) === '["mods.1"]',
    JSON.stringify(changedPaths(base, removed)));

  check('an unchanged config reports nothing', changedPaths(base, clone()).length === 0);

  const two = clone();
  two.display_name = 'X'; two.mods[0].default_enabled = false;
  check('two independent changes are two paths',
    JSON.stringify(changedPaths(base, two).sort()) === '["display_name","mods.0"]',
    JSON.stringify(changedPaths(base, two)));
}

// #148: whether a handshake claim can be derived at all, from the loader alone.
{
  check('a 1.12.2 forge server advertises its mod list', advertisesModList('forge') === true);
  check('so does a fork that inherits one', advertisesModList('cleanroom') === true);
  check('and the modernised 1.7.10 loader', advertisesModList('lwjgl3ify') === true);
  // the case that sends people pressing a button that cannot work
  check('a neoforge server advertises nothing', advertisesModList('neoforge') === false);
  check('nor does fabric', advertisesModList('fabric') === false);
  check('a loader nobody named is not assumed to advertise', advertisesModList('') === false);
  check('the answer does not depend on spelling', advertisesModList('  NeoForge ') === false);
}

// A pack's own files are named for the pack, not for one launcher, and the old
// name keeps resolving for every pack that already uses it.
{
  check('new files are minted under the neutral prefix',
    assetPath('icon.png') === '_pack/icon.png', assetPath('icon.png'));
  check('nested ones too', assetPath('assets', 'servers.dat') === '_pack/assets/servers.dat');
  check('the prefix names no launcher', !/nexira|smrt/i.test(ASSET_PREFIX), ASSET_PREFIX);
  check('it stays out of the way of game directories', ASSET_PREFIX.startsWith('_'));

  // the sweep that keeps an icon resolving to one file has to see both names,
  // or a re-upload leaves the old image behind under the old prefix
  check('an icon is recognised under the new prefix', isPackFile('_pack/icon.png', 'icon'));
  check('and under the old one', isPackFile('_nexira/icon.webp', 'icon'));
  check('a banner is not an icon', !isPackFile('_pack/banner.png', 'icon'));
  check('and a lookalike elsewhere is neither',
    !isPackFile('resourcepacks/icon.png', 'icon'));
}

// Walking a paged listing means following the address the mirror hands back. Read
// it wrong and the walk stops at the first page while looking like it finished.
{
  const link = '</v1/audit?limit=60&after=NDI>; rel="next"';
  check('the next page is the address it names',
    nextPageUrl(link) === '/v1/audit?limit=60&after=NDI', nextPageUrl(link));
  check('the last page names nothing after it', nextPageUrl(null) === null);
  check('a header with no next in it yields none',
    nextPageUrl('</v1/audit?limit=60>; rel="prev"') === null);
  check('the next is found among other relations',
    nextPageUrl('</a>; rel="prev", </b>; rel="next"') === '/b');
  // the cursor is base64url, so it can carry characters a naive split would eat
  check('a cursor is taken whole',
    nextPageUrl('</v1/registry/mods?q=a-b_c&after=eyJhIjoxfQ>; rel="next"')
      === '/v1/registry/mods?q=a-b_c&after=eyJhIjoxfQ');
}

// ── what a commit is about to record ────────────────────────────────────────
// The commit box showed a count of changed JSON paths and nothing else, so its
// message was written from memory. The count is also not a list of things a
// person did: the dependency fill writes `display.requires` on save, and a save
// that changed nothing else still reported 22.
{
  const cfg = () => JSON.parse(JSON.stringify({
    ...base,
    mods: [
      { filename: 'jei.jar', default_enabled: true, source: { type: 'modrinth', project_id: 'u6dRKJwZ', version_id: 'sc43sMLj' } },
      { filename: 'sodium.jar', default_enabled: true, source: { type: 'smrt_cache', sha1: 'b'.repeat(40) } },
    ],
    assets: [{ dest: 'config/a.json', required: true, source: { type: 'smrt_static', rel_path: 'config/a.json' }, display: { name: 'A' } }],
  }));
  const rows = (b) => diffConfigs(cfg(), b).map((r) => `${r.group}/${r.op}/${r.label}`);

  check('an unchanged config has nothing to say', rows(cfg()).length === 0);

  const derived = cfg();
  derived.mods[0].display = { requires: [{ filename: 'sodium.jar', optional: false }], presence: 'required' };
  derived.mods[1].pulled = true;
  check('what the mirror fills in is not a change anyone made', rows(derived).length === 0,
    JSON.stringify(rows(derived)));

  const authored = cfg();
  authored.assets[0].display.description = 'what it does';
  check('what a curator writes is', rows(authored).join() === 'assets/change/config/a.json');

  const moved = cfg();
  moved.mods[0].source.version_id = 'bqMxf6Ua';
  const pinRow = diffConfigs(cfg(), moved)[0];
  check('a moved pin carries both ends', pinRow.from === 'sc43sMLj' && pinRow.to === 'bqMxf6Ua');
  // the row names the project so the view can replace the ids with version
  // numbers -- `P4yXqsnw -> bqMxf6Ua` tells a reader nothing
  check('and the project whose labels can replace them', pinRow.project === 'u6dRKJwZ');

  const swapped = cfg();
  swapped.mods = [swapped.mods[1], { filename: 'iris.jar', default_enabled: false, source: { type: 'smrt_cache', sha1: 'c'.repeat(40) } }];
  check('an arrival and a departure are their own rows',
    rows(swapped).join() === 'mods/add/iris.jar,mods/remove/jei.jar', JSON.stringify(rows(swapped)));

  const toggled = cfg();
  toggled.mods[1].default_enabled = false;
  check('so is the install default a player gets', rows(toggled).join() === 'mods/change/sodium.jar');

  const loader = cfg();
  loader.loader = { name: 'neoforge', version: '21.1.248' };
  const loaderRow = diffConfigs(cfg(), loader)[0];
  check('the loader reads as a loader, not as two fields',
    loaderRow.from === 'forge 14.23.5.2860' && loaderRow.to === 'neoforge 21.1.248');
}

console.log(failures ? `\n${failures} failed` : '\nall good');
process.exit(failures ? 1 : 0);
