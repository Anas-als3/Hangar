/**
 * SPEC.md §10 — Add/Edit dialog: folder picker → package.json scripts → port suggestion →
 * editable command → optional url override (edit only) → save.
 *
 * Duplicate-port / not-stopped rejections are enforced by Rust (§7); this dialog just calls
 * addProjectAction/updateProjectAction and lets `setToast` surface whatever comes back.
 */
import { useEffect, useState } from "react";
// Native folder picker (§10 step 1) — dialog plugin only, never tauri-plugin-shell (§4).
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { findFreePort, readPackageJson } from "../api";
import {
  addProjectAction,
  closeDialog,
  stopIfRunningWithConfirm,
  updateProjectAction,
  useHangarStore,
} from "../store";
import { relativeTime } from "../status";
import type { PackageJsonInfo, Project, ProjectStack, ProjectView } from "../types";
// §10 step 4: the token rewrite is a separate zero-import module (node --test coverage, plan 043)
// — never inlined here, so the framework/package-manager table stays in one place.
import { rewritePortToken, type PortTokenForm } from "../portToken";

/** §10 step 3: `npm run <script>` / `pnpm run <script>` / `yarn <script>` per package manager. */
function commandFor(pm: PackageJsonInfo["packageManager"], script: string): string {
  if (pm === "npm") return `npm run ${script}`;
  if (pm === "pnpm") return `pnpm run ${script}`;
  return `yarn ${script}`;
}

/** §10 step 2: pre-select `dev` if present, else `start`, else the first script found. */
function pickDefaultScript(scripts: Record<string, string>): string | null {
  if ("dev" in scripts) return "dev";
  if ("start" in scripts) return "start";
  const keys = Object.keys(scripts);
  return keys.length > 0 ? keys[0] : null;
}

/** §10 step 4: `find_free_port`'s starting point when the Port field is empty (unsniffed
 * framework, no package.json) — otherwise there is nothing meaningful to walk upward from. */
const DEFAULT_PORT_FALLBACK = 3000;

/** SPEC.md §10 step 4's caption, shown `aria-live="polite"` beneath the Port field. */
function describePortPick(
  from: number,
  result: number,
  others: ProjectView[],
  framework: string | undefined,
): string {
  if (result === from) {
    return framework
      ? `Pinned :${result} (${framework}'s default, free right now). Command updated.`
      : `Pinned :${result} (free right now). Command updated.`;
  }
  const owner = others.find((p) => p.port === from);
  if (owner) {
    return `Pinned :${result} — ${from} is pinned by ${owner.name}. Command updated.`;
  }
  return `Pinned :${result} and updated the command. (${from} is in use right now.)`;
}

/**
 * Fix 3 (plan 034): the TS mirror of `registry.rs`'s `extract_url_port` — deliberately not a full
 * URL parser (no dependency for one call site), just enough to find an explicit `host:port`
 * between the scheme and the next `/`, `?` or `#`. Kept in lockstep with the Rust original, which
 * stays the tested canonical definition.
 */
function extractUrlPort(url: string): number | null {
  const schemeIdx = url.indexOf("://");
  const afterScheme = schemeIdx === -1 ? url : url.slice(schemeIdx + 3);
  const cut = ["/", "?", "#"]
    .map((c) => afterScheme.indexOf(c))
    .filter((i) => i !== -1);
  const authority = afterScheme.slice(0, cut.length > 0 ? Math.min(...cut) : afterScheme.length);
  // Last colon, not first — an IPv6 literal's own colons (`[::1]:3000`) must not confuse the split.
  const lastColon = authority.lastIndexOf(":");
  if (lastColon === -1) return null;
  const portStr = authority.slice(lastColon + 1);
  if (!/^\d+$/.test(portStr)) return null;
  const parsed = Number(portStr);
  return parsed <= 65535 ? parsed : null;
}

/**
 * SPEC.md §6's amended run-inert bullet, mirrored here — ONE named list, not a second
 * hand-picked "guarded fields" list (that duplication is exactly how this bug class returns; see
 * `commands.rs`'s `is_run_inert_change`, the canonical definition this must be kept in lockstep
 * with). The frontend cannot call Rust's predicate directly (no §7 command for it, and none is
 * warranted — plan 048), so `isRunInertChange` below recomputes the same structural comparison
 * locally, just for deciding whether Save needs the confirm-and-stop first.
 */
const RUN_INERT_FIELDS = ["notes", "folderId", "folderName", "openBrowserOnReady"] as const;

/**
 * Structural comparison with `RUN_INERT_FIELDS` normalised out on both sides — same shape as
 * `is_run_inert_change`, deliberately not a hand-enumerated list of fields that ARE guarded, so a
 * field added to `Project` later is guarded by default until it is added to the list above.
 */
function isRunInertChange(stored: Project, incoming: Project): boolean {
  const keys = new Set<keyof Project>([
    ...(Object.keys(stored) as (keyof Project)[]),
    ...(Object.keys(incoming) as (keyof Project)[]),
  ]);
  for (const key of keys) {
    if ((RUN_INERT_FIELDS as readonly string[]).includes(key)) continue;
    if (JSON.stringify(stored[key]) !== JSON.stringify(incoming[key])) return false;
  }
  return true;
}

/** SPEC.md §5: non-blocking warning text, mirroring `url_port_mismatch_warning` in `registry.rs`. */
function urlPortMismatchWarning(url: string, port: number): string | null {
  const trimmed = url.trim();
  if (trimmed === "") return null;
  const urlPort = extractUrlPort(trimmed);
  if (urlPort === null || urlPort === port) return null;
  return "URL port differs from the ready-check port.";
}

export function AddEditDialog() {
  const { dialog, projects } = useHangarStore();
  const editing = dialog?.kind === "edit" ? dialog.project : null;
  const isOpen = dialog?.kind === "add" || dialog?.kind === "edit";

  const [name, setName] = useState(editing?.name ?? "");
  const [path, setPath] = useState(editing?.path ?? "");
  const [command, setCommand] = useState(editing?.command ?? "");
  const [port, setPort] = useState(editing ? String(editing.port) : "");
  // §5 `url`: "shown only in the Edit dialog (placeholder shows the computed default)".
  const [url, setUrl] = useState(editing?.url ?? "");
  const [updateOnRun, setUpdateOnRun] = useState(editing?.updateOnRun ?? true);
  const [openBrowserOnReady, setOpenBrowserOnReady] = useState(
    editing?.openBrowserOnReady ?? true,
  );
  const [readyTimeoutSec, setReadyTimeoutSec] = useState(editing?.readyTimeoutSec ?? 60);
  const [saving, setSaving] = useState(false);
  const [scripts, setScripts] = useState<Record<string, string>>({});
  const [selectedScript, setSelectedScript] = useState<string | null>(null);
  const [packageManager, setPackageManager] =
    useState<PackageJsonInfo["packageManager"]>("npm");
  // §5: app-owned, never hand-edited here — only ever set from a `readPackageJson` result
  // (below) or carried through unchanged from the project being edited.
  const [stack, setStack] = useState<ProjectStack | undefined>(editing?.stack);
  // §10 step 4 "Choose for me" — the aria-live caption beneath the Port field, and a guard against
  // a second click landing mid-lookup (find_free_port is the only awaited step).
  const [portCaption, setPortCaption] = useState<string | null>(null);
  const [portPickerBusy, setPortPickerBusy] = useState(false);

  // Re-initialize whenever the dialog target changes (opened for a different project, or
  // switched from Edit back to Add).
  useEffect(() => {
    setName(editing?.name ?? "");
    setPath(editing?.path ?? "");
    setCommand(editing?.command ?? "");
    setPort(editing ? String(editing.port) : "");
    setUrl(editing?.url ?? "");
    setUpdateOnRun(editing?.updateOnRun ?? true);
    setOpenBrowserOnReady(editing?.openBrowserOnReady ?? true);
    setReadyTimeoutSec(editing?.readyTimeoutSec ?? 60);
    setStack(editing?.stack);
    setSaving(false);
    setPortCaption(null);
    // Fix 2 (plan 034): scripts/selectedScript/packageManager are `handleBrowse` output, not
    // project fields — left unreset, they held the *previous* dialog target's values.
    setScripts({});
    setSelectedScript(null);
    setPackageManager("npm");
    // Plan 025: opening Edit on a project that predates plan 023 (or whose package.json has
    // changed since) should pick up its stack without forcing a re-browse. `setStack` above
    // seeds from `editing?.stack` so the dialog renders immediately; this overwrites once the
    // read resolves. §12: a moved/deleted folder must still open in Edit, so a failed read is
    // swallowed and whatever stack was already seeded is left alone.
    let cancelled = false;
    if (editing?.path) {
      void readPackageJson(editing.path)
        .then((info) => {
          if (!cancelled) setStack(info.stack);
        })
        .catch(() => {
          // no-op: keep the seeded editing?.stack
        });
    }
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dialog]);

  useEffect(() => {
    if (!isOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeDialog();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [isOpen]);

  if (!isOpen) return null;

  async function handleBrowse() {
    const selected = await openFolderDialog({ directory: true });
    if (typeof selected !== "string") return; // cancelled
    setPath(selected);
    if (!name.trim()) setName(selected.split(/[\\/]/).filter(Boolean).pop() ?? selected);
    // A caption naming *this* folder's port situation must not survive a browse to another one.
    setPortCaption(null);
    try {
      const info = await readPackageJson(selected);
      setScripts(info.scripts);
      setPackageManager(info.packageManager);
      const defaultScript = pickDefaultScript(info.scripts);
      setSelectedScript(defaultScript);
      if (defaultScript) setCommand(commandFor(info.packageManager, defaultScript));
      // §10 step 4: "a suggestion, never silent magic" — a folder with no detectable port must
      // not keep showing the *previous* folder's port as if it were a suggestion for this one.
      setPort(info.portSuggestion !== undefined ? String(info.portSuggestion) : "");
      setStack(info.stack);
    } catch {
      // §10 step 6: no/unparseable package.json falls back to manual command + port entry.
      setScripts({});
      setSelectedScript(null);
      setPackageManager("npm");
      setStack(undefined);
    }
  }

  function handleScriptChange(script: string) {
    setSelectedScript(script);
    setCommand(commandFor(packageManager, script));
  }

  const parsedPort = Number(port);
  const portValid = port.trim() !== "" && Number.isInteger(parsedPort) && parsedPort > 0;
  // Fix 4 (plan 034): the input's `min={1}` is advisory only — React does not enforce it, and
  // `AttemptBudget::new(0)` kills the tree on the very first poll (§9 step 7).
  const readyTimeoutValid = Number.isInteger(readyTimeoutSec) && readyTimeoutSec >= 1;
  const canSave =
    name.trim() !== "" &&
    path.trim() !== "" &&
    command.trim() !== "" &&
    portValid &&
    readyTimeoutValid;
  // Fix 3 (plan 034): advisory only — SPEC.md §5 requires this to be non-blocking, so it must
  // never be folded into `canSave`.
  const urlPortWarning = portValid ? urlPortMismatchWarning(url, parsedPort) : null;

  // §10 step 4 "Choose for me": one press picks a free port AND rewrites the command's port token
  // in the same press — the two halves are inseparable (SPEC.md §10 step 4, amended 2026-08-10),
  // because Hangar's `port` is a prediction of what the child binds, never an instruction to it.
  async function handleChooseForMe(form: PortTokenForm) {
    if (!path.trim() || portPickerBusy) return;
    // "Prefer the framework default when it is free. Do not move for no reason": walking from the
    // already-prefilled/edited port means find_free_port returns it immediately when it is free.
    const from = portValid ? parsedPort : DEFAULT_PORT_FALLBACK;
    const others = projects.filter((p) => p.id !== editing?.id);
    setPortPickerBusy(true);
    try {
      const result = await findFreePort(from, others.map((p) => p.port));
      if (result === null) {
        setPortCaption(`Couldn't find a free port near ${from} — enter one yourself.`);
        return;
      }
      setPort(String(result));
      setCommand((current) => rewritePortToken(current, result, form, stack?.framework));
      setPortCaption(describePortPick(from, result, others, stack?.framework));
    } finally {
      setPortPickerBusy(false);
    }
  }

  async function handleSave() {
    if (!canSave) return;
    setSaving(true);
    const payload = {
      name: name.trim(),
      path,
      command,
      port: parsedPort,
      url: url.trim() === "" ? undefined : url.trim(),
      updateOnRun,
      readyTimeoutSec,
      stack,
      openBrowserOnReady,
    };
    if (editing) {
      const nextProject: Project = { ...editing, ...payload };
      // §6/§10 step 7 (plan 048): the confirm-and-stop moved here from dialog-open — it now runs
      // only when the pending change reaches beyond the run-inert set (`isRunInertChange` above),
      // instead of guarding every open of the dialog. `projects` (not `editing`, a snapshot taken
      // when the dialog opened) is looked up for the live status, same as `ProjectCard`'s guarded
      // actions.
      if (!isRunInertChange(editing, nextProject)) {
        const live = projects.find((p) => p.id === editing.id);
        const okToProceed = live ? await stopIfRunningWithConfirm(live) : true;
        if (!okToProceed) {
          setSaving(false);
          return;
        }
      }
    }
    const ok = editing
      ? await updateProjectAction({
          ...editing,
          ...payload,
        })
      : await addProjectAction(payload);
    if (!ok) setSaving(false);
  }

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center" role="presentation">
      {/* §11 enter transition: backdrop fades, dialog eases in — enter only, see plan 018. */}
      <div
        className="hangar-fade-in absolute inset-0 bg-black/40"
        onClick={closeDialog}
        aria-hidden="true"
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={editing ? `Edit ${editing.name}` : "Add project"}
        className="hangar-dialog-in relative z-10 max-h-[90vh] w-[min(32rem,92vw)] overflow-y-auto rounded-lg border border-white/10 bg-surface p-6 shadow-2xl"
      >
        <h2 className="font-display text-lg font-medium text-text">
          {editing ? "Edit project" : "Add project"}
        </h2>

        {/* Enter-to-save (plan 039): defers entirely to handleSave's own `canSave` guard, exactly
            like the Save button's onClick below — no duplicate validation here. */}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void handleSave();
          }}
        >
        <label className="mt-5 block text-sm text-muted" htmlFor="project-name">
          Name
        </label>
        <input
          id="project-name"
          type="text"
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="IELTS Coach"
          className="mt-1.5 w-full rounded-md border border-white/10 bg-bg px-3 py-2 text-sm text-text outline-none focus:border-accent"
        />

        <p className="mt-5 text-sm text-muted">Folder</p>
        <div className="mt-1.5 flex items-center gap-2">
          <p
            className="min-w-0 flex-1 truncate rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-xs text-text"
            title={path}
          >
            {path || "No folder selected"}
          </p>
          <button
            type="button"
            onClick={() => void handleBrowse()}
            className="shrink-0 rounded-md border border-white/10 px-3 py-2 text-sm text-text transition-colors hover:bg-white/5"
          >
            Browse…
          </button>
        </div>

        {/* §5/§11: app-owned, read-only — never an input. Edit dialog only, beneath the path.
            Plan 035 step 2: gate widened from "libraries non-empty" to "framework OR libraries"
            (a framework-only project, e.g. a fresh Next app with no allow-listed deps yet, must
            not render blank), and the framework is now prefixed — once step 1's card hover shows
            the whole stack, this line is where §11 says "the full list remains", so it must be
            at least as complete. */}
        {editing && stack && (stack.framework || stack.libraries.length > 0) && (
          <p className="mt-1.5 text-xs text-muted">
            {[...(stack.framework ? [stack.framework] : []), ...stack.libraries].join(" · ")}
            <span className="text-muted/60"> · detected {relativeTime(stack.detectedAt)}</span>
          </p>
        )}

        {Object.keys(scripts).length > 0 && (
          <>
            <label className="mt-5 block text-sm text-muted" htmlFor="project-script">
              Script
            </label>
            <select
              id="project-script"
              value={selectedScript ?? ""}
              onChange={(e) => handleScriptChange(e.target.value)}
              className="mt-1.5 w-full rounded-md border border-white/10 bg-bg px-3 py-2 text-sm text-text outline-none focus:border-accent"
            >
              {Object.entries(scripts).map(([scriptName, scriptCmd]) => (
                <option key={scriptName} value={scriptName}>
                  {scriptName} — {scriptCmd}
                </option>
              ))}
            </select>
          </>
        )}

        <label className="mt-5 block text-sm text-muted" htmlFor="project-command">
          Command
        </label>
        <input
          id="project-command"
          type="text"
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          placeholder="npm run dev"
          className="mt-1.5 w-full rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-sm text-text outline-none focus:border-accent"
        />
        <p className="mt-1.5 text-xs text-muted">
          Env vars work inline — <span className="font-mono">PORT=3001 npm run dev</span>, or on
          Windows <span className="font-mono">set PORT=3001 &amp;&amp; npm run dev</span>.
        </p>

        <label className="mt-5 block text-sm text-muted" htmlFor="project-port">
          Port
        </label>
        <div className="mt-1.5 flex items-center gap-2">
          <input
            id="project-port"
            type="number"
            required
            value={port}
            onChange={(e) => setPort(e.target.value)}
            placeholder="3000"
            className="min-w-0 flex-1 rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-sm text-text outline-none focus:border-accent"
          />
          {/* §10 step 4: framework known → one button; unknown → both forms, named. Disabled with
              nothing to walk against (no folder chosen yet). */}
          {stack?.framework ? (
            <button
              type="button"
              disabled={path.trim() === "" || portPickerBusy}
              onClick={() => void handleChooseForMe("--port")}
              className="shrink-0 rounded-md border border-white/10 px-3 py-2 text-sm text-text transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Choose for me
            </button>
          ) : (
            <div className="flex shrink-0 items-center gap-1.5">
              <span className="text-sm text-muted">Choose for me:</span>
              <button
                type="button"
                disabled={path.trim() === "" || portPickerBusy}
                onClick={() => void handleChooseForMe("--port")}
                className="rounded-md border border-white/10 px-2.5 py-2 font-mono text-xs text-text transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
              >
                --port
              </button>
              <button
                type="button"
                disabled={path.trim() === "" || portPickerBusy}
                onClick={() => void handleChooseForMe("PORT=")}
                className="rounded-md border border-white/10 px-2.5 py-2 font-mono text-xs text-text transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
              >
                PORT=
              </button>
            </div>
          )}
        </div>
        {!stack?.framework && (
          <p className="mt-1.5 text-xs text-muted">
            Hangar can't tell which this project reads.{" "}
            <span className="font-mono">--port</span> suits Vite, Next, Astro, Nuxt, SvelteKit and
            Angular; <span className="font-mono">PORT=</span> suits a plain Node/Express server.
          </p>
        )}
        {portCaption && (
          <p aria-live="polite" className="mt-1.5 text-xs text-muted">
            {portCaption}
          </p>
        )}

        {/* §5: url is "shown only in the Edit dialog (placeholder shows the computed default)". */}
        {editing && (
          <>
            <label className="mt-5 block text-sm text-muted" htmlFor="project-url">
              URL override
            </label>
            <input
              id="project-url"
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder={`http://localhost:${port || editing.port}`}
              className="mt-1.5 w-full rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-sm text-text outline-none focus:border-accent"
            />
            {/* §5: non-blocking — Save stays enabled, `canSave` never sees this. */}
            {urlPortWarning && (
              <p className="mt-1.5 text-xs text-status-danger">{urlPortWarning}</p>
            )}
          </>
        )}

        <div className="mt-5 flex items-center justify-between gap-4">
          <label className="text-sm text-muted" htmlFor="project-ready-timeout">
            Ready timeout (seconds)
          </label>
          <input
            id="project-ready-timeout"
            type="number"
            min={1}
            value={readyTimeoutSec}
            onChange={(e) => setReadyTimeoutSec(Number(e.target.value))}
            className="w-24 rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-sm text-text outline-none focus:border-accent"
          />
        </div>

        <label className="mt-4 flex items-center gap-2 text-sm text-text">
          <input
            type="checkbox"
            checked={updateOnRun}
            onChange={(e) => setUpdateOnRun(e.target.checked)}
            className="size-4 rounded border-white/20 bg-bg accent-accent"
          />
          Pull updates and reinstall when this project runs
        </label>

        <label className="mt-4 flex items-center gap-2 text-sm text-text">
          <input
            type="checkbox"
            checked={openBrowserOnReady}
            onChange={(e) => setOpenBrowserOnReady(e.target.checked)}
            className="size-4 rounded border-white/20 bg-bg accent-accent"
          />
          Open browser when ready
        </label>
        <p className="mt-1.5 text-xs text-muted">
          Turn this off for a project with no page to serve, like an API-only server.
        </p>

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={closeDialog}
            className="rounded-md border border-white/10 px-4 py-2 text-sm text-text transition-colors hover:bg-white/5"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={saving || !canSave}
            title={
              !canSave
                ? "Name, folder, command, a valid port and a ready timeout of at least 1 second are required"
                : undefined
            }
            className="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
        </form>
      </div>
    </div>
  );
}

export default AddEditDialog;
