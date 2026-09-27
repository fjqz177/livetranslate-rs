# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root. Created lazily by `/domain-modeling` when terms actually get resolved; if absent, proceed silently.
- **Decision ledger `docs/decisions.md`**: this repo records decisions as a single ledger — `D-xx` (behavior/product) and `ADR-x` (architecture). The Matt-skills notion of "ADR" maps onto ledger entries. Read entries touching the area you're about to work in.

If any of these don't exist, **proceed silently** — don't flag their absence, don't suggest creating them upfront. **Do not create `docs/adr/`**: the ledger is the single decision source.

## File structure

Single-context repo:

```
/
├── CONTEXT.md            (lazy, created by /domain-modeling)
├── docs/decisions.md     (decision ledger: D-xx / ADR-x)
└── crates/               (ten-crate Cargo workspace, one product domain)
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag decision-ledger conflicts

If your output contradicts an existing ledger entry, surface it explicitly rather than silently overriding:

> _Contradicts D-82 (translation domain: 对外只给翻译接口), but worth reopening because…_
