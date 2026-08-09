/**
 * SPEC.md §10 — Add/Edit dialog: folder picker → package.json scripts → port suggestion →
 * editable command → optional url override (edit only) → save.
 *
 * Duplicate-port / not-stopped rejections are enforced by Rust (§7); this dialog just calls
 * addProjectAction/updateProjectAction and lets `setToast` surface whatever comes back.
 *
 * Built in passes (plan 005): this pass is the shell — state, Esc-close, Name/Path fields only.
 * The folder picker, script list, port/url/hint fields and save wiring land in the next passes.
 */
import { useEffect, useState } from "react";
// Native folder picker (§10 step 1) — dialog plugin only, never tauri-plugin-shell (§4).
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { readPackageJson } from "../api";
import { closeDialog, useHangarStore } from "../store";
import type { PackageJsonInfo } from "../types";

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

export function AddEditDialog() {
  const { dialog } = useHangarStore();
  const editing = dialog?.kind === "edit" ? dialog.project : null;
  const isOpen = dialog?.kind === "add" || dialog?.kind === "edit";

  const [name, setName] = useState(editing?.name ?? "");
  const [path, setPath] = useState(editing?.path ?? "");
  const [command, setCommand] = useState(editing?.command ?? "");
  const [port, setPort] = useState(editing ? String(editing.port) : "");
  const [saving, setSaving] = useState(false);
  const [scripts, setScripts] = useState<Record<string, string>>({});
  const [selectedScript, setSelectedScript] = useState<string | null>(null);
  const [packageManager, setPackageManager] =
    useState<PackageJsonInfo["packageManager"]>("npm");

  // Re-initialize whenever the dialog target changes (opened for a different project, or
  // switched from Edit back to Add).
  useEffect(() => {
    setName(editing?.name ?? "");
    setPath(editing?.path ?? "");
    setCommand(editing?.command ?? "");
    setPort(editing ? String(editing.port) : "");
    setSaving(false);
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
    try {
      const info = await readPackageJson(selected);
      setScripts(info.scripts);
      setPackageManager(info.packageManager);
      const defaultScript = pickDefaultScript(info.scripts);
      setSelectedScript(defaultScript);
      if (defaultScript) setCommand(commandFor(info.packageManager, defaultScript));
      if (info.portSuggestion !== undefined) setPort(String(info.portSuggestion));
    } catch {
      // §10 step 6: no/unparseable package.json falls back to manual command + port entry.
      setScripts({});
      setSelectedScript(null);
    }
  }

  function handleScriptChange(script: string) {
    setSelectedScript(script);
    setCommand(commandFor(packageManager, script));
  }

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center" role="presentation">
      <div className="absolute inset-0 bg-black/40" onClick={closeDialog} aria-hidden="true" />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={editing ? `Edit ${editing.name}` : "Add project"}
        className="relative z-10 max-h-[90vh] w-[min(32rem,92vw)] overflow-y-auto rounded-lg border border-white/10 bg-surface p-6 shadow-2xl"
      >
        <h2 className="font-display text-lg font-medium text-text">
          {editing ? "Edit project" : "Add project"}
        </h2>

        <label className="mt-5 block text-sm text-muted" htmlFor="project-name">
          Name
        </label>
        <input
          id="project-name"
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="IELTS Coach"
          className="mt-1.5 w-full rounded-md border border-white/10 bg-bg px-3 py-2 text-sm text-text outline-none focus:border-accent"
        />

        <p className="mt-5 text-sm text-muted">Folder</p>
        <p className="mt-1.5 truncate rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-xs text-text">
          {path || "No folder selected"}
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
            type="button"
            disabled={saving}
            className="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default AddEditDialog;
