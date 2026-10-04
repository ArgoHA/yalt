import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readImageFile } from "../backend";
import type { ImageRecord } from "../types";
import { ImageUrlCache } from "./imageCache";

vi.mock("../backend", () => ({ readImageFile: vi.fn() }));

function image(id: string): ImageRecord {
  return { id, fileName: `${id}.jpg`, relativePath: `${id}.jpg`, width: 100, height: 100, status: "active", orientation: 1 };
}

function deferred() {
  let resolve!: (value: number[]) => void;
  const promise = new Promise<number[]>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("image URL cache", () => {
  beforeEach(() => {
    vi.mocked(readImageFile).mockReset().mockResolvedValue([1, 2, 3]);
    let serial = 0;
    vi.spyOn(URL, "createObjectURL").mockImplementation(() => `blob:test-${++serial}`);
    vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  });
  afterEach(() => vi.restoreAllMocks());

  it("keeps the active image alive when old prefetches finish after navigation", async () => {
    const late = deferred();
    vi.mocked(readImageFile).mockImplementation((_root, id) => id === "old-neighbor" ? late.promise : Promise.resolve([1]));
    const cache = new ImageUrlCache(3);
    cache.prefetch("/project", [image("old-neighbor")]);
    const activeUrl = await cache.load("/project", image("current"));
    cache.prefetch("/project", [image("previous"), image("next")]);
    await vi.waitFor(() => expect(URL.createObjectURL).toHaveBeenCalledTimes(3));
    late.resolve([2]);
    await vi.waitFor(() => expect(URL.createObjectURL).toHaveBeenCalledTimes(4));
    expect(URL.revokeObjectURL).not.toHaveBeenCalledWith(activeUrl);
    expect(await cache.load("/project", image("current"))).toBe(activeUrl);
    cache.clear();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith(activeUrl);
  });

  it("does not recreate URLs after the cache is cleared during a read", async () => {
    const read = deferred();
    vi.mocked(readImageFile).mockReturnValue(read.promise);
    const cache = new ImageUrlCache();
    const load = cache.load("/project", image("current"));
    const rejection = expect(load).rejects.toThrow("cache was cleared");
    cache.clear();
    read.resolve([1]);
    await rejection;
    expect(URL.createObjectURL).not.toHaveBeenCalled();
  });

  it("keeps a new pending request when a read from before clear finishes", async () => {
    const oldRead = deferred();
    const newRead = deferred();
    vi.mocked(readImageFile).mockReturnValueOnce(oldRead.promise).mockReturnValueOnce(newRead.promise);
    const cache = new ImageUrlCache();
    const oldLoad = cache.load("/project", image("current"));
    const rejection = expect(oldLoad).rejects.toThrow();
    cache.clear();
    const newLoad = cache.load("/project", image("current"));
    oldRead.resolve([1]);
    await rejection;
    const duplicate = cache.load("/project", image("current"));
    expect(readImageFile).toHaveBeenCalledTimes(2);
    newRead.resolve([2]);
    expect(await duplicate).toBe(await newLoad);
    cache.clear();
  });

  it("does not reuse images with the same ID from another project", async () => {
    const cache = new ImageUrlCache();
    const first = await cache.load("/first", image("same-id"));
    const second = await cache.load("/second", image("same-id"));
    expect(second).not.toBe(first);
    expect(readImageFile).toHaveBeenCalledTimes(2);
    cache.clear();
  });
});
