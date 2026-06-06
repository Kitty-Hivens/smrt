// English strings. This dictionary is the key source of truth: `ru` is typed
// against its keys, so a missing or stray Russian key is a compile error.

export const en = {
  'app.checkingSession': 'Checking session...',
  'common.loading': 'Loading...',

  'nav.overview': 'Overview',
  'nav.packs': 'Packs',
  'nav.servers': 'Servers',
  'nav.featured': 'Featured',
  'nav.cache': 'Cache',

  'shell.signOut': 'Sign out',
  'shell.refresh': 'Refresh',
  'shell.health': 'v{version} / schema {schema}',
  'shell.locale': 'Language',

  'login.subtitle': 'Mirror admin. Paste the admin token to continue.',
  'login.submit': 'Enter',
  'login.checking': 'Checking...',
  'login.rejected': 'Rejected. Check the admin token.',
  'login.foot': 'smrt mirror control panel',

  'dialog.confirmTitle': 'Confirm',
  'dialog.inputTitle': 'Input',
  'dialog.ok': 'OK',
  'dialog.cancel': 'Cancel',
  'dialog.delete': 'Delete',

  'packs.new': 'New pack',
  'packs.newPrompt': 'New pack id (letters, digits, - _ .):',
  'packs.col.pack': 'Pack',
  'packs.col.mc': 'MC',
  'packs.col.latest': 'Latest',
  'packs.col.tags': 'Tags',
  'packs.col.flags': 'Flags',
  'packs.unbuilt': '(unbuilt)',
  'packs.flag.featured': 'featured',
  'packs.flag.authoring': 'editable',
  'packs.empty': 'No packs yet. Create one or bootstrap from an SC archive.',
} as const;

export type Dict = Record<keyof typeof en, string>;
