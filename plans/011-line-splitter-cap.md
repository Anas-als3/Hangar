# Plan 011: Bound `LineSplitter`'s buffer so a newline-less stream cannot grow memory without limit

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 91be38f..HEAD -- src-tauri/src/process.rs`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

SPEC.md §8's log pipeline has three overload protections: 4 KB per-line
truncation, a 500-line ring, and a 2000-line flush cap. All three engage only
when a line *break* arrives. `LineSplitter::push` appends every non-break byte
to an internal `Vec<u8>` with no cap, so a child that emits a long run of bytes
with no `\n`/`\r` — a minified bundle printed to stdout, a binary blob, a
progress writer that redraws with escape codes only — grows Hangar's resident
memory linearly and without bound. The reader loop feeds it 8 KB chunks
forever. One misbehaving child should never be able to OOM the supervisor.

## Current state

`src-tauri/src/process.rs`:

- Constants (~line 33-36): `MAX_LINE_BYTES: usize = 4096`,
  `TRUNCATION_MARKER: &str = " …[truncated]"`.
- `LineSplitter` (~line 1080-1140):

```rust
pub struct LineSplitter {
    buf: Vec<u8>,
    /// The previous byte was `\r`, so a following `\n` is the same break, not an empty line.
    pending_cr: bool,
}

impl LineSplitter {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &byte in chunk {
            match byte {
                b'\r' => { ... }
                b'\n' => { ... }
                _ => {
                    self.buf.push(byte);
                    self.pending_cr = false;
                }
            }
        }
        out
    }

    pub fn finish(&mut self) -> Option<String> { self.take_line() }

    fn take_line(&mut self) -> Option<String> {
        let bytes = std::mem::take(&mut self.buf);
        let line = sanitize_line(&String::from_utf8_lossy(&bytes));
        if line.is_empty() { None } else { Some(line) }
    }
}
```

- `sanitize_line` = `truncate_line(strip_ansi(raw))` — truncation happens in
  `take_line`, i.e. only at a break.
- The reader (`spawn_reader`, ~line 1285): reads 8 KB chunks in a loop,
  `splitter.push(&chunk[..n])`.
- Existing splitter tests (~line 1650+, names like
  `splits_on_carriage_returns_and_newlines`,
  `a_multi_byte_char_split_across_chunks_survives`) — the structural pattern
  for new tests.

Subtlety the cap must respect: ANSI escape sequences are stripped BEFORE
truncation, so a buffer capped at exactly `MAX_LINE_BYTES` raw bytes could
strip down to well under 4 KB of visible text and lose real content that the
uncapped version kept. Cap with slack (see step 1) rather than at the exact
line limit.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| Windows check | `PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` | exit 0 |
| TypeScript / build | `npx tsc --noEmit && npm run build` | exit 0 |

## Scope

**In scope**:
- `src-tauri/src/process.rs` — `LineSplitter` and its tests only.

**Out of scope** (do NOT touch):
- `strip_ansi`, `truncate_line`, `sanitize_line` — their behaviour is pinned by
  tests and shared.
- The reader/flusher tasks, ring buffer, batching — no change.
- `MAX_LINE_BYTES` / `TRUNCATION_MARKER` values — §8 requirements.

## Git workflow

- Work on `main`. One commit:
  `Cap LineSplitter's buffer so a newline-less stream cannot grow memory unboundedly`

## Steps

### Step 1: The cap

Add to `LineSplitter` a `discarding: bool` field (default false) and a buffer
cap constant local to the splitter:

```rust
/// Raw-byte cap on the pending line. 4× the visible-line limit so that a line
/// heavy with ANSI escapes (stripped BEFORE truncation) still yields its full
/// 4 KB of visible text; beyond this, the stream is not line-oriented and the
/// §8 protections must engage without waiting for a break that may never come.
const MAX_PENDING_BYTES: usize = MAX_LINE_BYTES * 4;
```

In `push`'s default arm: if `discarding`, drop the byte (do not buffer). Else
push; if `buf.len()` just reached `MAX_PENDING_BYTES`, emit the (sanitized,
truncated — `take_line` already does both) contents as a line into `out` now,
and set `discarding = true`.

In BOTH break arms (`\r` and `\n`): if `discarding` is set, clear it and treat
the break as consuming the already-emitted line — i.e. reset `pending_cr`
bookkeeping as today but do NOT emit an extra empty line for the `\n` of a
discarded tail. (Concretely: in the `\n` arm, when `discarding` was true,
behave as the CRLF-second-half case does — skip the emit.) `finish()` on a
discarding splitter returns `None` (buf is empty).

Guarantee after this step: `buf` never exceeds `MAX_PENDING_BYTES`; a capped
line always carries the existing `" …[truncated]"` marker (via `truncate_line`,
since `MAX_PENDING_BYTES > MAX_LINE_BYTES` even after stripping typical
escapes).

**Verify**: `cargo check` → exit 0; `cargo test` → ALL existing splitter tests
still pass unmodified (they never exceed the cap, so behaviour is identical).

### Step 2: Tests

Add, following the existing splitter-test style:

1. `a_stream_with_no_line_breaks_is_capped_and_marked_truncated` — push
   `MAX_PENDING_BYTES + 10_000` bytes of `b'x'` in 8 KB chunks (mirroring the
   reader); assert exactly ONE line comes out across all pushes, it ends with
   `TRUNCATION_MARKER`, and the splitter's buffer stays ≤ `MAX_PENDING_BYTES`
   (expose via `#[cfg(test)] fn pending_len(&self) -> usize`).
2. `bytes_after_the_cap_are_dropped_until_the_next_break` — after the capped
   emit, push `b"IGNORED\nnext line\n"`; assert output is exactly
   `["next line"]` (the tail of the over-long line vanished, the next real line
   survives intact).
3. `a_capped_line_heavy_with_ansi_still_keeps_its_visible_text` — build input
   as ~3 KB of visible text interleaved with ~10 KB of `\x1b[32m`-style
   escapes, no break; assert the emitted line contains the full visible text
   (not cut below `MAX_LINE_BYTES` of visible content) — this pins the
   4×-slack rationale.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass,
3 new tests in the output.

### Step 3: All gates, then commit

**Verify**: the four gate commands green; `git status` shows only
`src-tauri/src/process.rs`.

## Test plan

The three tests in step 2, plus the unmodified pass of every existing splitter
test (which proves sub-cap behaviour is byte-identical).

## Done criteria

- [ ] All gates green; 3 new tests pass; zero existing tests modified
- [ ] `grep -n "MAX_PENDING_BYTES" src-tauri/src/process.rs` → constant + uses
- [ ] No files outside `src-tauri/src/process.rs` modified
- [ ] `plans/README.md` status row for 011 updated

## STOP conditions

Stop and report back if:

- Any EXISTING splitter test fails — sub-cap behaviour must not change at all.
- The cap interacts with `pending_cr` in a way that makes test 2 emit an empty
  line and one fix attempt doesn't resolve it cleanly — report the exact
  byte-sequence trace instead of special-casing further.

## Maintenance notes

- If `MAX_LINE_BYTES` ever changes (spec decision), `MAX_PENDING_BYTES` scales
  with it automatically — keep the multiplier, not an absolute.
- Reviewer should scrutinize: the discard flag resets on BOTH break kinds, and
  `finish()` after a capped stream returns `None` rather than a duplicate line.
