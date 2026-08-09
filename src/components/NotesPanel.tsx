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

  // Seeds the textarea only when a (possibly different) project's panel opens. Deliberately NOT
  // keyed on `project` itself: a save triggers `loadRegistry()`, which would otherwise re-run
  // this effect mid-typing and clobber keystrokes made during that round trip with the slightly
  // stale value the save just wrote.
  useEffect(() => {
    setValue(project?.notes ?? "");
    setDirty(false);
    setJustSaved(false);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [notesFor]);

  // §11: Esc closes the slide-over — the only keyboard shortcut in v0.
  useEffect(() => {
    if (!notesFor) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeNotes();
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
    void saveNotesAction(project, text).then(() => {
      setDirty(false);
      setJustSaved(true);
    });
  }

  function handleChange(text: string): void {
    setValue(text);
    setDirty(true);
    setJustSaved(false);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => save(text), SAVE_DEBOUNCE_MS);
  }

  function handleBlur(): void {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (dirty) save(value);
  }

  if (!notesFor || !project) return null;

  return null;
}

export default NotesPanel;
