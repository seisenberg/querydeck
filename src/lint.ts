// SQL validation + identifier/comment spellcheck for the editor.
//
// Validation: DML statements are parsed with node-sql-parser in the
// connection's dialect; parse failures get a red squiggle at the reported
// spot (e.g. the second SELECT in "SELECT SELECT"). DDL and vendor-specific
// statements are deliberately NOT parse-checked — node-sql-parser's grammar
// coverage there is partial and false red squiggles are worse than none.
// A doubled-keyword heuristic still covers every statement.
//
// Spellcheck: prose-ish words inside comments, strings and identifiers get a
// dotted underline when they're not dictionary words. High-entropy tokens,
// digit-bearing tokens, acronyms, SQL vocabulary and schema names are all
// skipped so hashes and ids don't light up.

import { Diagnostic, linter } from "@codemirror/lint";
import { syntaxTree } from "@codemirror/language";
import { splitStatements } from "./sqlsplit";

// The parser grammars and the dictionary are heavy (megabytes); load them
// lazily on the first lint pass so they never slow app startup.
type SqlParser = { astify: (sql: string, opt: { database: string }) => unknown };
let parserPromise: Promise<SqlParser> | null = null;
function loadParser(): Promise<SqlParser> {
  parserPromise ??= import("node-sql-parser").then((m) => new m.Parser() as SqlParser);
  return parserPromise;
}

/** node-sql-parser dialect ids per driver kind (missing = don't parse). */
const PARSER_DB: Record<string, string> = {
  postgres: "PostgresQL",
  redshift: "Redshift",
  mysql: "MySQL",
  mariadb: "MariaDB",
  sqlite: "Sqlite",
  mssql: "TransactSQL",
  snowflake: "Snowflake",
  trino: "Trino",
};

const DML = /^\s*(select|insert|update|delete|with|values)\b/i;

const DOUBLED =
  /\b(select\s+select|from\s+from|where\s+where|and\s+and|or\s+or|join\s+join|on\s+on|not\s+not|as\s+as|in\s+in)\b/gi;

let dictPromise: Promise<Set<string>> | null = null;
function loadDictionary(): Promise<Set<string>> {
  dictPromise ??= import("an-array-of-english-words").then((m) => new Set(m.default));
  return dictPromise;
}

/** SQL vocabulary that isn't English but must never be "misspelled". */
const SQL_WORDS = new Set(
  (
    "varchar nvarchar char text int integer bigint smallint tinyint mediumint bool boolean " +
    "numeric decimal float double serial bigserial uuid json jsonb blob clob bytea datetime " +
    "timestamp timestamptz timetz interval ilike coalesce nullif ifnull isnull concat substr " +
    "substring strftime regexp rlike autoincrement unsigned zerofill rowid ctid xmin oid " +
    "pragma tablespace unnest jsonb_agg string_agg array_agg group_concat len nvl decode trunc " +
    "sysdate getdate dateadd datediff datepart charindex nchar ntext varbinary rownum dual " +
    "limit offset asc desc distinct having exists cascade restrict grant revoke func proc " +
    "tmp temp cfg ctx idx pkey fkey seq trg usr pwd src dst dest util db dbo sess"
  ).split(" "),
);

/** True for tokens spellcheck should never touch: ids, hashes, hex, base64…
 * (high-entropy or clearly non-prose). */
function looksRandom(w: string): boolean {
  if (w.length >= 24) return true;
  if (/[bcdfghjklmnpqrstvwxz]{5,}/i.test(w)) return true; // long consonant run
  const vowels = (w.match(/[aeiouy]/gi) ?? []).length;
  if (vowels === 0 || vowels / w.length < 0.2) return true;
  return false;
}

/** Shorten node-sql-parser's "Expected X, Y, Z, … but W found" messages. */
function compactParseError(msg: string): string {
  const m = /but (.+) found/.exec(msg);
  if (m) return `Syntax error: unexpected ${m[1]}`;
  const firstLine = msg.split("\n")[0];
  return firstLine.length > 160 ? firstLine.slice(0, 160) + "…" : firstLine;
}

export interface LintCtx {
  driver: string | null;
  /** Lowercased schema/table/column names (and their word parts). */
  schemaWords: Set<string>;
  /** Master switches (spellcheck can be turned off in config later). */
  validate?: boolean;
  spellcheck?: boolean;
}

const MAX_DOC = 100_000;
const MAX_SPELL = 50;

export function sqlLinter(getCtx: () => LintCtx) {
  return linter(
    async (view) => {
      const ctx = getCtx();
      const doc = view.state.doc.toString();
      if (doc.length > MAX_DOC) return [];
      const [parser, dictionary] = await Promise.all([loadParser(), loadDictionary()]);
      const diagnostics: Diagnostic[] = [];
      const tree = syntaxTree(view.state);

      const inStringOrComment = (pos: number) => {
        const name = tree.resolveInner(pos, 1).name;
        return /Comment|String/i.test(name);
      };

      // 1. Doubled-keyword heuristic first — it pinpoints the repeated word,
      //    which beats a whole-statement underline from the parser.
      if (ctx.validate !== false) {
        for (const m of doc.matchAll(DOUBLED)) {
          const at = m.index ?? 0;
          if (inStringOrComment(at)) continue;
          // Underline the repeated (second) word.
          const secondWord = m[1].split(/\s+/)[1] ?? m[1];
          const wordFrom = at + m[1].lastIndexOf(secondWord);
          const wordTo = wordFrom + secondWord.length;
          if (diagnostics.some((d) => d.from < wordTo && wordFrom < d.to)) continue;
          diagnostics.push({
            from: wordFrom,
            to: wordTo,
            severity: "error",
            message: `Repeated keyword "${secondWord.toUpperCase()}"`,
          });
        }
      }

      // 2. Dialect parse of DML statements. Syntax errors carry a location;
      //    semantic errors don't and underline the whole statement.
      if (ctx.validate !== false) {
        const db = ctx.driver ? PARSER_DB[ctx.driver] : undefined;
        if (db) {
          for (const stmt of splitStatements(doc)) {
            if (!DML.test(stmt.sql)) continue;
            try {
              parser.astify(stmt.sql, { database: db });
            } catch (e) {
              const err = e as {
                message?: string;
                location?: { start?: { offset?: number }; end?: { offset?: number } };
              };
              const hasLoc = err.location?.start?.offset != null;
              const from = hasLoc
                ? stmt.from + (err.location?.start?.offset ?? 0)
                : stmt.from;
              const to = hasLoc
                ? Math.min(stmt.to, Math.max(from + 1, stmt.from + (err.location?.end?.offset ?? 0)))
                : stmt.to;
              if (diagnostics.some((d) => d.from < to && from < d.to)) continue;
              diagnostics.push({
                from,
                to,
                severity: "error",
                message: compactParseError(err.message ?? "syntax error"),
              });
            }
          }
        }
      }

      // 3. Spellcheck over comments, strings and identifiers.
      if (ctx.spellcheck !== false) {
        let spellCount = 0;
        tree.iterate({
          enter: (node) => {
            if (spellCount >= MAX_SPELL) return false;
            const name = node.name;
            const isText = /Comment|String/i.test(name);
            const isIdent = /Identifier/i.test(name) && !/Quoted/i.test(name);
            if (!isText && !isIdent) return;
            const text = doc.slice(node.from, node.to);
            // Hump words: lowercase runs with an optional leading capital.
            for (const m of text.matchAll(/[A-Za-z]?[a-z']+/g)) {
              const raw = m[0].replace(/'/g, "");
              if (raw.length < 4) continue;
              const lower = raw.toLowerCase();
              if (SQL_WORDS.has(lower) || ctx.schemaWords.has(lower)) continue;
              if (looksRandom(raw)) continue;
              if (dictionary.has(lower)) continue;
              const from = node.from + (m.index ?? 0);
              diagnostics.push({
                from,
                to: from + m[0].length,
                severity: "info",
                markClass: "qd-spell",
                message: `Possible typo: "${raw}"`,
              });
              if (++spellCount >= MAX_SPELL) break;
            }
            return;
          },
        });
      }

      return diagnostics;
    },
    { delay: 600 },
  );
}
