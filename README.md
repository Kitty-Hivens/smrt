# smrt

HTTP mirror backend for SmartyCraft Minecraft servers.

Sits between [Nexira](https://github.com/Kitty-Hivens/Nexira) and SmartyCraft: serves per-pack manifests, curated server metadata, and self-hosted mod jars for mods unreachable through Modrinth. SC's own infrastructure remains the source of truth for live game-server endpoints and authentication; this mirror does not proxy those.

## Status

Early skeleton. Implements `/v1/health` only. The full API surface and behavioral semantics are defined in the spec ahead of implementation.

## Specification

Wire format and endpoint contracts live in the Nexira docs site:
[smrt-api-spec.md](https://github.com/Kitty-Hivens/Nexira/blob/stable/docs/src/content/docs/dev/smrt-api-spec.md).

## Building

Requires Rust 1.85 or newer (edition 2024).

```
cargo build --release
```

Binary lands at `target/release/smrt` -- single static binary, ~10-15 MB.

## Running locally

Environment variables:

| Variable | Default | Purpose |
|---|---|---|
| `SMRT_BIND_ADDR` | `127.0.0.1:9000` | TCP bind address. |
| `SMRT_STORAGE_DIR` | `/var/lib/smrt` | Root of pack manifests, server metadata, mod cache. |
| `SMRT_ADMIN_TOKEN` | none | Bearer token required for `/v1/admin/...` endpoints. Read endpoints work without it. |
| `RUST_LOG` | `smrt=info,tower_http=info` | tracing-subscriber filter. |

```
SMRT_ADMIN_TOKEN=$(openssl rand -base64 32) cargo run
```

Then:

```
curl http://127.0.0.1:9000/v1/health
```

## Deployment

See [deploy/README.md](deploy/README.md) for the systemd + nginx + certbot walkthrough on a fresh VPS.

### Continuous deployment

`.github/workflows/ci.yml` builds + tests on every PR, and on push-to-`main` ships the release binary to the VPS via SSH and restarts the systemd unit. The workflow uses three repository secrets:

| Secret                       | What                                                                                   |
| ---------------------------- | -------------------------------------------------------------------------------------- |
| `SMRT_DEPLOY_HOST`           | `user@host[:port]` -- the SSH target. Typically `root@hivens.dev`.                     |
| `SMRT_DEPLOY_SSH_KEY`        | PEM-encoded private key (ed25519). Public part goes into the VPS's `authorized_keys`.  |
| `SMRT_DEPLOY_KNOWN_HOSTS`    | Output of `ssh-keyscan -H <host>` for the VPS. Pinning host identity prevents MITM mid-deploy. |

The push branch (`main`) is also gated through a GitHub Environment named `production`. Adding a required reviewer under Repo Settings -> Environments -> production locks every deploy behind a manual approval.

One-time setup, run locally on a trusted workstation (NOT inside CI logs):

```
# 1. Mint a dedicated CI-only deploy key (do NOT reuse a personal key).
ssh-keygen -t ed25519 -f ./smrt-ci-deploy -N '' -C 'smrt-ci-deploy@github'

# 2. Authorise the public key on the VPS (one prompt for the existing
#    root password / personal key auth).
ssh-copy-id -i ./smrt-ci-deploy.pub root@<host>

# 3. Capture the VPS host key.
ssh-keyscan -H <host> > ./smrt-ci-known-hosts

# 4. Push all three into the repo's Actions secrets via gh CLI.
gh secret set SMRT_DEPLOY_HOST       --body 'root@<host>'
gh secret set SMRT_DEPLOY_SSH_KEY    --body "$(cat ./smrt-ci-deploy)"
gh secret set SMRT_DEPLOY_KNOWN_HOSTS --body "$(cat ./smrt-ci-known-hosts)"

# 5. Verify, then shred the local copies (the key now lives only in
#    GitHub secrets + VPS authorized_keys).
shred -u ./smrt-ci-deploy ./smrt-ci-deploy.pub ./smrt-ci-known-hosts
```

The legacy `deploy/deploy.sh` stays as the emergency-local-deploy fallback: it scp+ssh's from the operator's workstation, useful when GitHub Actions is down or the operator wants to ship a one-off build without going through main.

## License

Apache License 2.0. See [LICENSE](LICENSE).
