#!/usr/bin/env python3
"""Top up the corpus with reference jars for the under-represented categories
(server-side mods are absent from the two published packs). Fetches each slug's
project env flags + latest version's primary jar, verifies sha1, stores the jar
under jars/ and folds the project/version objects into the corpus snapshots so
the classification runner identifies them the way prod would.

Usage: topup.py <slug> [<slug> ...]
"""
import hashlib
import json
import os
import sys
import time
import urllib.request

BASE = os.path.dirname(os.path.abspath(__file__))
UA = "smrt-corpus-audit/1.0 (dev tooling)"


def get(url, timeout=120):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read()


def load(name):
    p = os.path.join(BASE, name)
    return json.load(open(p)) if os.path.exists(p) else {}


def save(name, data):
    json.dump(data, open(os.path.join(BASE, name), "w"), indent=1)


projects = load("modrinth_projects.json")
versions = load("modrinth_versions.json")
os.makedirs(os.path.join(BASE, "jars"), exist_ok=True)

for slug in sys.argv[1:]:
    p = json.loads(get(f"https://api.modrinth.com/v2/project/{slug}"))
    projects[p["id"]] = p
    vs = json.loads(get(f"https://api.modrinth.com/v2/project/{slug}/version"))
    v = vs[0]
    f = next((f for f in v["files"] if f["primary"]), v["files"][0])
    sha = f["hashes"]["sha1"]
    path = os.path.join(BASE, "jars", f"{sha}.jar")
    if not os.path.exists(path):
        data = get(f["url"])
        assert hashlib.sha1(data).hexdigest() == sha, slug
        open(path, "wb").write(data)
    versions[sha] = v
    print(f"{slug}: env {p['client_side']}/{p['server_side']} sha1 {sha} "
          f"({f['filename']})")
    time.sleep(0.5)

save("modrinth_projects.json", projects)
save("modrinth_versions.json", versions)
