export const SHORTCUT_STORAGE_KEY = "yalt.shortcuts.v1";
const LEGACY_SHORTCUT_STORAGE_KEY = "labeler.shortcuts.v1";

export type CustomShortcutId =
  | "fit"
  | "edit"
  | "rectangle"
  | "polygon"
  | "island"
  | "previousClass"
  | "nextClass";

export type ShortcutMap = Record<CustomShortcutId, string>;

export const DEFAULT_SHORTCUTS: ShortcutMap = {
  fit: "f",
  edit: "e",
  rectangle: "r",
  polygon: "p",
  island: "i",
  previousClass: "[",
  nextClass: "]",
};

export const SHORTCUT_FIELDS: ReadonlyArray<{ id: CustomShortcutId; label: string }> = [
  { id: "fit", label: "Fit image" },
  { id: "edit", label: "Edit objects" },
  { id: "rectangle", label: "Draw box" },
  { id: "polygon", label: "Draw polygon" },
  { id: "island", label: "Add polygon island" },
  { id: "previousClass", label: "Previous class" },
  { id: "nextClass", label: "Next class" },
];

export function loadShortcuts(storage: Pick<Storage, "getItem"> = localStorage): ShortcutMap {
  try {
    const stored = storage.getItem(SHORTCUT_STORAGE_KEY) ?? storage.getItem(LEGACY_SHORTCUT_STORAGE_KEY);
    const parsed = JSON.parse(stored ?? "null") as Partial<ShortcutMap> | null;
    if (!parsed) return { ...DEFAULT_SHORTCUTS };
    const candidate = { ...DEFAULT_SHORTCUTS, ...parsed };
    return shortcutError(candidate) ? { ...DEFAULT_SHORTCUTS } : candidate;
  } catch {
    return { ...DEFAULT_SHORTCUTS };
  }
}

export function saveShortcuts(shortcuts: ShortcutMap, storage: Pick<Storage, "setItem"> = localStorage): void {
  const error = shortcutError(shortcuts);
  if (error) throw new Error(error);
  storage.setItem(SHORTCUT_STORAGE_KEY, JSON.stringify(shortcuts));
}

export function shortcutError(shortcuts: ShortcutMap): string | null {
  const values = Object.values(shortcuts).map(normalizeShortcut);
  if (values.some((value) => !isAllowedShortcut(value))) {
    return "Use one letter or punctuation key for every custom shortcut.";
  }
  if (new Set(values).size !== values.length) {
    return "Each custom shortcut must use a different key.";
  }
  if (values.some((value) => /^[0-9]$/.test(value))) {
    return "Number keys are reserved for classes.";
  }
  if (values.includes("d")) {
    return "D is reserved for the D D image-delete sequence.";
  }
  return null;
}

export function normalizeShortcut(value: string): string {
  return value.trim().toLowerCase();
}

export function shortcutLabel(value: string): string {
  return normalizeShortcut(value).toUpperCase();
}

export function matchesShortcut(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey">, value: string): boolean {
  return !event.metaKey && !event.ctrlKey && !event.altKey
    && normalizeShortcut(event.key) === normalizeShortcut(value);
}

function isAllowedShortcut(value: string): boolean {
  return value.length === 1 && !/\s/.test(value);
}
