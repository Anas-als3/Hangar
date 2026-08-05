# CLAUDE.md
Read SPEC.md before any work. Build only the current milestone.
Scope: §3 OUT list is absolute — flag, don't build. §16 is a parking lot, not a backlog.
Rust: tokio types only, no own runtime (§4). Tauri plugins: dialog, opener, single-instance — never shell.
TS: strict, no `any`. New dependency = one-line justification comment at the import.
The §7 command/event API is FROZEN — implement subsets, never rename or reshape.
The §6 state machine and §8 process rules (one spawn helper, Job Objects, stdin null,
env resolution, kill-then-status ordering) are the highest-priority correctness requirements.
A milestone is not done until `cargo check`, `npx tsc --noEmit`, and `npm run build`
all exit 0 — run them and show the output before claiming done.
If a Tauri/plugin API doesn't match this spec's snippet, trust the compiler and current
docs, keep the spec's INTENT, and note the deviation in a code comment.
UI must follow §11 exactly — tokens, fonts, phase strip. No generic defaults.
