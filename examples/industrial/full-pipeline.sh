#!/usr/bin/env bash
# Full Industrial release pipeline.
#
# Takes the SC archive zip as $1 and walks every step needed to turn
# it into a live mirror manifest:
#
#   1. smrt-pack bootstrap        (SC zip -> starter PackConfig + jars
#                                  into storage/cache/)
#   2. examples/industrial/
#        upload-mods.sh           (push cache jars to admin /v1/admin/cache,
#                                  substitutes Smarty -> open-smrt-network)
#   3. smrt-pack upload-static    (push <clientDir>/{config,resourcepacks,
#                                  shaderpacks,options.txt,...} to
#                                  admin /v1/admin/packs/.../static)
#   4. smrt-pack build            (run enrichment passes -- mcmod.info
#                                  display + requires -- resolve every
#                                  source, write wire manifest + summary,
#                                  atomically swap the `latest` symlink)
#   5. curl health probe          (verify the new pack_version actually
#                                  serves)
#
# Pack-card metadata (icon / banner / gallery / description) and per-mod
# settings (optional / default-off, category, role, incompatibilities) live
# in the pack config, edited in the panel's Config tab.
#
# Run from anywhere; the script is self-locating. Each step is
# idempotent so re-running after a partial failure picks up cleanly.

set -euo pipefail

# ── inputs ────────────────────────────────────────────────────────────────

if [[ $# -lt 1 ]]; then
    cat >&2 <<'USAGE'
usage: full-pipeline.sh <sc-archive.zip>

env overrides:
  SMRT_PACK_BIN     path to smrt-pack binary           (default: ./target/release/smrt-pack)
  STORAGE           mirror storage root                 (default: /var/lib/smrt)
  PACK_ID           pack identifier                     (default: Industrial)
  DISPLAY_NAME      human pack name                     (default: Industrial)
  TAGLINE           short pack tagline                  (default: "SC Industrial via Hivens Mirror")
  MC_VERSION        Minecraft version                   (default: 1.12.2)
  LOADER_VERSION    Forge version                       (default: 14.23.5.2922)
  JAVA_MAJOR        Java major version                  (default: 8)
  CLIENT_DIR        SC client install for upload-static (default: ~/.local/share/nexira/clients/Industrial)
  MIRROR_BASE       mirror URL base                     (default: https://smrt.hivens.dev)
  TOKEN_FILE        admin token path                    (default: /tmp/smrt-token)
  WORK_DIR          per-run staging dir                 (default: /tmp/smrt-pipeline)
  SKIP_BOOTSTRAP    set to non-empty to reuse the prior PackConfig
                    (useful when the SC archive hasn't changed but
                    you only re-uploaded mods or static assets)
USAGE
    exit 64
fi

SC_ARCHIVE="$1"

SMRT_PACK_BIN="${SMRT_PACK_BIN:-./target/release/smrt-pack}"
STORAGE="${STORAGE:-/var/lib/smrt}"
PACK_ID="${PACK_ID:-Industrial}"
DISPLAY_NAME="${DISPLAY_NAME:-Industrial}"
TAGLINE="${TAGLINE:-SmartyCraft Industrial via Hivens Mirror}"
MC_VERSION="${MC_VERSION:-1.12.2}"
LOADER_VERSION="${LOADER_VERSION:-14.23.5.2922}"
JAVA_MAJOR="${JAVA_MAJOR:-8}"
CLIENT_DIR="${CLIENT_DIR:-$HOME/.local/share/nexira/clients/$PACK_ID}"
MIRROR_BASE="${MIRROR_BASE:-https://smrt.hivens.dev}"
TOKEN_FILE="${TOKEN_FILE:-/tmp/smrt-token}"
WORK_DIR="${WORK_DIR:-/tmp/smrt-pipeline}"

HERE="$(cd "$(dirname "$0")" && pwd)"
UPLOAD_MODS_SH="$HERE/upload-mods.sh"

BOOTSTRAP_JSON="$WORK_DIR/$PACK_ID.bootstrap.json"

# ── preflight ─────────────────────────────────────────────────────────────

mkdir -p "$WORK_DIR"

if [[ ! -x "$SMRT_PACK_BIN" ]]; then
    echo "smrt-pack binary not found at $SMRT_PACK_BIN" >&2
    echo "build it first:  cargo build --release --bin smrt-pack" >&2
    exit 66
fi
if [[ ! -f "$TOKEN_FILE" ]]; then
    echo "admin token not found at $TOKEN_FILE" >&2
    echo "place the token there (chmod 600) before running" >&2
    exit 66
fi

# ── 1. bootstrap ──────────────────────────────────────────────────────────

if [[ -z "${SKIP_BOOTSTRAP:-}" ]]; then
    if [[ ! -f "$SC_ARCHIVE" ]]; then
        echo "SC archive not found at $SC_ARCHIVE" >&2
        exit 66
    fi
    echo "==> [1/5] bootstrap"
    "$SMRT_PACK_BIN" bootstrap \
        --sc-archive       "$SC_ARCHIVE" \
        --out              "$BOOTSTRAP_JSON" \
        --pack-id          "$PACK_ID" \
        --display-name     "$DISPLAY_NAME" \
        --tagline          "$TAGLINE" \
        --minecraft-version "$MC_VERSION" \
        --loader-name      forge \
        --loader-version   "$LOADER_VERSION" \
        --java-major       "$JAVA_MAJOR" \
        --storage          "$STORAGE"
else
    echo "==> [1/5] bootstrap SKIPPED (SKIP_BOOTSTRAP is set)"
    [[ -f "$BOOTSTRAP_JSON" ]] || {
        echo "  but no prior $BOOTSTRAP_JSON to reuse" >&2
        exit 66
    }
fi

# ── 2. upload mod jars to mirror cache ────────────────────────────────────

if [[ -x "$UPLOAD_MODS_SH" ]]; then
    echo "==> [2/5] upload mod jars (legacy bash uploader)"
    bash "$UPLOAD_MODS_SH"
else
    echo "==> [2/5] upload-mods.sh missing or not executable -- skipping" >&2
fi

# ── 3. upload-static ──────────────────────────────────────────────────────

if [[ -d "$CLIENT_DIR" ]]; then
    echo "==> [3/5] upload-static from $CLIENT_DIR"
    "$SMRT_PACK_BIN" upload-static \
        --pack-id   "$PACK_ID" \
        --dir       "$CLIENT_DIR" \
        --mirror-base "$MIRROR_BASE" \
        --token-file "$TOKEN_FILE"
else
    echo "==> [3/5] upload-static SKIPPED -- $CLIENT_DIR does not exist"
fi

# ── 4. build ──────────────────────────────────────────────────────────────

echo "==> [4/5] build wire manifest + summary"
"$SMRT_PACK_BIN" build \
    --config   "$BOOTSTRAP_JSON" \
    --storage  "$STORAGE" \
    --mirror-base "$MIRROR_BASE"

# ── 5. verify ─────────────────────────────────────────────────────────────

echo "==> [5/5] verify live manifest"
PACK_URL="$MIRROR_BASE/v1/packs/$PACK_ID"
echo "  GET $PACK_URL"
curl -fsS "$PACK_URL" | tee "$WORK_DIR/$PACK_ID.summary.json"
echo
echo "  GET $PACK_URL/manifest (first 3 mods)"
# jq is nice-to-have but not required; fall back to head if absent.
if command -v jq >/dev/null; then
    curl -fsS "$PACK_URL/manifest" | jq '.mods[0:3]'
else
    curl -fsS "$PACK_URL/manifest" | head -c 1200
    echo
fi

echo
echo "==> done -- $PACK_ID rebuild complete"
