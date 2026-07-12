import { useEffect, useRef } from "react";
import {
  EditorView,
  Panel,
  keymap,
  lineNumbers,
  highlightActiveLine,
  drawSelection,
} from "@codemirror/view";
import { EditorState, Compartment } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  sql,
  SQLNamespace,
  StandardSQL,
  PostgreSQL,
  MySQL,
  MariaSQL,
  MSSQL,
  SQLite,
} from "@codemirror/lang-sql";
import {
  autocompletion,
  completionKeymap,
  closeBrackets,
  acceptCompletion,
  Completion,
  CompletionContext,
  CompletionResult,
} from "@codemirror/autocomplete";
import {
  search,
  searchKeymap,
  highlightSelectionMatches,
  SearchQuery,
  setSearchQuery,
  getSearchQuery,
  findNext,
  findPrevious,
  replaceNext,
  replaceAll,
  closeSearchPanel,
} from "@codemirror/search";
import {
  syntaxHighlighting,
  defaultHighlightStyle,
  HighlightStyle,
  indentOnInput,
  bracketMatching,
} from "@codemirror/language";
import { tags } from "@lezer/highlight";
import { statementAt } from "../sqlsplit";
import { sqlLinter } from "../lint";
import type { DriverKind } from "../api";

/** Find/replace panel: plain text or regex (.*), case toggle (iI), close
 * button at the end — replaces CodeMirror's stock checkbox panel. */
function createSearchPanel(view: EditorView): Panel {
  const dom = document.createElement("div");
  dom.className = "qd-search";

  const initial = getSearchQuery(view.state);
  let caseSensitive = initial.caseSensitive;
  let regexp = initial.regexp;

  const findField = document.createElement("input");
  findField.placeholder = "Find";
  findField.value = initial.search;
  findField.setAttribute("main-field", "true");
  const replaceField = document.createElement("input");
  replaceField.placeholder = "Replace";
  replaceField.value = initial.replace;

  const commit = () => {
    const query = new SearchQuery({
      search: findField.value,
      replace: replaceField.value,
      caseSensitive,
      regexp,
    });
    if (!query.eq(getSearchQuery(view.state))) {
      view.dispatch({ effects: setSearchQuery.of(query) });
    }
  };
  findField.oninput = commit;
  replaceField.oninput = commit;

  const button = (label: string, title: string, cls: string, onClick: () => void) => {
    const b = document.createElement("button");
    b.textContent = label;
    b.title = title;
    b.className = cls;
    b.onclick = onClick;
    return b;
  };

  const caseBtn = button("iI", "Ignore case", "qd-toggle", () => {
    caseSensitive = !caseSensitive;
    caseBtn.classList.toggle("active", !caseSensitive);
    caseBtn.title = caseSensitive ? "Match case" : "Ignore case";
    commit();
  });
  caseBtn.classList.toggle("active", !caseSensitive);
  const regexBtn = button(".*", "Regular expression", "qd-toggle", () => {
    regexp = !regexp;
    regexBtn.classList.toggle("active", regexp);
    commit();
  });
  regexBtn.classList.toggle("active", regexp);

  dom.onkeydown = (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      closeSearchPanel(view);
      view.focus();
    } else if (e.key === "Enter" && e.target === findField) {
      e.preventDefault();
      if (e.shiftKey) findPrevious(view);
      else findNext(view);
    } else if (e.key === "Enter" && e.target === replaceField) {
      e.preventDefault();
      replaceNext(view);
    }
  };

  dom.append(
    findField,
    caseBtn,
    regexBtn,
    button("↑", "Previous match (⇧⏎)", "", () => void findPrevious(view)),
    button("↓", "Next match (⏎)", "", () => void findNext(view)),
    replaceField,
    button("Replace", "Replace current match", "", () => void replaceNext(view)),
    button("All", "Replace all matches", "", () => void replaceAll(view)),
  );
  const spacer = document.createElement("span");
  spacer.className = "qd-search-spacer";
  dom.append(
    spacer,
    button("✕", "Close search (Esc)", "qd-search-close", () => {
      closeSearchPanel(view);
      view.focus();
    }),
  );

  return {
    dom,
    top: true,
    mount() {
      findField.focus();
      findField.select();
    },
  };
}

interface Props {
  initialSql: string;
  /** Bumped by the store when it rewrites the SQL (e.g. Reformat Query);
   * makes the editor reload `initialSql` even though it owns the doc. */
  sqlEpoch?: number;
  driver: DriverKind | null;
  schema: SQLNamespace | undefined;
  /** Extra completions mined from this connection's query history. */
  historyCompletions?: Completion[];
  /** Editor pane height as a fraction of the console split. */
  heightPct?: number;
  onChange: (sql: string) => void;
  onRunStatement: (sql: string) => void;
  onRunAll: () => void;
}

/** Editor chrome. Colors come from the CSS theme tokens, so one spec serves
 * both schemes — only CodeMirror's internal `dark` flag needs to differ. */
function makeChrome(dark: boolean) {
  return EditorView.theme(
    {
      "&": { backgroundColor: "var(--bg)", color: "var(--text)" },
      ".cm-content": { caretColor: "var(--accent)" },
      ".cm-cursor": { borderLeftColor: "var(--accent)" },
      ".cm-gutters": {
        backgroundColor: "var(--bg-panel)",
        color: "var(--text-dim)",
        border: "none",
        borderRight: "1px solid var(--border)",
      },
      ".cm-activeLine": { backgroundColor: "var(--active-line)" },
      ".cm-activeLineGutter": { backgroundColor: "transparent" },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
        backgroundColor: "var(--sel) !important",
      },
      ".cm-tooltip": {
        backgroundColor: "var(--bg-elevated)",
        border: "1px solid var(--border)",
        color: "var(--text)",
      },
      ".cm-tooltip-autocomplete ul li[aria-selected]": {
        backgroundColor: "var(--accent-dim)",
      },
    },
    { dark },
  );
}

/** Syntax colors tuned for the dark palette (defaultHighlightStyle is
 * designed for light backgrounds and reads muddy on dark). */
const darkHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: "#c678dd" },
  { tag: [tags.string, tags.special(tags.string)], color: "#98c379" },
  { tag: [tags.number, tags.bool, tags.null], color: "#d19a66" },
  { tag: tags.comment, color: "#7f848e", fontStyle: "italic" },
  { tag: [tags.typeName, tags.className], color: "#e5c07b" },
  { tag: [tags.operator, tags.compareOperator], color: "#56b6c2" },
  { tag: tags.function(tags.variableName), color: "#61afef" },
  { tag: tags.propertyName, color: "#e06c75" },
]);

const themeFor = (dark: boolean) => [
  makeChrome(dark),
  syntaxHighlighting(dark ? darkHighlight : defaultHighlightStyle, { fallback: true }),
];

/** All schema/table/column names (and their word parts), lowercased — the
 * spellchecker must never flag them. */
const schemaWordsCache = new WeakMap<object, Set<string>>();
function schemaWordsOf(schema: SQLNamespace | undefined): Set<string> {
  if (!schema || typeof schema !== "object") return EMPTY_WORDS;
  const cached = schemaWordsCache.get(schema);
  if (cached) return cached;
  const out = new Set<string>();
  const add = (name: string) => {
    out.add(name.toLowerCase());
    for (const part of name.split(/[_\d]+/)) {
      if (part) out.add(part.toLowerCase());
      for (const hump of part.matchAll(/[A-Z]?[a-z]+/g)) out.add(hump[0].toLowerCase());
    }
  };
  const walk = (ns: SQLNamespace) => {
    if (Array.isArray(ns)) {
      for (const item of ns) {
        if (typeof item === "string") add(item);
        else if (item && typeof item === "object" && "label" in item) add(String(item.label));
      }
      return;
    }
    if (typeof ns === "object" && ns) {
      for (const [k, v] of Object.entries(ns)) {
        if (k === "self" || k === "children") continue;
        add(k);
        walk(v as SQLNamespace);
      }
    }
  };
  walk(schema);
  schemaWordsCache.set(schema, out);
  return out;
}
const EMPTY_WORDS = new Set<string>();

function sqlDialect(driver: DriverKind | null) {
  switch (driver) {
    case "postgres":
    case "redshift":
    case "duckdb":
      return PostgreSQL;
    case "mysql":
    case "clickhouse":
      return MySQL;
    case "mariadb":
      return MariaSQL;
    case "mssql":
      return MSSQL;
    case "sqlite":
      return SQLite;
    default:
      return StandardSQL;
  }
}

export default function SqlEditor({
  initialSql,
  sqlEpoch,
  driver,
  schema,
  historyCompletions,
  heightPct,
  onChange,
  onRunStatement,
  onRunAll,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const langCompartment = useRef(new Compartment());
  const themeCompartment = useRef(new Compartment());
  // Keep latest callbacks without recreating the editor.
  const cbRef = useRef({ onChange, onRunStatement, onRunAll, historyCompletions });
  cbRef.current = { onChange, onRunStatement, onRunAll, historyCompletions };

  // Lint context read through a ref so the linter never needs rebuilding.
  const lintCtxRef = useRef({ driver: driver as string | null, schemaWords: new Set<string>() });
  lintCtxRef.current = { driver, schemaWords: schemaWordsOf(schema) };

  // Completions mined from the connection's query history; a stable source
  // reading through cbRef so the editor never needs reconfiguring for it.
  const historySource = useRef((ctx: CompletionContext): CompletionResult | null => {
    const options = cbRef.current.historyCompletions ?? [];
    if (!options.length) return null;
    const word = ctx.matchBefore(/[\w.]+$/);
    if (!word || (word.from === word.to && !ctx.explicit)) return null;
    return { from: word.from, options, validFor: /^[\w.]*$/ };
  }).current;

  const langExtensions = (drv: DriverKind | null, ns: SQLNamespace | undefined) => {
    const dialect = sqlDialect(drv);
    return [
      sql({ dialect, schema: ns, upperCaseKeywords: true }),
      dialect.language.data.of({ autocomplete: historySource }),
    ];
  };

  useEffect(() => {
    if (!hostRef.current) return;

    const runAtCursor = (view: EditorView) => {
      const { from, to } = view.state.selection.main;
      if (from !== to) {
        cbRef.current.onRunStatement(view.state.sliceDoc(from, to));
        return true;
      }
      const stmt = statementAt(view.state.doc.toString(), from);
      if (stmt) cbRef.current.onRunStatement(stmt.sql);
      return true;
    };

    const state = EditorState.create({
      doc: initialSql,
      extensions: [
        lineNumbers(),
        history(),
        drawSelection(),
        indentOnInput(),
        bracketMatching(),
        closeBrackets(),
        autocompletion(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        search({ createPanel: createSearchPanel, top: true }),
        sqlLinter(() => lintCtxRef.current),
        themeCompartment.current.of(
          themeFor(window.matchMedia("(prefers-color-scheme: dark)").matches),
        ),
        keymap.of([
          // Both must run before the default Enter/Tab bindings; they return
          // false when no completion is active, so typing is unaffected.
          { key: "Enter", run: acceptCompletion },
          { key: "Tab", run: acceptCompletion },
          {
            key: "Mod-Enter",
            run: runAtCursor,
          },
          {
            key: "Mod-Shift-Enter",
            run: () => {
              cbRef.current.onRunAll();
              return true;
            },
          },
          indentWithTab,
          ...defaultKeymap,
          ...historyKeymap,
          ...searchKeymap,
          ...completionKeymap,
        ]),
        langCompartment.current.of(langExtensions(driver, schema)),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) cbRef.current.onChange(u.state.doc.toString());
        }),
      ],
    });

    const view = new EditorView({ state, parent: hostRef.current });
    viewRef.current = view;

    // Follow the OS appearance live — no restart needed on theme change.
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onScheme = () =>
      view.dispatch({ effects: themeCompartment.current.reconfigure(themeFor(mq.matches)) });
    mq.addEventListener("change", onScheme);

    return () => {
      mq.removeEventListener("change", onScheme);
      view.destroy();
      viewRef.current = null;
    };
    // The editor owns the document after mount; initialSql is only the seed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Store-driven rewrite (Reformat Query): replace the doc, keep undo intact.
  const lastEpoch = useRef(sqlEpoch);
  useEffect(() => {
    if (sqlEpoch === lastEpoch.current) return;
    lastEpoch.current = sqlEpoch;
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === initialSql) return;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: initialSql },
    });
  }, [sqlEpoch, initialSql]);

  // Reconfigure language/completion when driver or schema changes.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: langCompartment.current.reconfigure(langExtensions(driver, schema)),
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [driver, schema]);

  return (
    <div
      className="editor-container"
      ref={hostRef}
      style={heightPct != null ? { height: `${heightPct * 100}%` } : undefined}
    />
  );
}
