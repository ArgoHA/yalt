import { readImageFile } from "../backend";
import type { ImageRecord } from "../types";

interface CacheEntry {
  url: string;
  touchedAt: number;
}

export class ImageUrlCache {
  private readonly entries = new Map<string, CacheEntry>();
  private readonly pending = new Map<string, Promise<string>>();
  private activeKey: string | null = null;
  private generation = 0;

  constructor(private readonly capacity = 3) {}

  async load(rootPath: string, image: ImageRecord): Promise<string> {
    this.activeKey = this.key(rootPath, image);
    return this.loadImage(rootPath, image);
  }

  private key(rootPath: string, image: ImageRecord): string {
    return JSON.stringify([rootPath, image.id]);
  }

  private async loadImage(rootPath: string, image: ImageRecord): Promise<string> {
    const key = this.key(rootPath, image);
    const cached = this.entries.get(key);
    if (cached) {
      cached.touchedAt = performance.now();
      return cached.url;
    }
    const inFlight = this.pending.get(key);
    if (inFlight) return inFlight;

    const generation = this.generation;
    const request = readImageFile(rootPath, image.id).then((raw) => {
      if (generation !== this.generation) throw new Error("Image load was cancelled because the cache was cleared.");
      const bytes = raw instanceof ArrayBuffer ? raw : new Uint8Array(raw).buffer;
      const blob = new Blob([bytes], { type: mimeType(image.fileName) });
      const url = URL.createObjectURL(blob);
      this.entries.set(key, { url, touchedAt: performance.now() });
      this.trim();
      return url;
    }).finally(() => {
      if (this.pending.get(key) === request) this.pending.delete(key);
    });
    this.pending.set(key, request);
    return request;
  }

  prefetch(rootPath: string, images: ImageRecord[]): void {
    for (const image of images) void this.loadImage(rootPath, image).catch(() => undefined);
  }

  clear(): void {
    this.generation += 1;
    this.activeKey = null;
    for (const entry of this.entries.values()) URL.revokeObjectURL(entry.url);
    this.entries.clear();
    this.pending.clear();
  }

  private trim(): void {
    if (this.entries.size <= this.capacity) return;
    // Slow prefetches from a previous image can complete after navigation.
    // Never revoke the URL currently being decoded or displayed by the editor.
    const oldest = [...this.entries.entries()]
      .filter(([key]) => key !== this.activeKey)
      .sort((left, right) => left[1].touchedAt - right[1].touchedAt)[0];
    if (oldest) {
      URL.revokeObjectURL(oldest[1].url);
      this.entries.delete(oldest[0]);
    }
  }
}

function mimeType(fileName: string): string {
  const extension = fileName.split(".").at(-1)?.toLowerCase();
  if (extension === "png") return "image/png";
  if (extension === "webp") return "image/webp";
  // TIFF files are decoded and converted to PNG by the native backend so the
  // WebView receives a format it renders consistently across supported macOS versions.
  if (extension === "tif" || extension === "tiff") return "image/png";
  return "image/jpeg";
}
