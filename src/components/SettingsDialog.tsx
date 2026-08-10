/**
 * SPEC.md §11 — the gear dialog. "Editor command" (default `code`), and since plan 059 the one
 * opt-in switch Hangar has: the osv.dev dependency check the Doctor panel runs. Both are backed by
 * the §7 `get_settings` / `set_settings` commands. Nothing else — §11 is explicit.
 *
 * **The dependency label states exactly what leaves this machine**, in the place the user actually
 * reads, not only in a comment: package names and versions from `package-lock.json`, and nothing
 * else. It is off until they turn it on. Both halves of that promise are checked by tests in
 * `src-tauri/src/osv.rs`; the sentence below is what makes them a promise rather than a detail.
 *
 * Esc closes, same pattern as `LogPanel`.
 */
import { useEffect, useState } from "react";
import { getSettings } from "../api";
import { closeDialog, saveSettingsAction, useHangarStore } from "../store";

export function SettingsDialog() {
  const { dialog } = useHangarStore();
  const isOpen = dialog?.kind === "settings";
  const [editorCommand, setEditorCommand] = useState("code");
  // Mirrors the backend default (plan 059): off, so a dialog that somehow rendered before the
  // fetch resolved could never show this as on.
  const [checkDependencies, setCheckDependencies] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  // Plan 039 step 5: re-fetch on every open, not just once per app lifetime — otherwise typing a
  // wrong command, pressing Cancel, then reopening shows the abandoned in-memory value instead of
  // what's actually on disk. Same `cancelled` flag pattern as before.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    void getSettings().then((s) => {
      if (!cancelled) {
        setEditorCommand(s.editorCommand);
        setCheckDependencies(s.checkDependencies);
        setLoading(false);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [isOpen]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeDialog();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, []);

  // Guard sits after both hooks above — React forbids conditional hooks — same placement
  // as AddEditDialog's early-return-when-closed guard.
  if (!isOpen) return null;

  async function handleSave() {
    // Enter-to-save (plan 039) reaches this the same way the click path always did, but the
    // click path was only ever gated by the Save button's `disabled` attribute, not a guard in
    // here. A submit event isn't guaranteed to respect a disabled submit button in every browser,
    // so mirror that same condition explicitly — the one place both routes now share.
    if (saving || loading || editorCommand.trim() === "") return;
    setSaving(true);
    const ok = await saveSettingsAction({ editorCommand, checkDependencies });
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

        {/* Enter-to-save (plan 039): defers entirely to handleSave's own `saving`/empty guard,
            exactly like the Save button's onClick below — no duplicate validation here. */}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void handleSave();
          }}
        >
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

        {/* Plan 059. The sentence under the checkbox is the feature's contract with the user, and
            it is deliberately specific: naming osv.dev, naming package-lock.json, and listing what
            is NOT sent. "We take your privacy seriously" is what this exists instead of. */}
        <div className="mt-6 border-t border-white/5 pt-5">
          <label className="flex items-start gap-2.5" htmlFor="check-dependencies">
            <input
              id="check-dependencies"
              type="checkbox"
              disabled={loading}
              checked={checkDependencies}
              onChange={(e) => setCheckDependencies(e.target.checked)}
              className="mt-0.5 size-4 shrink-0 rounded border-white/20 bg-bg accent-accent disabled:opacity-50"
            />
            <span className="min-w-0">
              <span className="block text-sm text-text">
                Check dependencies for known vulnerabilities
              </span>
              <span className="mt-1 block text-xs leading-relaxed text-muted">
                Off unless you turn it on. Each time the Doctor panel is opened or refreshed,
                Hangar sends the package names and versions from each project&rsquo;s
                package-lock.json to osv.dev. Nothing else is sent &mdash; no file paths, no
                project names, no machine identifier. Dependencies installed from git, a local
                path or a link are never sent; a package from a private registry cannot be told
                apart from a public one, so its name is. npm projects only; pnpm and yarn
                lockfiles are not read.
              </span>
            </span>
          </label>
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
            type="submit"
            disabled={saving || loading || editorCommand.trim() === ""}
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

export default SettingsDialog;
