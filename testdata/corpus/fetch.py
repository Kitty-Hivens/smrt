#!/usr/bin/env python3
"""Fetch the golden corpus: every jar of every published pack on the mirror,
plus Modrinth version objects (dependencies) and project objects (env flags).
Builds a local storage replica mirroring prod layout for `smrt-pack registry harvest`.
"""
import hashlib
import json
import os
import sys
import time
import urllib.request

BASE = os.path.dirname(os.path.abspath(__file__))
JARS = os.path.join(BASE, "jars")
REPLICA = os.path.join(BASE, "..", "replica")
UA = "smrt-corpus-audit/1.0 (dev tooling)"

os.makedirs(JARS, exist_ok=True)


def get(url, retries=3, timeout=120):
    last = None
    for i in range(retries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return r.read()
        except Exception as e:  # noqa: BLE001
            last = e
            time.sleep(2 * (i + 1))
    raise RuntimeError(f"GET {url} failed: {last}")


def post_json(url, body, retries=3, timeout=120):
    last = None
    data = json.dumps(body).encode()
    for i in range(retries):
        try:
            req = urllib.request.Request(
                url, data=data,
                headers={"User-Agent": UA, "Content-Type": "application/json"},
            )
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read())
        except Exception as e:  # noqa: BLE001
            last = e
            time.sleep(2 * (i + 1))
    raise RuntimeError(f"POST {url} failed: {last}")


packs = json.load(open(os.path.join(BASE, "packs.json")))["packs"]
manifests = {}
for p in packs:
    pid = p["pack_id"]
    manifests[pid] = json.load(open(os.path.join(BASE, f"{pid}.json")))

# ── collect every mod entry, dedupe by sha1 ─────────────────────────────────
mods = {}  # sha1 -> meta
for pid, m in manifests.items():
    for e in m["mods"]:
        sha = e["sha1"]
        meta = mods.setdefault(sha, {
            "sha1": sha,
            "filename": e["filename"],
            "size_bytes": e["size_bytes"],
            "source": e["source"],
            "packs": [],
        })
        meta["packs"].append({
            "pack": pid,
            "required": e.get("required", True),
            "default_enabled": e.get("default_enabled", True),
            "display": e.get("display"),
            "slug": e.get("slug"),
        })
print(f"{len(mods)} distinct jars across {len(packs)} packs")

# ── Modrinth: version objects by sha1 (all of them, incl. cache jars: a cache
#    jar whose bytes Modrinth knows gets an identity, same as harvest does) ──
all_shas = sorted(mods)
versions_by_sha = {}
for i in range(0, len(all_shas), 100):
    chunk = all_shas[i:i + 100]
    got = post_json("https://api.modrinth.com/v2/version_files",
                    {"hashes": chunk, "algorithm": "sha1"})
    versions_by_sha.update(got)
    time.sleep(0.5)
print(f"modrinth knows {len(versions_by_sha)} of {len(all_shas)} jars by sha1")
json.dump(versions_by_sha, open(os.path.join(BASE, "modrinth_versions.json"), "w"), indent=1)

# ── Modrinth: project objects (env flags client_side/server_side) ───────────
project_ids = sorted({v["project_id"] for v in versions_by_sha.values()})
# also projects named in manifest sources but unknown by sha (repacked jars)
for meta in mods.values():
    if meta["source"]["type"] == "modrinth":
        project_ids.append(meta["source"]["project_id"])
# and every dependency target project
for v in versions_by_sha.values():
    for d in v.get("dependencies", []):
        if d.get("project_id"):
            project_ids.append(d["project_id"])
project_ids = sorted(set(project_ids))
projects = {}
for i in range(0, len(project_ids), 100):
    chunk = project_ids[i:i + 100]
    ids = json.dumps(chunk)
    url = "https://api.modrinth.com/v2/projects?ids=" + urllib.request.quote(ids)
    got = json.loads(get(url))
    for p in got:
        projects[p["id"]] = p
    time.sleep(0.5)
print(f"fetched {len(projects)} modrinth projects")
json.dump(projects, open(os.path.join(BASE, "modrinth_projects.json"), "w"), indent=1)

# ── download every jar ──────────────────────────────────────────────────────
def dl(sha, url):
    path = os.path.join(JARS, f"{sha}.jar")
    if os.path.exists(path):
        h = hashlib.sha1(open(path, "rb").read()).hexdigest()
        if h == sha:
            return "cached"
    data = get(url)
    h = hashlib.sha1(data).hexdigest()
    if h != sha:
        raise RuntimeError(f"sha mismatch for {url}: got {h} want {sha}")
    open(path, "wb").write(data)
    return f"{len(data)} B"


fails = []
for n, (sha, meta) in enumerate(sorted(mods.items()), 1):
    src = meta["source"]
    if src["type"] == "smrt_cache":
        url = src["url"]
    else:
        v = versions_by_sha.get(sha)
        url = None
        if v:
            for f in v["files"]:
                if f["hashes"]["sha1"] == sha:
                    url = f["url"]
                    break
        if url is None:
            # a Modrinth-declared source whose bytes Modrinth does not know by
            # sha (repacked upstream?) -- resolve through the declared version id
            pv = json.loads(get(
                f"https://api.modrinth.com/v2/project/{src['project_id']}/version/{src['version_id']}"))
            for f in pv["files"]:
                if f["hashes"]["sha1"] == sha:
                    url = f["url"]
                    break
        if url is None:
            fails.append((meta["filename"], sha, "no url"))
            continue
    try:
        r = dl(sha, url)
        print(f"[{n}/{len(mods)}] {meta['filename']}: {r}", flush=True)
    except Exception as e:  # noqa: BLE001
        fails.append((meta["filename"], sha, str(e)))

json.dump(mods, open(os.path.join(BASE, "mods_meta.json"), "w"), indent=1)
if fails:
    print("FAILURES:")
    for f in fails:
        print(" ", f)

# ── storage replica: prod-faithful (only smrt_cache jars in cache/) ─────────
for pid, m in manifests.items():
    pdir = os.path.join(REPLICA, "packs", pid)
    mdir = os.path.join(pdir, "manifests")
    os.makedirs(mdir, exist_ok=True)
    os.makedirs(os.path.join(pdir, "authoring"), exist_ok=True)
    ver = m["pack_version"]
    json.dump(m, open(os.path.join(mdir, f"{ver}.json"), "w"))
    latest = os.path.join(mdir, "latest")
    if os.path.lexists(latest):
        os.remove(latest)
    os.symlink(f"{ver}.json", latest)
    # summary.json from the live mirror
    open(os.path.join(pdir, "summary.json"), "wb").write(
        get(f"https://smrt.hivens.dev/v1/packs/{pid}"))

count = 0
for sha, meta in mods.items():
    if meta["source"]["type"] != "smrt_cache":
        continue
    src = os.path.join(JARS, f"{sha}.jar")
    if not os.path.exists(src):
        continue
    dst_dir = os.path.join(REPLICA, "cache", sha[:2])
    os.makedirs(dst_dir, exist_ok=True)
    dst = os.path.join(dst_dir, f"{sha}.jar")
    if not os.path.exists(dst):
        os.link(src, dst)
    count += 1
print(f"replica: {count} cache jars linked, {len(manifests)} packs")
print("done")
