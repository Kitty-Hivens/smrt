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

console.log(failures ? `\n${failures} failed` : '\nall good');
process.exit(failures ? 1 : 0);
