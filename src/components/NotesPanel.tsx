/**
 * SPEC.md §11 — the notes slide-over: one free-text scratchpad per project, autosaved on blur
 * and after a pause in typing, Esc closes. Modelled on `LogPanel.tsx`'s shell.
 *
 * Notes are user-owned and never read anywhere else in the app (SPEC.md §5) — this is the only
 * component that touches the field, and it never parses or acts on the text.
 */
import { useEffect, useRef, useState } from "react";
import { closeNotes, saveNotesAction, useHangarStore } from "../store";

/** §11 says "autosaved" — no Save button. Debounce after typing stops, plus save-on-blur. */
const SAVE_DEBOUNCE_MS = 800;

export function NotesPanel() {
  const { notesFor, projects } = useHangarStore();
  const project = projects.find((p) => p.id === notesFor) ?? null;

  const [value, setValue] = useState("");
  const [dirty, setDirty] = useState(false);
  const [justSaved, setJustSaved] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Mirrors `value`/`dirty` for flushPendingSave() (below), which the Esc handler also calls.
  // That handler lives in an effect keyed only on [notesFor], so a closure over `value`/`dirty`
  // directly would go stale between re-renders — it would keep whatever those were when
  // [notesFor] last changed, not what was last typed. Refs are dereferenced at call time, not
  // closure-creation time, so keeping these in lockstep with every setValue/setDirty call means
  // flushPendingSave() always reads the latest keystroke no matter how old its own closure is.
  const valueRef = useRef("");
  const dirtyRef = useRef(false);

  // Seeds the textarea only when a (possibly different) project's panel opens. Deliberately NOT
  // keyed on `project` itself: a save triggers `loadRegistry()`, which would otherwise re-run
  // this effect mid-typing and clobber keystrokes made during that round trip with the slightly
  // stale value the save just wrote.
  useEffect(() => {
    valueRef.current = project?.notes ?? "";
    dirtyRef.current = false;
    setValue(project?.notes ?? "");
    setDirty(false);
    setJustSaved(false);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [notesFor]);

  // §11: Esc closes the slide-over — the only keyboard shortcut in v0. Must flush first: Esc
  // fires no `blur`, so without this an edit made in the last 800ms of debounce is silently
  // discarded (plan 039). flushPendingSave() (below) reads valueRef/dirtyRef rather than
  // `value`/`dirty` directly, so this closure cannot go stale even though the effect only
  // re-runs when `notesFor` changes.
  useEffect(() => {
    if (!notesFor) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        flushPendingSave();
        closeNotes();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [notesFor]);

  // Cleans up a pending debounce on unmount — the panel unmounts only with the app.
  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  function save(text: string): void {
    if (!project) return;
    // Pass only the id: `saveNotesAction` reads the current project fresh from the store, so a
    // stale `project` closure here (e.g. from before some other field changed elsewhere while
    // this panel sat open) can never leak a non-notes change into the saved payload.
    void saveNotesAction(project.id, text).then(() => {
      dirtyRef.current = false;
      setDirty(false);
      setJustSaved(true);
    });
  }

  function handleChange(text: string): void {
    valueRef.current = text;
    dirtyRef.current = true;
    setValue(text);
    setDirty(true);
    setJustSaved(false);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => save(text), SAVE_DEBOUNCE_MS);
  }

  // Extracted so both the blur route and the Esc route (above) flush the same way: clear the
  // pending debounce, then save if there's unsaved text. Reads the refs, not `value`/`dirty`
  // directly, for the reason explained where they're declared.
  function flushPendingSave(): void {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (dirtyRef.current) save(valueRef.current);
  }

  function handleBlur(): void {
    flushPendingSave();
  }

  if (!notesFor || !project) return null;

  return (
    <div className="fixed inset-0 z-20 flex justify-end" role="presentation">
      {/* Click-away closes, same as Esc. §11 enter transition: backdrop fades in. */}
      <div
        className="hangar-fade-in flex-1 bg-black/40"
        onClick={closeNotes}
        aria-hidden="true"
      />

      <aside
        role="dialog"
        aria-modal="true"
        aria-label={`Notes for ${project.name}`}
        className="hangar-slide-in flex h-full w-[min(32rem,92vw)] flex-col border-l border-white/10 bg-surface shadow-2xl"
      >
        <header className="flex items-center justify-between gap-3 border-b border-white/5 px-5 py-4">
          <div className="min-w-0">
            <h2 className="truncate font-display text-base font-medium text-text">
              {project.name}
            </h2>
            {/* §11: "a small, quiet saved indicator" — reserves its line with a non-breaking
                space so the header does not jump when the text appears/disappears. */}
            <p className="mt-0.5 text-xs text-muted" aria-live="polite">
              {justSaved && !dirty ? "Saved" : " "}
            </p>
          </div>
          <button
            type="button"
            aria-label="Close notes"
            onClick={closeNotes}
            className="shrink-0 rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            <span aria-hidden="true">✕</span>
          </button>
        </header>

        <textarea
          value={value}
          onChange={(e) => handleChange(e.target.value)}
          onBlur={handleBlur}
          placeholder="Notes for this project — a scratchpad the app never reads."
          className="flex-1 resize-none bg-bg px-5 py-4 text-sm text-text outline-none placeholder:text-muted/60"
        />
      </aside>
    </div>
  );
}

export default NotesPanel;
