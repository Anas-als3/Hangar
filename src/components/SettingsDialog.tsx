/**
 * SPEC.md §11 — the gear dialog with exactly one field, "Editor command" (default `code`),
 * backed by the §7 `get_settings` / `set_settings` commands. Nothing else — §11 is explicit.
 *
 * Esc closes, same pattern as `LogPanel`.
 */
import { useEffect, useState } from "react";
import { getSettings } from "../api";
import { closeDialog, saveSettingsAction, useHangarStore } from "../store";

export function SettingsDialog() {
  const { dialog } = useHangarStore();
  const [editorCommand, setEditorCommand] = useState("code");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getSettings().then((s) => {
      if (!cancelled) {
        setEditorCommand(s.editorCommand);
        setLoading(false);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeDialog();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, []);

  // Guard sits after both hooks above — React forbids conditional hooks — same placement
  // as AddEditDialog's early-return-when-closed guard.
  if (dialog?.kind !== "settings") return null;

  async function handleSave() {
    setSaving(true);
    const ok = await saveSettingsAction({ editorCommand });
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
        aria-label="Settings"
        className="hangar-dialog-in relative z-10 w-[min(28rem,92vw)] rounded-lg border border-white/10 bg-surface p-6 shadow-2xl"
      >
        <h2 className="font-display text-lg font-medium text-text">Settings</h2>

        <label className="mt-5 block text-sm text-muted" htmlFor="editor-command">
          Editor command
        </label>
        <input
          id="editor-command"
          type="text"
          autoFocus
          disabled={loading}
          value={editorCommand}
          onChange={(e) => setEditorCommand(e.target.value)}
          placeholder="code"
          className="mt-1.5 w-full rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-sm text-text outline-none focus:border-accent disabled:opacity-50"
        />

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
            disabled={saving || loading || editorCommand.trim() === ""}
            onClick={() => void handleSave()}
            className="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default SettingsDialog;
