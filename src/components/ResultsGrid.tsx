import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, CellValue, StatementOutcome } from "../api";

const ROW_H = 26;
const HEADER_H = 28;
const PAGE = 500;
/** Rendered rows beyond the viewport on each side. */
const OVERSCAN = 30;
/** Exact mode: render window moves in steps of this many rows. */
const WINDOW_STEP = 10;
/**
 * WebKit clamps element heights in the tens of millions of pixels; past this
 * budget we switch to a scaled scroll range (like DataGrip): the scrollbar
 * maps proportionally onto the row range and the rendered block is anchored
 * at the viewport.
 */
const H_MAX = 8_000_000;

/** Flick-compounding glide: successive flicks while scrolling ADD velocity
 * (native macOS resets momentum per flick — Sublime-style accumulation needs
 * this assist). Native scrolling stays untouched for small deltas. */
const FLICK_DELTA = 45; // px per wheel event that counts as a flick
const FLICK_BOOST = 9; // velocity gained per flick-delta px (px/s)
const GLIDE_MAX = 90_000; // px/s cap
const GLIDE_TAU = 0.55; // s, exponential decay
const GLIDE_MIN = 60; // px/s, stop threshold

interface Props {
  outcome: StatementOutcome;
}

export default function ResultsGrid({ outcome }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const pages = useRef(new Map<number, CellValue[][]>());
  const inflight = useRef(new Set<number>());
  const fetchTimer = useRef<number | undefined>(undefined);
  const rafPending = useRef(false);
  const lastScrollTs = useRef(0);
  const glide = useRef({ v: 0, raf: 0, lastTs: 0 });
  const [, setVersion] = useState(0);
  const [windowStart, setWindowStart] = useState(0);
  const [anchorPx, setAnchorPx] = useState(0);
  const [viewH, setViewH] = useState(400);
  const [sel, setSel] = useState<{ r: number; c: number } | null>(null);

  const columns = outcome.columns ?? [];
  const rowCount = outcome.rowCount;
  const resultId = outcome.resultId;

  const naturalH = rowCount * ROW_H;
  const scaled = naturalH > H_MAX;
  const virtualH = scaled ? H_MAX : naturalH;
  const visibleRows = Math.ceil(viewH / ROW_H);
  const blockRows = Math.min(rowCount, visibleRows + OVERSCAN * 2);

  // Column widths (px), resizable.
  const [widths, setWidths] = useState<number[]>([]);
  useEffect(() => {
    setWidths(columns.map((c) => Math.min(320, Math.max(90, c.name.length * 9 + 40))));
    pages.current.clear();
    inflight.current.clear();
    glide.current.v = 0;
    setSel(null);
    setWindowStart(0);
    setAnchorPx(0);
    if (containerRef.current) containerRef.current.scrollTop = 0;
    setVersion((v) => v + 1);
  }, [resultId]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewH(el.clientHeight));
    ro.observe(el);
    setViewH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  /** First row for a given scrollTop under the active mapping. */
  const rowForScroll = useCallback(
    (st: number) => {
      if (!scaled) return Math.floor(st / ROW_H);
      const maxScroll = Math.max(1, virtualH - viewH);
      const frac = Math.min(1, Math.max(0, st / maxScroll));
      return Math.floor(frac * Math.max(0, rowCount - visibleRows));
    },
    [scaled, virtualH, viewH, rowCount, visibleRows],
  );

  const firstRow = windowStart;
  const lastRow = Math.min(rowCount - 1, windowStart + blockRows - 1);

  const fetchPages = useCallback(
    (from: number, to: number) => {
      if (resultId == null) return;
      const p0 = Math.floor(from / PAGE);
      const p1 = Math.floor(to / PAGE);
      for (let p = p0; p <= p1; p++) {
        if (pages.current.has(p) || inflight.current.has(p)) continue;
        inflight.current.add(p);
        api
          .pageFetch(resultId, p * PAGE, PAGE)
          .then((res) => {
            pages.current.set(p, res.rows);
            if (pages.current.size > 80) {
              for (const k of pages.current.keys()) {
                if (k < p0 - 20 || k > p1 + 20) pages.current.delete(k);
                if (pages.current.size <= 60) break;
              }
            }
            setVersion((v) => v + 1);
          })
          .catch(() => {
            /* result evicted; leave gap */
          })
          .finally(() => inflight.current.delete(p));
      }
    },
    [resultId],
  );

  const scheduleFetch = useCallback(
    (from: number, to: number, immediate: boolean) => {
      window.clearTimeout(fetchTimer.current);
      if (immediate) {
        fetchPages(from, to);
      } else {
        fetchTimer.current = window.setTimeout(() => fetchPages(from, to), 50);
      }
    },
    [fetchPages],
  );

  useEffect(() => {
    if (rowCount > 0) scheduleFetch(firstRow, lastRow, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resultId, rowCount, viewH]);

  const onScroll = useCallback(() => {
    lastScrollTs.current = performance.now();
    if (rafPending.current) return;
    rafPending.current = true;
    requestAnimationFrame(() => {
      rafPending.current = false;
      const el = containerRef.current;
      if (!el) return;
      const st = el.scrollTop;
      const targetRow = Math.max(0, rowForScroll(st) - OVERSCAN);
      const quantized = scaled
        ? targetRow // scaled mode: every ROW_H of anchor movement re-renders anyway
        : Math.floor(targetRow / WINDOW_STEP) * WINDOW_STEP;
      // Anchor the rendered block at the viewport (quantized to row height so
      // we re-render at most once per 26px of travel). Clamp with the ACTUAL
      // number of rows left to render — at the tail the block is shorter than
      // blockRows and a blockRows-based clamp would park it above the viewport.
      const renderedRows = Math.min(rowCount - 1, quantized + blockRows - 1) - quantized + 1;
      const maxAnchor = Math.max(0, virtualH - renderedRows * ROW_H);
      const nextAnchor = scaled
        ? Math.min(maxAnchor, Math.max(0, Math.floor(st / ROW_H) * ROW_H - OVERSCAN * ROW_H))
        : quantized * ROW_H;

      setWindowStart((prev) => {
        setAnchorPx((prevA) => (nextAnchor === prevA ? prevA : nextAnchor));
        if (quantized === prev) return prev;
        const to = Math.min(rowCount - 1, quantized + blockRows - 1);
        const fast = Math.abs(quantized - prev) > PAGE;
        scheduleFetch(quantized, to, !fast);
        return quantized;
      });
    });
  }, [rowCount, blockRows, scaled, virtualH, rowForScroll, scheduleFetch]);

  // ---- Flick-compounding glide assist (Sublime-style momentum stacking) ----
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const stopGlide = () => {
      glide.current.v = 0;
    };

    const glideStep = (ts: number) => {
      const g = glide.current;
      if (g.v === 0) return;
      const dt = Math.min(0.05, (ts - g.lastTs) / 1000);
      g.lastTs = ts;
      const before = el.scrollTop;
      el.scrollTop = before + g.v * dt;
      // Hit an edge: kill the glide.
      if (el.scrollTop === before && dt > 0) {
        g.v = 0;
        return;
      }
      g.v *= Math.exp(-dt / GLIDE_TAU);
      if (Math.abs(g.v) < GLIDE_MIN) {
        g.v = 0;
        return;
      }
      g.raf = requestAnimationFrame(glideStep);
    };

    const onWheel = (e: WheelEvent) => {
      const d = e.deltaY;
      const g = glide.current;
      if (Math.abs(d) < FLICK_DELTA) return; // gentle scrolling: fully native
      if (g.v !== 0 && Math.sign(d) !== Math.sign(g.v)) {
        stopGlide(); // reversing direction cancels accumulated speed
        return;
      }
      // Compound only when motion is already in flight (2nd+ flick); the
      // first flick is handled natively.
      const inMotion = performance.now() - lastScrollTs.current < 140 || g.v !== 0;
      if (!inMotion) return;
      const wasIdle = g.v === 0;
      g.v = Math.max(-GLIDE_MAX, Math.min(GLIDE_MAX, g.v + d * FLICK_BOOST));
      if (wasIdle && g.v !== 0) {
        g.lastTs = performance.now();
        g.raf = requestAnimationFrame(glideStep);
      }
    };

    const onDown = () => stopGlide();
    el.addEventListener("wheel", onWheel, { passive: true });
    el.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onDown);
    return () => {
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onDown);
      cancelAnimationFrame(glide.current.raf);
      stopGlide();
    };
  }, []);

  const getRow = (r: number): CellValue[] | null => {
    const page = pages.current.get(Math.floor(r / PAGE));
    return page ? (page[r % PAGE] ?? null) : null;
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "c" && sel) {
      const row = getRow(sel.r);
      if (row) {
        const v = row[sel.c];
        void navigator.clipboard.writeText(v == null ? "" : String(v));
      }
      e.preventDefault();
    }
  };

  const startResize = (idx: number, e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startW = widths[idx] ?? 120;
    const move = (ev: MouseEvent) => {
      setWidths((w) => {
        const next = [...w];
        next[idx] = Math.max(50, startW + ev.clientX - startX);
        return next;
      });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const visible = useMemo(() => {
    const rows: { idx: number; cells: CellValue[] | null }[] = [];
    for (let r = firstRow; r <= lastRow; r++) rows.push({ idx: r, cells: getRow(r) });
    return rows;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [firstRow, lastRow, rowCount, resultId, pages.current.size, sel, widths]);

  if (rowCount === 0) {
    return (
      <div className="placeholder" style={{ minHeight: 80 }}>
        <div>0 rows</div>
      </div>
    );
  }

  const rownumW = Math.max(46, String(rowCount).length * 9 + 20);
  const topPad = anchorPx;
  const bottomPad = Math.max(0, virtualH - topPad - visible.length * ROW_H);

  return (
    <div
      className="grid"
      ref={containerRef}
      onScroll={onScroll}
      tabIndex={0}
      onKeyDown={onKeyDown}
    >
      <table style={{ width: rownumW + widths.reduce((a, b) => a + b, 0) }}>
        <colgroup>
          <col style={{ width: rownumW }} />
          {columns.map((_, i) => (
            <col key={i} style={{ width: widths[i] ?? 120 }} />
          ))}
        </colgroup>
        <thead>
          <tr style={{ height: HEADER_H }}>
            <th className="rownum">#</th>
            {columns.map((c, i) => (
              <th key={i} style={{ position: "sticky", top: 0 }}>
                <span style={{ position: "relative", display: "block" }}>
                  {c.name}
                  {c.typeName && <span className="coltype">{c.typeName}</span>}
                  <span className="resize-handle" onMouseDown={(e) => startResize(i, e)} />
                </span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {topPad > 0 && (
            <tr style={{ height: topPad }}>
              <td colSpan={columns.length + 1} style={{ border: "none", padding: 0 }} />
            </tr>
          )}
          {visible.map(({ idx, cells }) => (
            <tr key={idx} style={{ height: ROW_H }}>
              <td className="rownum">{(idx + 1).toLocaleString()}</td>
              {columns.map((_, c) => {
                const isSel = sel?.r === idx && sel?.c === c;
                const v = cells ? cells[c] : undefined;
                return (
                  <td
                    key={c}
                    className={isSel ? "selected" : undefined}
                    onClick={() => setSel({ r: idx, c })}
                    title={v == null ? undefined : String(v)}
                  >
                    {cells === null ? (
                      <span className="null">…</span>
                    ) : v === null ? (
                      <span className="null">NULL</span>
                    ) : (
                      String(v)
                    )}
                  </td>
                );
              })}
            </tr>
          ))}
          {bottomPad > 0 && (
            <tr style={{ height: bottomPad }}>
              <td colSpan={columns.length + 1} style={{ border: "none", padding: 0 }} />
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
