# Plan 008: Pin the frozen §7 wire contract with tests on both untested shapes and a TS drift guard

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 91be38f..HEAD -- src-tauri/src/registry.rs src-tauri/src/process.rs src-tauri/src/commands.rs src/types.ts`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

SPEC.md §7 freezes the command/event API. The Rust side serializes with serde
(`rename_all = "camelCase"` / kebab-case statuses) and `src/types.ts` mirrors
those shapes **by hand** — its own header says "Single source of truth for the
wire shapes, mirroring the Rust structs". Nothing checks the mirror. A rename on
either side passes all five gates (`cargo test` is Rust-self-consistent,
`tsc --noEmit` is TS-self-consistent) and fails only at runtime as an undefined
field in a card or an event that never matches. An audit on 2026-08-06 verified
the shapes currently match — this plan is what KEEPS them matching.

Three §7 payloads also have zero serialization tests today: `ProjectView` (the
payload of `get_projects`, i.e. every card, and the riskiest — it uses
`#[serde(flatten)]`), `LogLinesPayload` (the higher-volume event), and
`RegistryError` (drives the corrupt-registry banner). Their siblings
(`Status`, `Project`, `LogLine`, `StatusChangedPayload`) are already tested;
these three were never backfilled.

## Current state

Files:

- `src-tauri/src/registry.rs` — `Status` (~line 21), `Project` (~line 34),
  `ProjectView` (~line 62: `#[serde(flatten)] project: Project` + `status` +
  `path_exists`), `RegistryError` (~line 87: `backup_path: Option<String>`,
  `error: String`). Existing wire tests in its test module (~line 296+):
  `project_serde_round_trip_is_camel_case`, `status_serializes_kebab_case`.
- `src-tauri/src/process.rs` — `LogLine`/`Stream` (~line 60-80),
  `StatusChangedPayload` and `LogLinesPayload` (~lines 330-345). Existing tests
  `log_line_serializes_to_the_frozen_wire_shape` and
  `status_changed_payload_is_camel_case` (~line 2020+) are the structural
  pattern to copy:

```rust
    #[test]
    fn status_changed_payload_is_camel_case() {
        let json = serde_json::to_string(&StatusChangedPayload {
            project_id: "abc".into(),
            status: Status::Crashed,
            message: None,
        })
        .unwrap();
        assert_eq!(json, r#"{"projectId":"abc","status":"crashed"}"#);
    }
```

- `src-tauri/src/commands.rs` — `to_view` (~line 62) derives `status` (map miss
  → `Status::Stopped`) and `path_exists`; `get_projects` (~line 74) preserves
  registry array order ("Array order is the display order — no sorting, ever
  (SPEC.md §11)"). Zero tests in this module today.
- `src/types.ts` — the hand-written mirror. Relevant: `Status` union of eight
  kebab-case strings, `Project`, `ProjectView extends Project { status;
  pathExists }`, `LogLine`, `StatusChangedPayload`, `LogLinesPayload`,
  `Settings`, `RegistryError { backupPath: string | null; error: string }`.

Conventions: tests live in `#[cfg(test)] mod tests` at the bottom of each Rust
file; test names are full sentences in snake_case; wire tests assert the exact
JSON string. No new dependencies (SPEC.md §4 rule) — `serde_json` is already a
direct dependency.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust tests | `cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| Rust check | `cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| TypeScript | `npx tsc --noEmit` | exit 0 |
| Frontend build | `npm run build` | exit 0 |

(`cargo` lives in `~/.cargo/bin` — prefix `PATH="$HOME/.cargo/bin:$PATH"` if not found.)

## Scope

**In scope** (the only files you should modify):
- `src-tauri/src/registry.rs` (test module only)
- `src-tauri/src/process.rs` (test module only)
- `src-tauri/src/commands.rs` (test module — create it)

**Out of scope** (do NOT touch):
- `src/types.ts` — if a drift test FAILS, that is a finding to report, not a
  license to edit the frozen contract on either side.
- Any non-test Rust code. This plan adds tests only.
- Adding any dependency or JS test runner.

## Git workflow

- Work on `main`. One commit:
  `Add wire-contract tests for ProjectView, LogLinesPayload, RegistryError and a types.ts drift guard`

## Steps

### Step 1: Serialization tests for the three untested shapes

In the matching files' test modules, following the
`status_changed_payload_is_camel_case` pattern exactly:

1. `registry.rs`: `project_view_flattens_project_and_adds_derived_fields` —
   build a fully-populated `Project` (every Option = Some), wrap in
   `ProjectView { project, status: Status::Running, path_exists: true }`,
   assert the exact JSON: flattened camelCase project fields at the top level
   plus `"status":"running","pathExists":true`. Assert `"project"` does NOT
   appear as a key (the flatten must stay flat).
2. `registry.rs`: `registry_error_serializes_backup_path_as_nullable_camel_case`
   — both `backup_path: Some(...)` and `None`; `None` must serialize as
   `"backupPath":null` (NOT be omitted) because `types.ts` declares
   `string | null`, not optional. If the current serde attributes omit it
   instead, that IS drift — STOP and report rather than changing either side.
3. `process.rs`: `log_lines_payload_is_camel_case` — one payload, two lines,
   exact JSON.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass,
3 new tests visible in the output.

### Step 2: The types.ts drift guard

In `registry.rs`'s test module add
`every_wire_key_the_backend_emits_appears_in_types_ts`:

```rust
    #[test]
    fn every_wire_key_the_backend_emits_appears_in_types_ts() {
        // The §7 contract is frozen and mirrored BY HAND in src/types.ts; this
        // test is the only thing linking the two sides. It is deliberately
        // key-presence only — exact-shape assertions live in the per-struct
        // wire tests.
        let types_ts = include_str!("../../src/types.ts");
        let samples: Vec<serde_json::Value> = vec![
            serde_json::to_value(/* fully-populated ProjectView */).unwrap(),
            serde_json::to_value(/* StatusChangedPayload with message: Some */).unwrap(),
            serde_json::to_value(/* LogLinesPayload with one line */).unwrap(),
            serde_json::to_value(/* Settings */).unwrap(),
            serde_json::to_value(/* RegistryError with backup_path: Some */).unwrap(),
        ];
        for sample in samples {
            assert_keys_in(&sample, types_ts);
        }
    }
```

`assert_keys_in` walks the JSON value recursively; for every object key it
asserts `types_ts.contains(key)`, with a failure message naming the missing
key. Also assert every `Status` variant's wire string
(serialize each of the eight) appears in `types_ts` — that covers the kebab-case
union. Use every fully-populated sample (all `Option`s = `Some`) so
`skip_serializing_if` fields are present.

Note the limitation in a comment: this catches Rust-side keys missing from TS;
it cannot catch TS-side keys the backend never emits. That direction is
protected by `tsc --noEmit` failing when the frontend reads a field it doesn't
declare — acceptable.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → passes. Then
prove the guard bites: temporarily rename `pathExists` to `path_exists` in
`types.ts`, run the test, confirm it FAILS naming `pathExists`, revert, confirm
it passes. Record this in your report.

### Step 3: `commands.rs` derived-field tests

Create `#[cfg(test)] mod tests` in `commands.rs`:

1. `a_project_absent_from_the_runtime_map_reads_stopped` — `to_view` with an
   empty `RuntimeMap` → `status == Status::Stopped`, and `path_exists` false
   for a nonexistent path (use a path under `std::env::temp_dir()` that you do
   not create).
2. `a_project_present_in_the_runtime_map_reflects_its_status` — insert a
   `ProjectRuntime` with `status: Status::Running` → view says `Running`.

(`to_view` takes plain `&Project` / `&RuntimeMap` — no `AppHandle` needed. If
it is private, test it in-module; do not make it pub.)

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass.

### Step 4: All gates, then commit

**Verify**: the four gate commands green; `git status` shows only the three
in-scope files.

## Test plan

Covered by the steps: 3 shape tests + 1 drift guard + 2 derived-field tests =
6 new tests, all in existing gate commands. The step 2 bite-check (deliberate
break, observe failure, revert) is the meta-test and must be in the report.

## Done criteria

- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` passes with 6 new tests
- [ ] The drift guard demonstrably fails on a deliberate `types.ts` rename (recorded in report, change reverted)
- [ ] `npx tsc --noEmit` and `npm run build` exit 0 (nothing frontend changed)
- [ ] No non-test code modified (`git diff` shows test modules and nothing else)
- [ ] `plans/README.md` status row for 008 updated

## STOP conditions

Stop and report back if:

- Any new shape test fails against the CURRENT code — that means live drift
  exists today and both sides need a maintainer decision, not a silent fix.
- `RegistryError`'s `None` serializes omitted rather than as `null` (see step 1.2).
- `include_str!("../../src/types.ts")` fails to resolve — report the actual
  relative path rather than moving files.

## Maintenance notes

- When §7 legitimately grows (plan 005 adds `add_project`/`update_project`/
  `remove_project`/`read_package_json` payloads), extend BOTH the per-struct
  tests and the drift-guard sample list — the guard only checks what it is fed.
- Reviewer should scrutinize: samples are fully-populated (an `Option::None`
  sample silently skips its key), and the flatten test asserts the absence of a
  nested `"project"` key.
