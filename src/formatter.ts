import type { FormatOptionsWithLanguage } from "sql-formatter";

// sql-formatter (~180 kB) is only needed when the user reformats a query or
// opens the formatter-settings preview — never during startup or rendering.
// Load it on demand so it stays out of the initial bundle and off the
// cold-load path.
let modPromise: Promise<typeof import("sql-formatter")> | null = null;

export async function formatSql(
  sql: string,
  opts: FormatOptionsWithLanguage,
): Promise<string> {
  modPromise ??= import("sql-formatter");
  const { format } = await modPromise;
  return format(sql, opts);
}
