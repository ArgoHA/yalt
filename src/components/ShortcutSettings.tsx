import { useEffect, useRef, useState } from "react";
import {
  DEFAULT_SHORTCUTS,
  SHORTCUT_FIELDS,
  normalizeShortcut,
  shortcutError,
  shortcutLabel,
  type ShortcutMap,
} from "../editor/shortcuts";

export function ShortcutSettings({
  shortcuts,
  onSave,
  onClose,
}: {
  shortcuts: ShortcutMap;
  onSave: (shortcuts: ShortcutMap) => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<ShortcutMap>({ ...shortcuts });
  const dialogRef = useRef<HTMLDivElement>(null);
  const error = shortcutError(draft);

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialogRef.current?.querySelector<HTMLInputElement>("input")?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      previous?.focus();
    };
  }, [onClose]);

  return (
    <div className="sheet-backdrop shortcut-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="shortcut-sheet" role="dialog" aria-modal="true" aria-labelledby="shortcut-title" ref={dialogRef}>
        <header>
          <div><h2 id="shortcut-title">Keyboard shortcuts</h2><p>Enter one letter or punctuation key. Number keys and D stay reserved.</p></div>
          <button className="shortcut-close" aria-label="Close keyboard shortcut settings" onClick={onClose}>×</button>
        </header>
        <div className="shortcut-fields">
          {SHORTCUT_FIELDS.map(({ id, label }) => (
            <label key={id}>
              <span>{label}</span>
              <input
                value={shortcutLabel(draft[id])}
                maxLength={1}
                aria-invalid={Boolean(error)}
                aria-label={`${label} shortcut`}
                onChange={(event) => setDraft((current) => ({ ...current, [id]: normalizeShortcut(event.target.value) }))}
                onFocus={(event) => event.currentTarget.select()}
              />
            </label>
          ))}
        </div>
        {error && <p className="shortcut-error" role="alert">{error}</p>}
        <p className="shortcut-fixed">Always available: <kbd>←</kbd>/<kbd>→</kbd> images, <kbd>1</kbd>…<kbd>0</kbd> classes, <kbd>D D</kbd> delete image, <kbd>⌘Z</kbd> undo, <kbd>⇧⌘Z</kbd> redo, <kbd>⌘+</kbd>/<kbd>⌘−</kbd> zoom, <kbd>Space</kbd>-drag pan.</p>
        <footer>
          <button className="text-button" onClick={() => setDraft({ ...DEFAULT_SHORTCUTS })}>Restore defaults</button>
          <span />
          <button className="text-button" onClick={onClose}>Cancel</button>
          <button className="action-button" disabled={Boolean(error)} onClick={() => onSave(draft)}>Save</button>
        </footer>
      </div>
    </div>
  );
}
