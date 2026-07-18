#!/usr/bin/env python3
"""Faithful port of dependency_fill_plan + apply_requires + derive_required
(the post-569bcc4 pipeline) over the harvested replica registry, to record the
required-set each pack would get on its next config save + build. Also dumps
side facts and the inferred-hard-edges-into-client-mods evidence for symptom 2.
"""
import json, sqlite3, os

BASE = os.path.dirname(os.path.abspath(__file__))
db = sqlite3.connect(os.path.join(BASE, '..', 'replica', 'registry.db'))

LOADER_DEPS = {"forge","minecraftforge","mod_minecraftforge","fml","forgemodloader",
               "neoforge","fabric","fabricloader","cleanroom","quilt"}

def is_loader_dep(target):
    return target.split('@')[0].lower() in LOADER_DEPS

def mod_id_for_selector(sel):
    sel = sel.split('@')[0]
    if sel.startswith('modrinth:'):
        r = db.execute("SELECT mod_id FROM mod_alias WHERE source='modrinth' AND external_key=?",
                       (sel[9:],)).fetchone()
    else:
        r = db.execute("SELECT mod_id FROM mod_alias WHERE source='modid' AND external_key=? COLLATE NOCASE",
                       (sel,)).fetchone()
    return r[0] if r else None

def relations_for_artifact(mv_id, mod_id):
    return db.execute(
        """SELECT target_modid, target_version_range, kind, source FROM relation
           WHERE from_mod_version_id = ? OR (from_mod_version_id IS NULL AND from_mod_id = ?)
           ORDER BY confidence DESC, id""", (mv_id, mod_id)).fetchall()

def side_of(sha):
    r = db.execute("SELECT side FROM mod_version WHERE sha1=?", (sha,)).fetchone()
    return r[0] if r else None

out = {}
for name in ['Create', 'Industrial']:
    m = json.load(open(os.path.join(BASE, f'{name}.json')))
    mods = m['mods']
    # place each mod on the graph (by sha1 -- every manifest jar is harvested)
    placed = []   # (filename, default_enabled, mod_id, mv_id)
    for e in mods:
        r = db.execute("SELECT id, mod_id FROM mod_version WHERE sha1=?", (e['sha1'],)).fetchone()
        if r:
            placed.append((e['filename'], e.get('default_enabled', True), r[1], r[0], e['sha1']))
    by_mod = {}
    for i, p in enumerate(placed):
        by_mod.setdefault(p[2], i)

    # dependency_fill_plan: authoritative requires edge per target
    requires = []   # (from filename, dep filename, edge source)
    missing = set()
    for fn, dflt, mod_id, mv_id, sha in placed:
        seen = set()
        for target, rng, kind, src in relations_for_artifact(mv_id, mod_id):
            if target in seen:
                continue
            seen.add(target)
            if kind != 'requires':
                continue
            if is_loader_dep(target):
                continue
            tid = mod_id_for_selector(target)
            if tid is not None and tid in by_mod:
                requires.append((fn, placed[by_mod[tid]][0], src))
            elif tid is None or tid not in by_mod:
                missing.add(target)

    # derive_required: BFS from default-enabled seeds over hard edges
    idx = {p[0]: i for i, p in enumerate(placed)}
    hard = {}
    for f, d, src in requires:
        hard.setdefault(f, []).append(d)
    req = set()
    queue = [d for p in placed if p[1] for d in hard.get(p[0], [])]
    while queue:
        f = queue.pop()
        if f not in req:
            req.add(f)
            queue.extend(hard.get(f, []))

    rows = []
    for fn, dflt, mod_id, mv_id, sha in placed:
        drivers = sorted({a for a, b, s in requires if b == fn})
        rows.append({
            'filename': fn, 'default_enabled': dflt,
            'required_next_build': fn in req,
            'published_required': next(e.get('required', True) for e in mods if e['filename'] == fn),
            'side': side_of(sha),
            'required_by': drivers if fn in req else [],
        })
    out[name] = {'mods': rows, 'missing': sorted(missing),
                 'edges': [(a, b, s) for a, b, s in requires]}

# symptom 2 evidence: inferred hard relation rows whose target resolves to a
# mod whose artifacts are side=client
evidence = []
for target, src, from_mod in db.execute(
        "SELECT DISTINCT target_modid, source, from_mod_id FROM relation WHERE kind='requires'"):
    tid = mod_id_for_selector(target)
    if tid is None:
        continue
    sides = {s for (s,) in db.execute("SELECT side FROM mod_version WHERE mod_id=?", (tid,)) if s}
    if sides == {'client'}:
        fname = db.execute("SELECT COALESCE(canonical_name, slug) FROM mods WHERE id=?",
                           (from_mod,)).fetchone()[0]
        evidence.append({'from': fname or f'#{from_mod}', 'target': target, 'source': src})
out['hard_edges_into_client_mods'] = evidence

json.dump(out, open(os.path.join(BASE, 'baseline.json'), 'w'), indent=1)

for name in ['Create', 'Industrial']:
    rows = out[name]['mods']
    nreq = sum(1 for r in rows if r['required_next_build'])
    flips = [(r['filename'], r['published_required'], r['required_next_build'], r['side'])
             for r in rows if r['published_required'] != r['required_next_build']]
    print(f"{name}: placed={len(rows)} required_next_build={nreq} "
          f"published_required={sum(1 for r in rows if r['published_required'])} flips={len(flips)}")
    for f, p, n, s in flips:
        print(f"   {f:<46} published={str(p):<6} next={str(n):<6} side={s}")
    print(f"  missing (would depfill-pull): {out[name]['missing']}")
print("\nhard requires edges into client-side mods:")
for e in evidence:
    print(f"   {e['from']} -> {e['target']}  [{e['source']}]")
