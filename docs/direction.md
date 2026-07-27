# Direction

Where the interface is heading, and the reading of why it is where it is. This is
course, not settled decisions (those live in `docs/adr/`) and not tracked work
(that lives in the issues). It exists so the decisions have a frame, and so a new
decision can be checked against it.

## What the mirror is

Infrastructure, not a storefront. A self-hostable mirror of mods, a registry, a
pack build system, moderation, identity. The public catalog is the most visible
tip, not the substance. There is no clean line between a consumer and an author:
someone browses, forks, edits, builds. Treating half of it as a passive place to
browse packs undersells it and mis-shapes the UI.

## Why it is what it is

Much of the code was written by AI under a fixed ruleset. It is rigorous exactly
where a rule existed (security, the side and match-policy invariants, contrast
maths) and operator-brained exactly where a rule did not (newcomer UX, mod
discovery, human error messages). An AI applies a rule where it recognises the
pattern and falls to its defaults elsewhere, and those defaults are technically
correct and humanly cold.

The fake aria-labels are the seam. The rule "add an aria-label" fired everywhere;
the rule "the label names the field in the user's language" did not exist, so the
placeholder example got dropped in. The English error messages, the doctrine
written as poetry, the form fields showing raw schema names (`project_id`,
`sha1`) are the same shape: the mechanical half done, the human half left to a
default.

So the interface is not bad. It is a strong engine under roughly a third of a
product, built for one expert who holds the model in their head, and it grinds
anyone who does not.

## Posture

Progressive disclosure. A UI that leads. Depth available but not imposed,
revealed as you engage, instead of the current all-or-nothing where an operator
is assumed to know everything and a newcomer is shown nothing. The editor idiom
(ADR 0004) is the concrete form of this on the authoring surfaces. The graph
already does a little of it and is the proof it is possible here.

## What matters first, in order

1. Data. Silent config loss on concurrent edits (#52). Builds that die when
   Modrinth is down (#57).
2. Flow traps. Adding a mod blind, with no sight of what it pulls (#53). The
   editor you cannot leave with the back button (#54).
3. Friction and comprehension. Forms with no field names (#55). No mod search.
   No loading states. Jargon in the user's face.
4. Cosmetics. Transliterations, wrong labels, tooltips in the wrong language. The
   cheapest class and the least of the evils.

The largest single gap is search. A pack cannot be assembled without the Modrinth
website open alongside, because the mirror only lets you add a mod whose exact
name you already know. It is still unfiled.

## How to write things down

Plainly, and so they can be checked. "Motion may overshoot 4px on controls" can
be argued with in a review; "anything that springs reads as a different product"
cannot, and that is how the doctrine rotted until its own author could not read
it. A decision cited but not readable is worse than none. When two of us
understood a decision differently, the record is the reconciliation: draft it,
correct where it diverges, and treat the divergence as signal.
