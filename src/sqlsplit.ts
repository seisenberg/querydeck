// Statement boundary detection for run-at-cursor. Mirrors the Rust splitter
// in src-tauri/src/sqlsplit.rs (comments, strings, dollar-quoting, brackets).

export interface StatementSpan {
  sql: string;
  from: number;
  to: number;
}

export function splitStatements(script: string): StatementSpan[] {
  const out: StatementSpan[] = [];
  const n = script.length;
  let start = 0;
  let i = 0;

  const push = (endExclusive: number) => {
    const raw = script.slice(start, endExclusive);
    const sql = raw.trim();
    if (sql) {
      const lead = raw.length - raw.trimStart().length;
      out.push({ sql, from: start + lead, to: start + lead + sql.length });
    }
  };

  while (i < n) {
    const c = script[i];
    if (c === "-" && script[i + 1] === "-") {
      while (i < n && script[i] !== "\n") i++;
    } else if (c === "/" && script[i + 1] === "*") {
      let depth = 1;
      i += 2;
      while (i < n && depth > 0) {
        if (script[i] === "/" && script[i + 1] === "*") {
          depth++;
          i += 2;
        } else if (script[i] === "*" && script[i + 1] === "/") {
          depth--;
          i += 2;
        } else i++;
      }
    } else if (c === "'") {
      i++;
      while (i < n) {
        if (script[i] === "\\") i += 2;
        else if (script[i] === "'") {
          if (script[i + 1] === "'") i += 2;
          else {
            i++;
            break;
          }
        } else i++;
      }
    } else if (c === '"' || c === "`") {
      const q = c;
      i++;
      while (i < n) {
        if (script[i] === q) {
          if (script[i + 1] === q) i += 2;
          else {
            i++;
            break;
          }
        } else i++;
      }
    } else if (c === "[") {
      i++;
      while (i < n && script[i] !== "]") i++;
      i++;
    } else if (c === "$") {
      let j = i + 1;
      while (j < n && /[A-Za-z0-9_]/.test(script[j])) j++;
      if (script[j] === "$") {
        const tag = script.slice(i, j + 1);
        const end = script.indexOf(tag, j + 1);
        i = end === -1 ? n : end + tag.length;
      } else i++;
    } else if (c === ";") {
      push(i);
      i++;
      start = i;
    } else {
      i++;
    }
  }
  push(n);
  return out;
}

/** The statement whose span contains `pos` (or the nearest one before it). */
export function statementAt(script: string, pos: number): StatementSpan | null {
  const spans = splitStatements(script);
  if (spans.length === 0) return null;
  for (const s of spans) {
    if (pos <= s.to) return s;
  }
  return spans[spans.length - 1];
}
