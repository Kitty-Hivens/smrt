# Architecture decision records

Each file here records one decision: the context that forced it, what was
decided, what was rejected, and what it costs. One decision per file, numbered in
the order they were made. A record is written so it can be argued with later,
which is the whole point. A decision cited but not readable is how rationale gets
lost.

Numbering: 0001 and 0002 are referenced in the code (`src/domain/manifest.rs`,
`src/domain/diff.rs`) but were never written down. They are backfill debt, not
free numbers. New records start at 0003.

Format, kept short on purpose:

- **Title** with the number.
- **Status**: proposed, accepted, shipped, superseded.
- **Context**: what forced the decision.
- **Decision**: what we chose, concretely.
- **Rejected**: what we did not choose, and why.
- **Consequences**: what it buys and what it costs, including the debts it leaves.

Write plainly. A rule you can point at in review beats a sentence that sounds
right and means nothing.
