// What has changed since the last checkpoint, as rows a person can read.
//
// The commit box used to show a number and nothing else, so the message beside
// it was written from memory -- and the number is a count of changed JSON paths,
// which does not translate into things a curator did: a save that only let the
// dependency fill write two `display.requires` lists reported 22.
//
// This turns the same two configs into rows: which mod arrived, left, or moved,
// which asset, and which of the pack's own fields. Version ids are useless on
// screen (`P4yXqsnw -> bqMxf6Ua`), so a row carries the raw pin and the view
// fills in the labels when Modrinth can be asked -- the row is readable either
// way, and never waits on the network to appear.

import type { DeclaredAsset, DeclaredMod, Display, PackConfig, SourceDecl } from './types';

export type ChangeGroup = 'mods' | 'assets' | 'pack';
export type ChangeOp = 'add' | 'remove' | 'change';

export interface ChangeRow {
  group: ChangeGroup;
  op: ChangeOp;
  /** Filename, asset dest, or the pack field's own name. */
  label: string;
  /** What it was and what it became, for a change. */
  from?: string;
  to?: string;
  /**
   * The Modrinth project both sides pin, when the row is a version move within
   * one project -- the only case where `from`/`to` are ids a label can replace.
   */
  project?: string;
}

/** The pack's own scalar fields, in the order they are worth reading. */
const FIELDS: [keyof PackConfig, string][] = [
  ['minecraft_version', 'minecraft'],
  ['java_major', 'java'],
  ['version', 'version'],
  ['display_name', 'display_name'],
  ['tagline', 'tagline'],
  ['visibility', 'visibility'],
  ['featured', 'featured'],
];

/**
 * The parts of a display block a person writes. The rest of it -- the requires
 * graph, the presence class -- is the dependency fill's and the classifier's
 * own bookkeeping, written server-side on every save: counting those as changes
 * is what made the number beside this list mean nothing.
 */
const AUTHORED_DISPLAY = [
  'name',
  'description',
  'category',
  'icon_url',
  'license',
  'homepage',
  'incompatible_with',
] as const;

/** Every difference between a committed config and the live one. */
export function diffConfigs(head: PackConfig, live: PackConfig): ChangeRow[] {
  return [...fieldRows(head, live), ...modRows(head, live), ...assetRows(head, live)];
}

function fieldRows(head: PackConfig, live: PackConfig): ChangeRow[] {
  const rows: ChangeRow[] = [];
  const loader = (c: PackConfig) => `${c.loader.name} ${c.loader.version}`;
  if (loader(head) !== loader(live)) {
    rows.push({ group: 'pack', op: 'change', label: 'loader', from: loader(head), to: loader(live) });
  }
  for (const [key, label] of FIELDS) {
    const from = scalar(head[key]);
    const to = scalar(live[key]);
    if (from !== to) rows.push({ group: 'pack', op: 'change', label, from, to });
  }
  if (JSON.stringify(head.tags ?? []) !== JSON.stringify(live.tags ?? [])) {
    rows.push({
      group: 'pack',
      op: 'change',
      label: 'tags',
      from: (head.tags ?? []).join(', '),
      to: (live.tags ?? []).join(', '),
    });
  }
  // The pack card (icon, banner, gallery, description) is a block of prose and
  // urls; which of them moved is not worth a row apiece.
  if (JSON.stringify(head.pack_meta ?? {}) !== JSON.stringify(live.pack_meta ?? {})) {
    rows.push({ group: 'pack', op: 'change', label: 'pack_meta' });
  }
  if (JSON.stringify(head.auth ?? null) !== JSON.stringify(live.auth ?? null)) {
    rows.push({ group: 'pack', op: 'change', label: 'auth' });
  }
  return rows;
}

function modRows(head: PackConfig, live: PackConfig): ChangeRow[] {
  const before = new Map((head.mods ?? []).map((m) => [m.filename, m]));
  const after = new Map((live.mods ?? []).map((m) => [m.filename, m]));
  const rows: ChangeRow[] = [];
  for (const [filename, m] of after) {
    const was = before.get(filename);
    if (!was) {
      rows.push({ group: 'mods', op: 'add', label: filename, to: pin(m.source), project: projectOf(m.source) });
      continue;
    }
    const row = modChange(filename, was, m);
    if (row) rows.push(row);
  }
  for (const [filename, m] of before) {
    if (!after.has(filename)) {
      rows.push({ group: 'mods', op: 'remove', label: filename, from: pin(m.source), project: projectOf(m.source) });
    }
  }
  return sorted(rows);
}

/**
 * What moved on a mod that is in both configs. A pin change is the row worth
 * having; the install default is worth one too, since it decides what a player
 * gets. `pulled` and the display block are the fill's own bookkeeping and are
 * deliberately not rows -- they are the noise that made the count meaningless.
 */
function modChange(filename: string, was: DeclaredMod, now: DeclaredMod): ChangeRow | null {
  const from = pin(was.source);
  const to = pin(now.source);
  if (from !== to) {
    const project = projectOf(was.source);
    return {
      group: 'mods',
      op: 'change',
      label: filename,
      from,
      to,
      project: project && project === projectOf(now.source) ? project : undefined,
    };
  }
  if (was.default_enabled !== now.default_enabled) {
    return {
      group: 'mods',
      op: 'change',
      label: filename,
      from: enabled(was.default_enabled),
      to: enabled(now.default_enabled),
    };
  }
  if (authored(was.display) !== authored(now.display)) {
    return { group: 'mods', op: 'change', label: filename };
  }
  return null;
}

function assetRows(head: PackConfig, live: PackConfig): ChangeRow[] {
  const before = new Map((head.assets ?? []).map((a) => [a.dest, a]));
  const after = new Map((live.assets ?? []).map((a) => [a.dest, a]));
  const rows: ChangeRow[] = [];
  for (const [dest, a] of after) {
    const was = before.get(dest);
    if (!was) {
      rows.push({ group: 'assets', op: 'add', label: dest, to: pin(a.source), project: projectOf(a.source) });
      continue;
    }
    const row = assetChange(dest, was, a);
    if (row) rows.push(row);
  }
  for (const [dest, a] of before) {
    if (!after.has(dest)) {
      rows.push({ group: 'assets', op: 'remove', label: dest, from: pin(a.source), project: projectOf(a.source) });
    }
  }
  return sorted(rows);
}

function assetChange(dest: string, was: DeclaredAsset, now: DeclaredAsset): ChangeRow | null {
  const from = pin(was.source);
  const to = pin(now.source);
  if (from !== to) {
    const project = projectOf(was.source);
    return {
      group: 'assets',
      op: 'change',
      label: dest,
      from,
      to,
      project: project && project === projectOf(now.source) ? project : undefined,
    };
  }
  if (was.required !== now.required) {
    return { group: 'assets', op: 'change', label: dest, from: String(was.required), to: String(now.required) };
  }
  if (authored(was.display) !== authored(now.display)) {
    return { group: 'assets', op: 'change', label: dest };
  }
  return null;
}

/**
 * What a source points at, as one comparable string. A static asset is its
 * path: the bytes behind it can change without the config moving at all, which
 * is a difference no config diff can see and the build's own preview does.
 */
function pin(source: SourceDecl): string {
  switch (source.type) {
    case 'modrinth':
      return source.version_id;
    case 'smrt_cache':
      return source.sha1;
    case 'smrt_static':
      return source.rel_path;
  }
}

function projectOf(source: SourceDecl): string | undefined {
  return source.type === 'modrinth' ? source.project_id : undefined;
}

/** A display block reduced to what a person wrote in it. */
function authored(display: Display | undefined | null): string {
  if (!display) return '';
  const kept: Record<string, unknown> = {};
  for (const key of AUTHORED_DISPLAY) {
    const value = (display as Record<string, unknown>)[key];
    if (value !== undefined && value !== null && !(Array.isArray(value) && value.length === 0)) {
      kept[key] = value;
    }
  }
  // An absent block and one holding only derived keys are the same thing to a
  // reader, and must compare equal: the fill writing its first `requires` list
  // onto a mod is not an edit anyone made.
  return Object.keys(kept).length ? JSON.stringify(kept) : '';
}

function enabled(on: boolean): string {
  return on ? 'on' : 'off';
}

function scalar(v: unknown): string {
  return v === null || v === undefined ? '' : String(v);
}

/** Additions first, then removals, then moves; alphabetical inside each. */
function sorted(rows: ChangeRow[]): ChangeRow[] {
  const rank = { add: 0, remove: 1, change: 2 };
  return rows.sort((a, b) => rank[a.op] - rank[b.op] || a.label.localeCompare(b.label));
}
