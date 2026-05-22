import hashlib, json, sys, time, urllib.parse, urllib.request, zipfile
from pathlib import Path

CLIENT = Path("/home/haru/.local/share/nexira/clients/Industrial")
MODS = CLIENT / "mods"
UA = "Kitty-Hivens/smrt-enrich"

def http_get_json(url):
    try:
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.loads(r.read())
    except Exception:
        return None

def http_post_json(url, body):
    try:
        req = urllib.request.Request(
            url, data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json", "User-Agent": UA},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read())
    except Exception as e:
        print(f"  http_post err: {e}", file=sys.stderr)
        return None

def sha1_of(p):
    h = hashlib.sha1(); h.update(p.read_bytes()); return h.hexdigest()

def extract_mcmod(jar_path):
    """Return (modid, name, version, description, authors, url) or None."""
    try:
        with zipfile.ZipFile(jar_path) as z:
            if "mcmod.info" not in z.namelist():
                return None
            blob = z.read("mcmod.info").decode("utf-8", "ignore").lstrip("﻿").strip()
            if not blob.startswith(("[", "{")):
                return None
            d = json.loads(blob)
            if isinstance(d, dict) and "modList" in d:
                d = d["modList"]
            if isinstance(d, list) and d:
                e = d[0]
                authors = e.get("authorList") or e.get("authors") or []
                if isinstance(authors, str):
                    authors = [authors]
                return {
                    "modid": e.get("modid", ""),
                    "name": e.get("name", ""),
                    "version": e.get("version", ""),
                    "description": (e.get("description", "") or "").strip(),
                    "authors": authors,
                    "url": e.get("url", "") or "",
                    "credits": (e.get("credits", "") or "").strip(),
                }
    except Exception:
        pass
    return None

# Phase 1: enumerate every jar (top + 1.12.2)
jars = sorted(list(MODS.glob("*.jar")) + list((MODS / "1.12.2").glob("*.jar")))
# Dedup by sha1: top-level + 1.12.2/ may have same content
seen = {}
for j in jars:
    sha = sha1_of(j)
    if sha not in seen:
        seen[sha] = j
unique = list(seen.values())
print(f"=== {len(jars)} jar paths, {len(unique)} unique by sha1 ===", file=sys.stderr)

# Phase 2: pull mcmod.info per unique jar
records = []
for jar in unique:
    sha = sha1_of(jar)
    info = extract_mcmod(jar) or {}
    records.append({
        "filename": jar.name,
        "rel_path": str(jar.relative_to(MODS)),
        "sha1": sha,
        "size": jar.stat().st_size,
        **info,
    })

# Phase 3: Modrinth sha1 batch lookup
print(f"=== Modrinth sha1 batch ({len(records)}) ===", file=sys.stderr)
sha_hits = http_post_json(
    "https://api.modrinth.com/v2/version_files",
    {"hashes": [r["sha1"] for r in records], "algorithm": "sha1"},
) or {}
print(f"  matched: {len(sha_hits)}", file=sys.stderr)

# Phase 4: for unmatched with modid, try Modrinth project by slug
print(f"=== Modrinth slug lookups (unmatched) ===", file=sys.stderr)
for r in records:
    if r["sha1"] in sha_hits:
        v = sha_hits[r["sha1"]]
        r["modrinth"] = {
            "tier": "sha1_direct",
            "project_id": v.get("project_id"),
            "version_id": v.get("id"),
            "version_number": v.get("version_number"),
        }
        continue
    modid = r.get("modid", "")
    if not modid:
        r["modrinth"] = None
        continue
    # Try slug variants
    variants = [modid.lower().replace("_", "-"), modid.lower(), modid.lower().replace("_", "")]
    found = None
    for slug in variants:
        proj = http_get_json(f"https://api.modrinth.com/v2/project/{slug}")
        if proj and proj.get("project_type") == "mod":
            found = {"tier": "slug_match", "slug": slug, "project_id": proj["id"],
                     "title": proj.get("title"), "license": (proj.get("license") or {}).get("id")}
            break
    r["modrinth"] = found
    time.sleep(0.1)

# Phase 5: for matched (sha1 + slug), fetch project metadata for license + description
print(f"=== fetching project metadata for matched ===", file=sys.stderr)
for r in records:
    if not r.get("modrinth"):
        continue
    pid = r["modrinth"].get("project_id") or r["modrinth"].get("slug")
    proj = http_get_json(f"https://api.modrinth.com/v2/project/{pid}")
    if proj:
        r["modrinth"].update({
            "title": proj.get("title"),
            "summary": proj.get("description", ""),
            "license": (proj.get("license") or {}).get("id"),
            "source_url": proj.get("source_url"),
            "issues_url": proj.get("issues_url"),
            "wiki_url": proj.get("wiki_url"),
        })
    time.sleep(0.1)

# Output
out = Path("/tmp/industrial-mods-enriched.json")
out.write_text(json.dumps(records, indent=2, ensure_ascii=False))
print(f"\nwrote {out}: {len(records)} records", file=sys.stderr)

mr_count = sum(1 for r in records if r.get("modrinth"))
mr_sha = sum(1 for r in records if r.get("modrinth", {}).get("tier") == "sha1_direct")
mr_slug = sum(1 for r in records if r.get("modrinth", {}).get("tier") == "slug_match")
print(f"  Modrinth: {mr_count} total ({mr_sha} sha1-direct, {mr_slug} slug-match, {len(records)-mr_count} no match)", file=sys.stderr)
