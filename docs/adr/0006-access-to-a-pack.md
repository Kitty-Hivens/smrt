# 0006. Access is a grant on a pack, not a rung on the mirror

Status: accepted, not yet built

## Context

Two people can already edit one pack at the same time: edits merge live (#115),
presence says who is in the room, the markers say what they touched, and a
commit names everyone whose work it took in (#122). The machinery for working
together is built and running.

Nobody can be let in to use it.

Who may author a pack is one function of an identity (`http/auth.rs:311`): the
owner of a community namespace (`u/<uid>/<pack>`), or an admin. There is no
third answer. To let one person help with one pack, the only move available is
`POST /v1/users/{uid}/role` -- which hands them every pack on the mirror, the
registry, moderation, takedowns and the user list. The pack's own history
records contributors and cannot admit one.

The way out that exists is a fork: `POST /v1/me/forks` copies a pack into the
forker's namespace and records `fork_of`. It is a one-way street. `fork_of` is
read nowhere except to display where a pack came from; there is no request, no
review, no merge. Someone who improves a fork of your pack has no way to offer
the improvement back, and you have no way to take it.

Scoped curator grants were considered before and dropped (#45). What has changed
is that the collaboration underneath them now exists, so the grant is no longer
an abstraction -- it is the missing half of a feature that already works.

## Decision

**Three levels, granted per pack: `view`, `edit`, `own`.**

- `view` reads a draft, its history and its reports. Without it, showing an
  unpublished pack to someone means publishing it or handing over the keys.
- `edit` writes the config, commits, and builds. This is the level that makes
  the live merge reachable by a second person.
- `own` also grants and revokes access, changes visibility, and deletes.

The namespace owner has `own` on their packs by construction, and an admin has
`own` everywhere. Neither is a row in a table: they are the two rules the gate
knows before it reads anything. A grant is only ever needed for the third
answer -- somebody who is neither.

**One gate, asynchronous, ahead of every authored read and write.**
`authorize(state, identity, pack_id, level)` replaces `may_author` in all 39
places that call it, including the draft-visibility check on the public side
(`http/public.rs:476`), which is the same question asked about `view`.

**Grants live in `accounts.db`**, beside users and sessions. They are a record
about people, not about pack content. The pack's own config is the wrong home
twice over: it is authored by clients, and it is merged live by a CRDT -- a
permission that an editor can rewrite is not a permission. Every grant and
revocation is an audit entry, because "who let them in" is exactly the question
asked after something goes wrong.

**Proposing changes stays a separate layer, built on this one.** A person with
`view` and a fork will be able to offer their state back: the snapshot, the diff
between two states and the conditional write all exist (#122), so a proposal is
those parts plus a request and a decision. This record does not design it; it
makes it possible, and deliberately does not smuggle it in early.

## Rejected

- **Keeping roles as the only answer.** Making a helper an admin of the whole
  mirror to let them touch one pack is not a permission model, it is the absence
  of one.
- **Grants in the pack config.** It is the file two editors are merging into,
  and the file a restore rewrites wholesale. Access would be editable by the
  people it restrains, and would travel into forks and duplicates.
- **A longer ladder** (triage, maintain, admin, and so on). This mirror is run
  by a handful of people; every level past the third would be a distinction
  nobody here needs and every handler would have to answer for.
- **Per-endpoint checks instead of one gate.** Thirty-nine call sites already
  disagree about when they check; multiplying that by three levels is how a
  hole appears in the one place nobody reread.

## Consequences

- `may_author` disappears. Every call site becomes `authorize(...).await?`, and
  handlers that gate before touching storage keep that order -- an unauthorized
  caller must not learn whether a pack exists by timing.
- The gate reads sqlite on every authored request, as session resolution already
  does.
- A draft becomes showable: `view` is what "let me show you before it ships"
  means, and it is what makes a proposal reviewable by its author later.
- The panel needs a place to see and change this -- an access list on the pack,
  with the two rules (owner, admin) stated rather than hidden, so it is clear
  why a name is on the list without being in the table.
