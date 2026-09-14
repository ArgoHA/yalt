import { describe, expect, it } from "vitest";
import {
  DEFAULT_SHORTCUTS,
  loadShortcuts,
  matchesShortcut,
  saveShortcuts,
  shortcutError,
  type ShortcutMap,
} from "./shortcuts";

describe("custom shortcuts", () => {
  it("rejects duplicate, class-number, and image-delete bindings", () => {
    expect(shortcutError({ ...DEFAULT_SHORTCUTS, polygon: "r" })).toContain("different");
    expect(shortcutError({ ...DEFAULT_SHORTCUTS, fit: "1" })).toContain("reserved");
    expect(shortcutError({ ...DEFAULT_SHORTCUTS, fit: "d" })).toContain("image-delete");
  });

  it("falls back when persisted settings are malformed", () => {
    expect(loadShortcuts({ getItem: () => "not json" })).toEqual(DEFAULT_SHORTCUTS);
    expect(loadShortcuts({ getItem: () => JSON.stringify({ fit: "1" }) })).toEqual(DEFAULT_SHORTCUTS);
  });

  it("loads shortcuts saved by the pre-release app", () => {
    expect(loadShortcuts({
      getItem: (key) => key === "labeler.shortcuts.v1" ? JSON.stringify({ ...DEFAULT_SHORTCUTS, fit: "g" }) : null,
    }).fit).toBe("g");
  });

  it("saves valid settings and matches keys without command modifiers", () => {
    let stored = "";
    const shortcuts: ShortcutMap = { ...DEFAULT_SHORTCUTS, fit: "g" };
    saveShortcuts(shortcuts, { setItem: (_key, value) => { stored = value; } });
    expect(JSON.parse(stored).fit).toBe("g");
    expect(matchesShortcut({ key: "G", metaKey: false, ctrlKey: false, altKey: false }, "g")).toBe(true);
    expect(matchesShortcut({ key: "g", metaKey: true, ctrlKey: false, altKey: false }, "g")).toBe(false);
  });
});
