# Progress log

## Session 2026-07-11 (fresh context)
- Reviewed codebase; merged the new request items (keychain, connection list
  drag-reorder, delete popover, sidebar min-width/border fixes, JDBC catalog
  expansion, tree streamlining, live endpoint testing) into plan.md.
- Starting chunk 1: tab drag restore/animation + Open Console modal.

### Landed (commits)
1. Tab drag fix + FLIP; Open Console modal (always opens, saved consoles listed).
2. Sidebar polish: title collision fix (min width 240), rail border fix,
   connection drag-reorder (profiles_reorder cmd), delete confirm popover.
3. Reformatter: sql-formatter + Edit menu items (⌘⌥L), Formatter Settings modal,
   persistent config.json via config_save/config_load.
4. Find/replace panel (iI, .*, close button); autocomplete from query history
   (statements + words), Enter/Tab accept.
5. Lint: node-sql-parser DML validation + doubled-keyword squiggles; spellcheck
   with entropy/schema/keyword skips; heavy deps lazy-loaded (entry 944K).

Next: chunk 6 (native engines: mssql/tiberius, redshift, mariadb, clickhouse).
