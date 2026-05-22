import sys, urllib.parse, urllib.request, urllib.error
from pathlib import Path

CLIENT = Path("/home/haru/.local/share/nexira/clients/Industrial")
TOKEN = Path("/tmp/smrt-token").read_text().strip()
BASE = "https://smrt.hivens.dev"
PACK_ID = "Industrial"

# Top-level paths to ship: configs, resourcepacks, shaderpacks, plus root client settings
CANDIDATES = []
# All files under config/
for f in (CLIENT / "config").rglob("*"):
    if f.is_file():
        CANDIDATES.append(f.relative_to(CLIENT))
# Resource packs and shader packs (whole dirs)
for sub in ["resourcepacks", "shaderpacks"]:
    d = CLIENT / sub
    if d.is_dir():
        for f in d.iterdir():
            if f.is_file():
                CANDIDATES.append(f.relative_to(CLIENT))
# Root client-settings files
for fn in ["options.txt", "optionsof.txt", "servers.dat"]:
    p = CLIENT / fn
    if p.exists():
        CANDIDATES.append(p.relative_to(CLIENT))

print(f"=== {len(CANDIDATES)} candidates ===", file=sys.stderr)

# Validate rel_path each candidate against smrt's path rules:
#   - no leading dot per segment
#   - segment chars must be alnum/-/_/. only
# Files violating get reported & skipped (manual fix needed).
def is_safe_segment(s):
    if not s or s.startswith("."):
        return False
    return all(c.isalnum() or c in "-_." for c in s.replace(" ", "_").replace("(", "_").replace(")", "_").replace("+", "_").replace(",", "_"))

def is_safe_rel(rel):
    parts = rel.split("/")
    return all(is_safe_segment(p) for p in parts)

ok, skipped, failed = 0, 0, 0
for rel in CANDIDATES:
    rel_str = str(rel).replace("\\", "/")
    # client-side skip removed; let server validate (it's authoritative).
    # URL encode path segments (handles spaces in shaderpack names)
    encoded = "/".join(urllib.parse.quote(p, safe="") for p in rel_str.split("/"))
    url = f"{BASE}/v1/admin/packs/{PACK_ID}/static/{encoded}"
    body = (CLIENT / rel).read_bytes()
    req = urllib.request.Request(
        url, data=body, method="PUT",
        headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/octet-stream"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            if r.status == 201:
                ok += 1
            else:
                print(f"  ? HTTP {r.status} for {rel_str}", file=sys.stderr)
                failed += 1
    except urllib.error.HTTPError as e:
        print(f"  ! HTTP {e.code} for {rel_str}: {e.read().decode('utf-8', 'ignore')[:80]}", file=sys.stderr)
        failed += 1

print(f"\nuploaded: {ok}, skipped: {skipped}, failed: {failed}", file=sys.stderr)
