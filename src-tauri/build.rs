use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    tauri_build::build();

    // SPEC.md §11 "Build freshness" (plan 063) — the compile-time half of "is the bundle on disk
    // newer than the process running it". See `src/freshness.rs` for how it is used.
    //
    // WHY A BUILD-SCRIPT CONSTANT AND NOT THE RUNNING BINARY'S MTIME. `npm run install:app` is
    // `rm -rf /Applications/Hangar.app && cp -R …`, so (a) a copy carries the copy's mtime, not the
    // build's, and (b) the file the running process was launched from is the very file that gets
    // replaced — stat it at check time and you read the NEW build's mtime as though it were your
    // own, so the check could never fire. A value baked in at compile time cannot be moved by a
    // copy.
    //
    // THIS VALUE MAY LAG. Cargo re-runs a build script only when something in its `rerun-if-changed`
    // set changed, so a rebuild in which no Rust source and no Tauri config changed (a
    // frontend-only edit, which is re-embedded from `../dist` by `generate_context!`) keeps the
    // previous stamp. That is safe HERE AND ONLY HERE because `freshness.rs` never uses this
    // constant on its own: it takes the LATER of this stamp and the executable's own mtime read at
    // startup. A stale stamp can therefore only ever be ignored — it can never turn into a false
    // "you are running an old build", which is the one failure this feature must not have.
    let built_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=HANGAR_BUILD_UNIX_TIME={built_at}");
    // Narrow on purpose: enough that a Rust or dependency change re-stamps the constant, without
    // pointing at `../dist` (absent in a fresh checkout, which cargo treats as "changed" and would
    // re-run this script — and so recompile the crate — on every single `cargo check`).
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
}
