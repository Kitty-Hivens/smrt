// Rung-aware role checks. Roles rank member < admin < debug (#39), so an admin
// gate must admit debug too -- a bare `role === 'admin'` would lock a debug user
// out of the operator surface. The debug gate is exact.

type MaybeRole = string | null | undefined;

/** admin-and-up: the operator surface. Debug outranks admin, so it passes. */
export function isOperator(role: MaybeRole): boolean {
  return role === 'admin' || role === 'debug';
}

/** the debug rung: compat-affecting authoring (#39). */
export function isDebug(role: MaybeRole): boolean {
  return role === 'debug';
}
