/**
 * "Move to folder…" — SPEC.md §11: the required non-drag route both into AND out of a folder,
 * because §11 permits no keyboard shortcut that could substitute for the drag gesture (plan 030).
 *
 * Modelled on `AddEditDialog.tsx`'s shell — backdrop, `hangar-dialog-in`, focus handling, Esc via
 * its own `document` listener gated on `isOpen` — per plan 029: do not invent a second dialog
 * idiom.
 */
import { useEffect, useState } from "react";
import { closeDialog, moveToFolder, setToast, useHangarStore } from "../store";
import type { FolderTarget } from "../store";
import type { ProjectView } from "../types";

interface FolderOption {
  id: string;
  name: string;
  count: number;
}

/** Every distinct folder currently in the registry, first-seen (array) order — same tiebreak as
 *  `gridItems`: the earliest member's `folderName` is the one that's shown. */
function existingFolders(projects: ProjectView[]): FolderOption[] {
  const folders: FolderOption[] = [];
  const byId = new Map<string, FolderOption>();
  for (const p of projects) {
    if (!p.folderId) continue;
    const existing = byId.get(p.folderId);
    if (existing) {
      existing.count += 1;
      continue;
    }
    const created: FolderOption = { id: p.folderId, name: p.folderName ?? "", count: 1 };
    byId.set(p.folderId, created);
    folders.push(created);
  }
  return folders;
}

export function MoveToFolderDialog() {
  const { dialog, projects } = useHangarStore();
  const project = dialog?.kind === "move-folder" ? dialog.project : null;
  const isOpen = dialog?.kind === "move-folder";

  const [selection, setSelection] = useState<string>("none");
  const [newFolderName, setNewFolderName] = useState("");
  const [saving, setSaving] = useState(false);

  // Re-initializes whenever the dialog target changes — same shape as AddEditDialog's own reset
  // effect. Selects the project's current folder by default; "none" when it has none.
  useEffect(() => {
    setSelection(project?.folderId ?? "none");
    setNewFolderName("");
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

  if (!isOpen || !project) return null;

  const folders = existingFolders(projects);
  const canSave = selection !== "new" || newFolderName.trim() !== "";

  async function handleSave() {
    // `project` is narrowed above, but that narrowing doesn't carry into this nested closure —
    // re-check it here so TS (and a stray call after unmount) can't hit a null dereference.
    if (!canSave || !project) return;
    setSaving(true);
    // Captured before the write, purely for the confirmation toast's copy — the write itself
    // reads the project fresh inside `moveToFolder` (see store.ts), never from this snapshot.
    const projectName = project.name;
    const priorFolderName = project.folderName;
    const target: FolderTarget =
      selection === "none"
        ? { kind: "none" }
        : selection === "new"
          ? { kind: "new", name: newFolderName.trim() }
          : {
              kind: "existing",
              folderId: selection,
              folderName: folders.find((f) => f.id === selection)?.name ?? "",
            };
    const ok = await moveToFolder(project.id, target);
    if (!ok) {
      setSaving(false);
      return;
    }
    const destName =
      target.kind === "new" ? target.name : target.kind === "existing" ? target.folderName : null;
    const message =
      destName !== null
        ? `Moved ${projectName} to ${destName}`
        : priorFolderName
          ? `Moved ${projectName} out of ${priorFolderName}`
          : `${projectName} is not in a folder`;
    // §11: this is a confirmation, not an error — the neutral toast tone (added by this plan).
    setToast(message, "neutral");
    closeDialog();
  }

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center" role="presentation">
      <div
        className="hangar-fade-in absolute inset-0 bg-black/40"
        onClick={closeDialog}
        aria-hidden="true"
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Move ${project.name} to folder`}
        className="hangar-dialog-in relative z-10 max-h-[90vh] w-[min(24rem,92vw)] overflow-y-auto rounded-lg border border-white/10 bg-surface p-6 shadow-2xl"
      >
        <h2 className="font-display text-lg font-medium text-text">Move to folder</h2>

        <div role="radiogroup" aria-label="Folder" className="mt-4 flex flex-col gap-1">
          {/* §11/plan 029: always present, never conditional — the only non-mouse route out. */}
          <label className="flex items-center gap-2 rounded-md px-2 py-2 text-sm text-text hover:bg-white/5">
            <input
              type="radio"
              name="move-folder-target"
              checked={selection === "none"}
              onChange={() => setSelection("none")}
              className="accent-accent"
            />
            Not in a folder
          </label>

          {folders.map((folder) => (
            <label
              key={folder.id}
              className="flex items-center gap-2 rounded-md px-2 py-2 text-sm text-text hover:bg-white/5"
            >
              <input
                type="radio"
                name="move-folder-target"
                checked={selection === folder.id}
                onChange={() => setSelection(folder.id)}
                className="accent-accent"
              />
              <span className="min-w-0 flex-1 truncate">{folder.name || "Untitled folder"}</span>
              <span className="shrink-0 text-xs text-muted">{folder.count}</span>
            </label>
          ))}

          <label className="flex items-center gap-2 rounded-md px-2 py-2 text-sm text-text hover:bg-white/5">
            <input
              type="radio"
              name="move-folder-target"
              checked={selection === "new"}
              onChange={() => setSelection("new")}
              className="accent-accent"
            />
            New folder…
          </label>
          {selection === "new" && (
            <input
              type="text"
              autoFocus
              value={newFolderName}
              onChange={(e) => setNewFolderName(e.target.value)}
              placeholder="Folder name"
              className="ml-6 rounded-md border border-white/10 bg-bg px-3 py-2 text-sm text-text outline-none focus:border-accent"
            />
          )}
        </div>

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
            disabled={saving || !canSave}
            onClick={() => void handleSave()}
            title={!canSave ? "Name the new folder first" : undefined}
            className="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {saving ? "Moving…" : "Move"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default MoveToFolderDialog;

