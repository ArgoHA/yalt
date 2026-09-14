import { readImageFile } from "../backend";
import type { ImageRecord } from "../types";

interface CacheEntry {
  url: string;
  touchedAt: number;
}

export class ImageUrlCache {
  private readonly entries = new Map<string, CacheEntry>();
  private readonly pending = new Map<string, Promise<string>>();

  constructor(private readonly capacity = 3) {}

  async load(rootPath: string, image: ImageRecord): Promise<string> {
    const cached = this.entries.get(image.id);
    if (cached) {
      cached.touchedAt = performance.now();
      return cached.url;
    }
    const inFlight = this.pending.get(image.id);
    if (inFlight) return inFlight;

    const request = readImageFile(rootPath, image.id).then((raw) => {
      const bytes = raw instanceof ArrayBuffer ? raw : new Uint8Array(raw).buffer;
      const blob = new Blob([bytes], { type: mimeType(image.fileName) });
      const url = URL.createObjectURL(blob);
      this.entries.set(image.id, { url, touchedAt: performance.now() });
      this.trim();
      return url;
    }).finally(() => this.pending.delete(image.id));
    this.pending.set(image.id, request);
    return request;
  }

  prefetch(rootPath: string, images: ImageRecord[]): void {
    for (const image of images) void this.load(rootPath, image).catch(() => undefined);
  }

  clear(): void {
    for (const entry of this.entries.values()) URL.revokeObjectURL(entry.url);
    this.entries.clear();
    this.pending.clear();
  }

  private trim(): void {
    if (this.entries.size <= this.capacity) return;
    const oldest = [...this.entries.entries()].sort((left, right) => left[1].touchedAt - right[1].touchedAt)[0];
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
