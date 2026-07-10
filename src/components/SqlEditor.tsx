import { useEffect, useRef } from "react";
import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection } from "@codemirror/view";
import { EditorState, Compartment } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { sql, SQLNamespace, StandardSQL, PostgreSQL, MySQL, SQLite } from "@codemirror/lang-sql";
import { autocompletion, completionKeymap, closeBrackets } from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { syntaxHighlighting, defaultHighlightStyle, indentOnInput, bracketMatching } from "@codemirror/language";
import { statementAt } from "../sqlsplit";
import type { DriverKind } from "../api";

interface Props {
  initialSql: string;
  driver: DriverKind | null;
  schema: SQLNamespace | undefined;
  onChange: (sql: string) => void;
  onRunStatement: (sql: string) => void;
  onRunAll: () => void;
}

const darkTheme = EditorView.theme(
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
    ".cm-activeLine": { backgroundColor: "rgba(255,255,255,0.03)" },
    ".cm-activeLineGutter": { backgroundColor: "transparent" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
      backgroundColor: "rgba(20,184,166,0.25) !important",
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
  { dark: true },
);

function sqlDialect(driver: DriverKind | null) {
  switch (driver) {
    case "postgres":
      return PostgreSQL;
    case "mysql":
      return MySQL;
    case "sqlite":
      return SQLite;
    default:
      return StandardSQL;
  }
}

export default function SqlEditor({
  initialSql,
  driver,
  schema,
  onChange,
  onRunStatement,
  onRunAll,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const langCompartment = useRef(new Compartment());
  // Keep latest callbacks without recreating the editor.
  const cbRef = useRef({ onChange, onRunStatement, onRunAll });
  cbRef.current = { onChange, onRunStatement, onRunAll };

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
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        darkTheme,
        keymap.of([
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
        langCompartment.current.of(
          sql({ dialect: sqlDialect(driver), schema, upperCaseKeywords: true }),
        ),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) cbRef.current.onChange(u.state.doc.toString());
        }),
      ],
    });

    const view = new EditorView({ state, parent: hostRef.current });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // The editor owns the document after mount; initialSql is only the seed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Reconfigure language/completion when driver or schema changes.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: langCompartment.current.reconfigure(
        sql({ dialect: sqlDialect(driver), schema, upperCaseKeywords: true }),
      ),
    });
  }, [driver, schema]);

  return <div className="editor-container" ref={hostRef} />;
}
