"""Industrial v8: cozy additions (mods + RPs from Modrinth) + curated options.txt"""
import hashlib, json, urllib.parse, urllib.request
from pathlib import Path

UA = "Kitty-Hivens/smrt-build"
CLIENT = Path("/home/haru/.local/share/nexira/clients/Industrial")
MODS = CLIENT / "mods"
MANIFEST = Path("/home/haru/.local/share/nexira/manifest-cache/Industrial.json")
ENRICHED = Path("/tmp/industrial-mods-enriched.json")
OSN = Path("/home/haru/open-smrt-network/forge-1.12.2/build/libs/open-smrt-network-forge-1.12.2-0.1.0.jar")

def sha1_of(p):
    h = hashlib.sha1(); h.update(p.read_bytes()); return h.hexdigest()

def find_jar(fn, *dirs):
    for d in dirs:
        p = d / fn
        if p.exists():
            return p
    return None

def modrinth_lookup(slug):
    """Get latest 1.12.2 version, return {project_id, version_id, filename}."""
    url = f"https://api.modrinth.com/v2/project/{slug}/version?game_versions=%5B%221.12.2%22%5D"
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=20) as r:
        vs = json.loads(r.read())
    v = vs[0]
    primary = next((f for f in v["files"] if f.get("primary")), v["files"][0])
    return {
        "project_id": v["project_id"],
        "version_id": v["id"],
        "version_number": v["version_number"],
        "filename": primary["filename"],
        "loaders": v.get("loaders", []),
    }

# Cozy additions: mods (forced + optional) and RPs (forced + optional)
COZY_MODS_FORCED = [  # required: true, cozy default experience
    ("appleskin", "performance", "Hunger and saturation visualization."),
    ("ambientsounds", "audio", "Biome-contextual ambient sound (wind, water, cave echo)."),
    ("clumps", "performance", "XP orbs merge to reduce TPS lag from large drops."),
    ("entityculling", "performance", "Cull off-screen entities for FPS gains."),
]
COZY_MODS_OPTIONAL = [
    ("armorhud", "info-overlay", "Armor durability HUD."),
    ("carry-on", "content", "Pick up blocks intact."),
    ("controlling", "misc", "Keybinds search UI."),
    ("crafting-tweaks", "inventory", "Extra crafting-table buttons (clear, rotate, balance)."),
    ("tips", "misc", "Loading-screen tip messages."),
    ("ksyxis", "performance", "World load speedup by skipping unneeded chunks."),
    ("born-in-a-barn", "performance", "Fixes village mob-spawn lag."),
    ("music-triggers", "audio", "Context-aware music switching by biome / dimension / state."),
]
COZY_RPS_FORCED = [  # downloaded AND in options.txt
    ("better-farm-animals", "resource-pack", "Stardew-style 3D farm critters (cows, pigs, chickens)."),
    ("farm-3d", "resource-pack", "3D farm tools and crops."),
    ("mellowed", "resource-pack", "Soft palette overlay."),
]
COZY_RPS_OPTIONAL = [
    ("comforts-modernized", "resource-pack", "Sleeping bag / hammock textures (mod-specific)."),
    ("pixel-perfection-fidelity", "resource-pack", "Vanilla++ pixel-faithful textures."),
    ("new-default-plus", "resource-pack", "Vanilla++ default."),
    ("soft-bits", "resource-pack", "Soft palette pack."),
    ("bare-bones", "resource-pack", "Minimalist simplified textures."),
    ("lively-by-alexio", "resource-pack", "Warm-tone vanilla overlay."),
]
COZY_SHADERS_OPTIONAL = [
    ("mellow", "shader", "Soft palette shader (MIT)."),
    ("pastel-shaders", "shader", "Literally pastel; Stardew-adjacent mood."),
]

print("=== fetching Modrinth metadata for cozy additions ===")
new_mods = []
for slug, cat, desc in COZY_MODS_FORCED + COZY_MODS_OPTIONAL:
    m = modrinth_lookup(slug)
    required = (slug, cat, desc) in COZY_MODS_FORCED
    new_mods.append({
        "filename": m["filename"],
        "required": required,
        "source": {"type": "modrinth", "project_id": m["project_id"], "version_id": m["version_id"]},
        "display": {
            "name": slug.replace("-", " ").title(),
            "description": desc,
            "category": cat,
            "url": f"https://modrinth.com/mod/{slug}",
        },
    })
    print(f"  + mod {slug:25s} -> {m['filename']:40s} {'[FORCED]' if required else '[opt]'}")

new_rps = []
for slug, cat, desc in COZY_RPS_FORCED + COZY_RPS_OPTIONAL:
    m = modrinth_lookup(slug)
    required = (slug, cat, desc) in COZY_RPS_FORCED
    new_rps.append({
        "slug": slug,
        "dest": f"resourcepacks/{m['filename']}",
        "modrinth_filename": m["filename"],
        "required": required,
        "source": {"type": "modrinth", "project_id": m["project_id"], "version_id": m["version_id"]},
        "display": {
            "name": slug.replace("-", " ").title(),
            "description": desc,
            "category": cat,
            "url": f"https://modrinth.com/resourcepack/{slug}",
        },
    })
    print(f"  + rp  {slug:25s} -> {m['filename']:40s} {'[FORCED]' if required else '[opt]'}")

new_shaders = []
for slug, cat, desc in COZY_SHADERS_OPTIONAL:
    m = modrinth_lookup(slug)
    new_shaders.append({
        "slug": slug,
        "dest": f"shaderpacks/{m['filename']}",
        "source": {"type": "modrinth", "project_id": m["project_id"], "version_id": m["version_id"]},
        "display": {
            "name": slug.replace("-", " ").title(),
            "description": desc,
            "category": cat,
            "url": f"https://modrinth.com/shader/{slug}",
        },
    })
    print(f"  + shader {slug:25s} -> {m['filename']}")

# === Now generate curated options.txt: add forced RPs to resourcePacks list ===
print("\n=== curating options.txt ===")
forced_rp_filenames = [rp["modrinth_filename"] for rp in new_rps if rp["required"]]
new_resourcepacks_line = json.dumps(["Faithful.zip"] + [f"file/{n}" for n in forced_rp_filenames])
print(f"  new resourcePacks line: resourcePacks:{new_resourcepacks_line}")

options_in = (CLIENT / "options.txt").read_text()
options_out = []
for line in options_in.split("\n"):
    if line.startswith("resourcePacks:"):
        options_out.append(f"resourcePacks:{new_resourcepacks_line}")
    else:
        options_out.append(line)
options_curated = "\n".join(options_out)
Path("/tmp/industrial-options-curated.txt").write_text(options_curated)
print(f"  wrote /tmp/industrial-options-curated.txt")

# === Build full pack-config v8 ===
# Bring in everything from v7 + new adds.
print("\n=== generating pack-config v8 ===")

# Load enriched for existing mods/assets
enriched_list = json.loads(ENRICHED.read_text())
ENRICHED_BY_FN = {r["filename"]: r for r in enriched_list}

# Reuse logic from v3 generator (mostly)
TOGGLEABLE = {x.lower() for x in {
    "BetterChat", "ServerTabInfo", "XaerosMinimap", "MoreOverlays", "FoamFix",
    "VoxelMap", "ToroHealth", "XaerosWorldMap", "ReplayMod", "Phosphor",
    "WailaHarvestability", "TexFix", "MouseTweaks", "SoundFilters",
    "DamageIndicators", "Schematica", "NoRecipeBook", "DiscordRP", "InventoryTweaks",
}}

m = json.loads(MANIFEST.read_text())
ind = m["manifest"]["directories"]["Industrial"]
top_files = sorted(ind["directories"]["mods"]["files"].keys())
sub_files = sorted(ind["directories"]["mods"]["directories"]["1.12.2"]["files"].keys())

mods = []
# Existing core mods (server-required) -- minimal display
for fn in top_files:
    jar = find_jar(fn, MODS)
    if not jar: continue
    e = ENRICHED_BY_FN.get(fn, {}) or {}
    display = {"name": e.get("name") or fn.removesuffix(".jar"), "category": "core"}
    if e.get("description"):
        display["description"] = e["description"].strip()
    mods.append({
        "filename": fn, "required": True,
        "source": {"type": "smrt_cache", "sha1": sha1_of(jar)},
        "display": display,
    })

# Existing optional pool (toggleable + mandatory-extras)
for fn in sub_files:
    if fn == "Smarty-1.12.2.jar": continue
    jar = find_jar(fn, MODS / "1.12.2", MODS)
    if not jar: continue
    e = ENRICHED_BY_FN.get(fn, {}) or {}
    is_toggleable = fn.removesuffix(".jar").lower() in TOGGLEABLE
    display = {"name": e.get("name") or fn.removesuffix(".jar")}
    if e.get("description"): display["description"] = e["description"].strip()
    # Categories per earlier definitions
    CAT = {"OptiFine.jar":"render","Phosphor.jar":"render","XaerosMinimap.jar":"minimap",
           "VoxelMap.jar":"minimap","XaerosWorldMap.jar":"world-map","HWYLA.jar":"tooltip",
           "WailaHarvestability.jar":"tooltip","JEI.jar":"recipe-viewer","JER.jar":"recipe-viewer",
           "JEIBees.jar":"recipe-viewer","InventoryTweaks.jar":"inventory","MouseTweaks.jar":"inventory",
           "DamageIndicators.jar":"info-overlay","ToroHealth.jar":"info-overlay",
           "MoreOverlays.jar":"info-overlay","ServerTabInfo.jar":"info-overlay",
           "SoundFilters.jar":"audio","WorldEditCUI.jar":"world-tools","Schematica.jar":"world-tools",
           "FoamFix.jar":"performance","TexFix.jar":"performance","BetterChat.jar":"chat",
           "NoRecipeBook.jar":"misc","DiscordRP.jar":"misc","ReplayMod.jar":"misc",
           "Quark.jar":"content","CustomNPCs.jar":"content","VariedCommodities.jar":"content",
           "TreeCapitator.jar":"content","PotionCore.jar":"content",
           "Hats.jar":"cosmetic","HatStand.jar":"cosmetic","NBTEdit.jar":"admin-tool",
           "AutoRegLib.jar":"lib","bspkrsCore.jar":"lib","ChickenASM-1.12-1.0.2.7.jar":"lib",
           "iChunUtil.jar":"lib","LunatriusCore.jar":"lib"}
    if fn in CAT: display["category"] = CAT[fn]
    mods.append({
        "filename": fn, "required": not is_toggleable,
        "source": {"type": "smrt_cache", "sha1": sha1_of(jar)},
        "display": display,
    })

# OptiFine override + Smarty substitute
for e in mods:
    if e["filename"] == "OptiFine.jar":
        e["required"] = False
mods.append({
    "filename": "Smarty-1.12.2.jar", "required": True,
    "source": {"type": "smrt_cache", "sha1": sha1_of(OSN)},
    "display": {"name": "Open Smarty Network", "category": "lib",
                "description": "Drop-in replacement for SC's proprietary 'Smarty' auth mod. Apache-2.0."},
})

# Add cozy mods (Modrinth direct)
mods.extend(new_mods)

# Existing assets (configs + RPs + shaderpacks + root settings)
assets = []
for f in sorted((CLIENT / "config").rglob("*")):
    if not f.is_file(): continue
    rel = str(f.relative_to(CLIENT)).replace("\\", "/")
    assets.append({
        "dest": rel, "required": True,
        "source": {"type": "smrt_static", "rel_path": rel},
        "display": {"category": "config"},
    })
for f in sorted((CLIENT / "resourcepacks").glob("*")):
    if not f.is_file(): continue
    rel = str(f.relative_to(CLIENT)).replace("\\", "/")
    assets.append({
        "dest": rel, "required": False,
        "source": {"type": "smrt_static", "rel_path": rel},
        "display": {"name": f.stem, "category": "resource-pack"},
    })
for f in sorted((CLIENT / "shaderpacks").glob("*")):
    if not f.is_file(): continue
    rel = str(f.relative_to(CLIENT)).replace("\\", "/")
    assets.append({
        "dest": rel, "required": False,
        "source": {"type": "smrt_static", "rel_path": rel},
        "display": {"name": f.stem, "category": "shader-pack",
                    "description": "Requires OptiFine to render."},
    })

# Curated options.txt (will upload to smrt_static separately)
ROOT_DISPLAY = {
    "options.txt": {"name": "Default options", "category": "client-defaults",
                    "description": "Pack-shipped options with cozy RPs enabled by default."},
    "optionsof.txt": {"name": "OptiFine options", "category": "client-defaults"},
    "servers.dat": {"name": "Curated server list", "category": "client-defaults"},
}
for fn, disp in ROOT_DISPLAY.items():
    p = CLIENT / fn
    if not p.exists(): continue
    assets.append({
        "dest": fn, "required": True,
        "source": {"type": "smrt_static", "rel_path": fn},
        "display": disp,
    })

# Add cozy RPs (Modrinth direct, downloaded to resourcepacks/)
for rp in new_rps:
    entry = {
        "dest": rp["dest"], "required": rp["required"],
        "source": rp["source"], "display": rp["display"],
    }
    assets.append(entry)

# Add cozy shaders (optional)
for sh in new_shaders:
    assets.append({
        "dest": sh["dest"], "required": False,
        "source": sh["source"], "display": sh["display"],
    })

config = {
    "pack_id": "Industrial",
    "display_name": "Industrial",
    "tagline": "SmartyCraft Industrial via Hivens Mirror (v8: cozy additions)",
    "minecraft_version": "1.12.2",
    "loader": {"name": "forge", "version": "14.23.5.2922"},
    "java_major": 8,
    "tags": ["tech", "industrial", "1.12.2", "sc-compat", "cozy"],
    "featured": False,
    "mods": mods, "assets": assets,
}
OUT = Path("/tmp/industrial-pack-config.json")
OUT.write_text(json.dumps(config, indent=2, ensure_ascii=False))
print(f"\nwrote {OUT}")
print(f"  mods: {len(mods)} (req={sum(1 for m in mods if m['required'])}, opt={sum(1 for m in mods if not m['required'])})")
print(f"  assets: {len(assets)} (req={sum(1 for a in assets if a['required'])}, opt={sum(1 for a in assets if not a['required'])})")
