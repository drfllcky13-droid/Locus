// A stain photo on a canvas: wheel to zoom about the cursor, drag to pan, click to place a
// point (reported in image pixels: x right, y down, pixel (i, j) spans [i, i + 1)). Draws
// the alignment pairs, the scale's corners, the edge points, the automatic edge's seed and the
// marked tail.
import { useEffect, useRef, useState } from "react";

type P2 = [number, number];

export interface PhotoMarks {
  pairs: { px: P2; done: boolean }[];
  /** The scale's corners for the perspective correction, in click order. */
  corners: P2[];
  edges: P2[];
  seed: P2 | null;
  tail: P2 | null;
}

export function PhotoEditor({
  image,
  marks,
  onClick,
}: {
  image: HTMLImageElement;
  marks: PhotoMarks;
  onClick: (px: P2) => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [view, setView] = useState<{ s: number; ox: number; oy: number } | null>(null);
  const drag = useRef<{ x: number; y: number; ox: number; oy: number; moved: boolean } | null>(
    null,
  );

  // Fit the photo on first show and when the canvas resizes.
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const fit = () => {
      c.width = c.clientWidth;
      c.height = c.clientHeight;
      const s = Math.min(c.width / image.naturalWidth, c.height / image.naturalHeight);
      setView({
        s,
        ox: (c.width - image.naturalWidth * s) / 2,
        oy: (c.height - image.naturalHeight * s) / 2,
      });
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(c);
    return () => ro.disconnect();
  }, [image]);

  useEffect(() => {
    const c = canvas.current;
    const ctx = c?.getContext("2d");
    if (!c || !ctx || !view) return;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = "#1b1b1b";
    ctx.fillRect(0, 0, c.width, c.height);
    ctx.imageSmoothingEnabled = view.s < 2;
    ctx.setTransform(view.s, 0, 0, view.s, view.ox, view.oy);
    ctx.drawImage(image, 0, 0);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    const at = (p: P2): P2 => [p[0] * view.s + view.ox, p[1] * view.s + view.oy];
    ctx.fillStyle = "#ff3b30";
    for (const e of marks.edges) {
      const [x, y] = at(e);
      ctx.fillRect(x - 1, y - 1, 2, 2);
    }
    ctx.font = "12px sans-serif";
    marks.pairs.forEach((p, i) => {
      const [x, y] = at(p.px);
      ctx.strokeStyle = "#3fa9ff";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(x, y, 6, 0, Math.PI * 2);
      ctx.stroke();
      if (p.done) {
        ctx.fillStyle = "#3fa9ff";
        ctx.beginPath();
        ctx.arc(x, y, 2, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.fillStyle = "#3fa9ff";
      ctx.fillText(String(i + 1), x + 8, y - 8);
    });
    if (marks.corners.length > 0) {
      ctx.strokeStyle = "#bf5af2";
      ctx.fillStyle = "#bf5af2";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      marks.corners.forEach((p, i) => {
        const [x, y] = at(p);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      if (marks.corners.length === 4) ctx.closePath();
      ctx.stroke();
      marks.corners.forEach((p, i) => {
        const [x, y] = at(p);
        ctx.fillRect(x - 3, y - 3, 6, 6);
        ctx.fillText(`C${i + 1}`, x + 6, y + 14);
      });
    }
    const cross = (p: P2, colour: string, label: string) => {
      const [x, y] = at(p);
      ctx.strokeStyle = colour;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(x - 7, y);
      ctx.lineTo(x + 7, y);
      ctx.moveTo(x, y - 7);
      ctx.lineTo(x, y + 7);
      ctx.stroke();
      ctx.fillStyle = colour;
      ctx.fillText(label, x + 8, y + 14);
    };
    if (marks.seed) cross(marks.seed, "#ffd60a", "seed");
    if (marks.tail) cross(marks.tail, "#30d158", "tail");
  }, [image, marks, view]);

  const toImage = (e: React.PointerEvent | React.WheelEvent): P2 | null => {
    const c = canvas.current;
    if (!c || !view) return null;
    const r = c.getBoundingClientRect();
    return [(e.clientX - r.left - view.ox) / view.s, (e.clientY - r.top - view.oy) / view.s];
  };

  return (
    <canvas
      ref={canvas}
      className="photo-editor"
      onWheel={(e) => {
        const p = toImage(e);
        if (!p || !view) return;
        const s = Math.min(64, Math.max(0.02, view.s * Math.pow(1.15, -e.deltaY / 100)));
        const r = canvas.current!.getBoundingClientRect();
        const [cx, cy] = [e.clientX - r.left, e.clientY - r.top];
        setView({ s, ox: cx - p[0] * s, oy: cy - p[1] * s });
      }}
      onPointerDown={(e) => {
        if (!view) return;
        (e.target as Element).setPointerCapture(e.pointerId);
        drag.current = { x: e.clientX, y: e.clientY, ox: view.ox, oy: view.oy, moved: false };
      }}
      onPointerMove={(e) => {
        const d = drag.current;
        if (!d || !view) return;
        const [dx, dy] = [e.clientX - d.x, e.clientY - d.y];
        if (!d.moved && Math.hypot(dx, dy) < 4) return;
        d.moved = true;
        setView({ ...view, ox: d.ox + dx, oy: d.oy + dy });
      }}
      onPointerUp={(e) => {
        const d = drag.current;
        drag.current = null;
        if (!d || d.moved || e.button !== 0) return;
        const p = toImage(e);
        if (p) onClick(p);
      }}
    />
  );
}
