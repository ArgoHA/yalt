import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ImageRecord } from "../types";

const ROW_HEIGHT = 50;
const OVERSCAN = 8;

export function VirtualImageList({
  images,
  selectedId,
  onSelect,
}: {
  images: ImageRecord[];
  selectedId: string | null;
  onSelect: (image: ImageRecord) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(400);
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const end = Math.min(images.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN);

  useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const observer = new ResizeObserver(([entry]) => setHeight(entry.contentRect.height));
    observer.observe(host);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const index = images.findIndex((image) => image.id === selectedId);
    const host = hostRef.current;
    if (index < 0 || !host) return;
    const top = index * ROW_HEIGHT;
    const bottom = top + ROW_HEIGHT;
    if (top < host.scrollTop) host.scrollTop = top;
    else if (bottom > host.scrollTop + host.clientHeight) host.scrollTop = bottom - host.clientHeight;
  }, [images, selectedId]);

  return (
    <div className="image-list virtual-list" aria-label="Project images" ref={hostRef} onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}>
      <div className="virtual-space" style={{ height: images.length * ROW_HEIGHT }}>
        {images.slice(start, end).map((image, localIndex) => {
          const index = start + localIndex;
          return (
            <button
              className={image.id === selectedId ? "image-row active" : "image-row"}
              aria-current={image.id === selectedId ? "true" : undefined}
              style={{ position: "absolute", top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
              onClick={() => onSelect(image)}
              key={image.id}
            >
              <span className="image-index">{String(index + 1).padStart(3, "0")}</span>
              <span className="image-name"><strong>{image.fileName}</strong><small>{image.width} × {image.height}</small></span>
              {image.status === "missing" && <span className="missing-dot" title="File is missing" />}
              {image.status === "deleted" && <span className="deleted-mark" title="Image is in deleted">×</span>}
            </button>
          );
        })}
      </div>
    </div>
  );
}
