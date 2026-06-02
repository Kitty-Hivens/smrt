// Compare a freshly-computed (dry-run) manifest against the currently-published
// one, by mod filename + sha1. Drives the "what would publishing change?" panel.
// Pure; the launcher uses byte-equal sha1 for the same change detection.

import type { ModEntry, PackManifest } from './types';

export interface ModChange {
  filename: string;
  prevSha1: string;
  nextSha1: string;
}

export interface ManifestDiff {
  added: ModEntry[];
  removed: ModEntry[];
  changed: ModChange[];
  unchanged: number;
  prevVersion: string;
  nextVersion: string;
}

export function diffManifests(prev: PackManifest, next: PackManifest): ManifestDiff {
  const prevByName = new Map(prev.mods.map((m) => [m.filename, m]));
  const nextNames = new Set(next.mods.map((m) => m.filename));

  const added: ModEntry[] = [];
  const changed: ModChange[] = [];
  let unchanged = 0;

  for (const m of next.mods) {
    const before = prevByName.get(m.filename);
    if (!before) added.push(m);
    else if (before.sha1 !== m.sha1)
      changed.push({ filename: m.filename, prevSha1: before.sha1, nextSha1: m.sha1 });
    else unchanged++;
  }

  const removed = prev.mods.filter((m) => !nextNames.has(m.filename));

  return {
    added,
    removed,
    changed,
    unchanged,
    prevVersion: prev.pack_version,
    nextVersion: next.pack_version,
  };
}

export function diffIsEmpty(d: ManifestDiff): boolean {
  return d.added.length === 0 && d.removed.length === 0 && d.changed.length === 0;
}
