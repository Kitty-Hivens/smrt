// Shared reload signal so the shell's top bar can refresh the active view.
// The view runs its fetch on mount and whenever `count` changes; the button
// bumps `count` and reads `busy` to show progress.
let count = $state(0);
let busy = $state(false);

export const reload = {
  get count() {
    return count;
  },
  get busy() {
    return busy;
  },
  request() {
    count++;
  },
  setBusy(v: boolean) {
    busy = v;
  },
};
