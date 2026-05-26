#!/usr/bin/env bash
set -euo pipefail
TOKEN=$(cat /tmp/smrt-token)
BASE=https://smrt.hivens.dev
MODS=/home/haru/.local/share/nexira/clients/Industrial/mods
OSN=/home/haru/open-smrt-network/forge-1.12.2/build/libs/open-smrt-network-forge-1.12.2-0.1.0.jar

upload() {
    local jar="$1"
    local sha prefix code
    sha=$(sha1sum "$jar" | cut -d' ' -f1)
    prefix=${sha:0:2}
    code=$(curl -s -o /dev/null -w "%{http_code}" -X PUT \
        -H "Authorization: Bearer $TOKEN" \
        --data-binary @"$jar" \
        "$BASE/v1/admin/cache/$prefix/$sha.jar")
    case "$code" in
        201) printf "  + %-46s %s\n" "$(basename "$jar")" "$sha" ;;
        400) printf "  ! %-46s HTTP 400 (already removed or sha mismatch)\n" "$(basename "$jar")" ;;
        *)   printf "  ? %-46s HTTP %s\n" "$(basename "$jar")" "$code" ;;
    esac
}

echo "=== top-level mods/ (38, server-required core) ==="
for jar in "$MODS"/*.jar; do
    [ "$(basename "$jar")" = "Smarty-1.12.2.jar" ] && continue
    upload "$jar"
done

echo
echo "=== mods/1.12.2/ (17, optional pool -- minus Smarty) ==="
for jar in "$MODS"/1.12.2/*.jar; do
    [ "$(basename "$jar")" = "Smarty-1.12.2.jar" ] && continue
    upload "$jar"
done

echo
echo "=== open-smrt-network (Smarty replacement) ==="
upload "$OSN"
