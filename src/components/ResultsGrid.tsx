import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, CellValue, StatementOutcome } from "../api";

const ROW_H = 26;
const HEADER_H = 28;
const PAGE = 500;
const OVERSCAN = 12;

interface Props {
  outcome: StatementOutcome;
}

/**
 * Windowed grid over a backend-stored result set. Only visible rows are
 * materialized; pages of rows are fetched on demand and cached.
 */
export default function ResultsGrid({ outcome }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const pages = useRef(new Map<number, CellValue[][]>());
  const inflight = useRef(new Set<number>());
  const [, setVersion] = useState(0);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewH, setViewH] = useState(400);
  const [sel, setSel] = useState<{ r: number; c: number } | null>(null);

  const columns = outcome.columns ?? [];
  const rowCount = outcome.rowCount;
  const resultId = outcome.resultId;

  // Column widths (px), resizable.
  const [widths, setWidths] = useState<number[]>([]);
  useEffect(() => {
    setWidths(columns.map((c) => Math.min(320, Math.max(90, c.name.length * 9 + 40))));
    pages.current.clear();
    inflight.current.clear();
    setSel(null);
    setScrollTop(0);
    if (containerRef.current) containerRef.current.scrollTop = 0;
    setVersion((v) => v + 1);
  }, [resultId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Observe viewport height.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewH(el.clientHeight));
    ro.observe(el);
    setViewH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  const firstRow = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const lastRow = Math.min(rowCount - 1, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN);

  const ensurePages = useCallback(
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
            // rudimentary cache cap: keep ~40k rows in the webview
            if (pages.current.size > 80) {
              const keys = [...pages.current.keys()];
              for (const k of keys) {
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

  useEffect(() => {
    if (rowCount > 0) ensurePages(firstRow, lastRow);
  }, [firstRow, lastRow, rowCount, ensurePages]);

  const getRow = (r: number): CellValue[] | null => {
    const page = pages.current.get(Math.floor(r / PAGE));
    return page ? (page[r % PAGE] ?? null) : null;
  };

  // Copy selected cell.
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
  const topPad = firstRow * ROW_H;
  const bottomPad = Math.max(0, (rowCount - 1 - lastRow) * ROW_H);

  return (
    <div
      className="grid"
      ref={containerRef}
      onScroll={(e) => setScrollTop((e.target as HTMLDivElement).scrollTop)}
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
