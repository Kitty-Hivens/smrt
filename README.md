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

## License

Apache License 2.0. See [LICENSE](LICENSE).
