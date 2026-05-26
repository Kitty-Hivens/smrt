# Industrial pack ingest

End-to-end authoring pipeline for SmartyCraft's `Industrial` pack on the smrt mirror. Worked example: every shape and constant in this directory drives a real `Industrial.json` build that ships to clients.

## Pipeline

All steps below are `smrt-pack` subcommands (Rust). Earlier revisions of this directory wrapped the same flow in Python helpers (`extract-mod-annotations.py`, `enrich-mods.py`, `build-pack-config.py`, `upload-static.py`); those got retired in favour of native subcommands so a new pack onboarding does not require a Python toolchain.

### One-time setup

* Drop the SC client install (`~/.local/share/nexira/clients/Industrial/` or wherever Nexira put it) so the mods/, config/, resourcepacks/, shaderpacks/ trees are readable.
* Build `open-smrt-network` locally (the wire-protocol-clean replacement for SC's proprietary Smarty coremod) and `sha1sum` the resulting jar. Paste the sha1 into `curator.toml`'s `[substitute."Smarty-1.12.2.jar"].source.sha1` field.
* Make sure `/tmp/smrt-token` has the admin token (chmod 600; never commit).

### Recurring per-build, one-liner

The whole chain wrapped as one script. Re-runnable, idempotent, picks up cleanly after partial failure:

```bash
bash examples/industrial/full-pipeline.sh ~/IndustrialSC.zip
```

Drives: bootstrap → upload-mods.sh → apply-curator (which now also generates `hidemymods-spoof.json` from each jar's `mcmod.info`) → upload-static → build → curl health probe. Reads `CURATOR_TOML`, `STORAGE`, `CLIENT_DIR`, `TOKEN_FILE` etc from env with sensible defaults — `--help`-like usage block at the top of the script lists the override knobs.

When the SC archive hasn't changed but the curator config has (tweaked role table, added a cozy mod), skip the long re-bootstrap:

```bash
SKIP_BOOTSTRAP=1 bash examples/industrial/full-pipeline.sh _
```

### Recurring per-build, step by step

1. **Bootstrap the starter config** -- extracts mods + extras from the SC archive, runs the Modrinth sha1 batch lookup to identify which mods can ride Modrinth versus which need to live in the smrt cache. Writes a starter `Industrial.bootstrap.json`.

   ```bash
   smrt-pack bootstrap \
       --sc-archive ~/IndustrialSC.zip \
       --out        /tmp/Industrial.bootstrap.json \
       --pack-id    Industrial \
       --display-name Industrial \
       --tagline    "SmartyCraft Industrial via Hivens Mirror" \
       --minecraft-version 1.12.2 \
       --loader-name forge \
       --loader-version 14.23.5.2922 \
       --java-major 8
   ```

2. **Upload mod jars to the cache** -- one-shot bash uploader for every jar that lives in `mods/` (and substitutes Smarty for OSN). Pre-existing script; retained because it doubles as the OSN-swap moment.

   ```bash
   bash examples/industrial/upload-mods.sh
   ```

3. **Upload static assets** -- Rust subcommand that walks a local client directory and PUTs every regular file into the mirror's per-pack static area. Reads the admin token from `/tmp/smrt-token`.

   ```bash
   smrt-pack upload-static \
       --pack-id Industrial \
       --dir     ~/.local/share/nexira/clients/Industrial
   ```

4. **Run the curator chain** -- one omnibus subcommand reads `curator.toml` and applies every per-pack mutation in canonical order: mcmod.info enrich, role table, category table, mark-optional, source substitution (Smarty -> OSN), requires inference, and finally the Modrinth-direct extras (cozy mods + RPs + shaders).

   ```bash
   smrt-pack apply-curator \
       --config  /tmp/Industrial.bootstrap.json \
       --curator examples/industrial/curator.toml \
       --out     /tmp/Industrial.curated.json
   ```

5. **Build the wire manifest** -- resolves every source against the cache or Modrinth, writes `<storage>/packs/Industrial/manifests/<date>.json`, atomically swaps the `latest` symlink, and emits `<storage>/packs/Industrial/summary.json` with the rich pack metadata merged from `curator.toml`'s `[pack_meta]` section.

   ```bash
   smrt-pack build \
       --config  /tmp/Industrial.curated.json \
       --curator examples/industrial/curator.toml
   ```

## Files in this directory

| File              | Purpose                                                                                                       |
| ----------------- | ------------------------------------------------------------------------------------------------------------- |
| `curator.toml`    | Omnibus per-pack curator decisions: pack metadata + mark-optional + substitute + role table + category table + extra mods + extra assets + `[generate]` hidemymods spoof toggle. Drives both `apply-curator` and `build --curator`. |
| `full-pipeline.sh`| One-shot orchestrator: bootstrap -> upload-mods -> apply-curator -> upload-static -> build -> verify. Set `SKIP_BOOTSTRAP=1` to refresh without re-extracting the SC archive. |
| `role-table.toml` | Standalone role-table example -- kept for reference. The `apply-role-table` subcommand still reads files in this shape if a curator prefers separate files; `curator.toml`'s `[role_table]` section subsumes the same data. |
| `pack-meta.toml`  | Standalone pack-meta example -- same relationship as role-table.toml. `curator.toml`'s `[pack_meta]` section subsumes it. |
| `upload-mods.sh`  | Bulk uploader for mod jars (bash). Keeps the OSN-substitute step inline; will get a `smrt-pack upload-cache` subcommand in a follow-up.                              |
| `README.md`       | This file.                                                                                                    |

## License notes that turned up while curating

A scan of the optional pool surfaced a mix of licenses on the assets we host -- worth recording because it shapes which entries the launcher should mark as redistributable in the future Display block:

- `MIT` / `LGPL` / `Apache` / `MPL` -- fully redistributable, no concern
- `CC-BY-NC-SA` -- non-commercial, fine for our non-paid mirror
- `CC-BY-NC-ND` -- no derivatives, hosting verbatim is fine, repackaging is not
- `LicenseRef-All-Rights-Reserved` -- formally no redistribution permission; civil-DMCA risk on hosting. SC has hosted these for years; we inherit the same posture
- Proprietary (OptiFine, Sildurs Vibrant Shaders) -- not on Modrinth, hosted via smrt_cache when SC includes them
