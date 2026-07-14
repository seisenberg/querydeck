import type { DriverPackage } from "./api";

/**
 * Staleness detection for JDBC driver jars in the connection editor.
 *
 * A jar path is matched back to a catalog package purely by filename shape:
 * the catalog filename is `<prefix><version><suffix>` (e.g. "postgresql-" +
 * "42.7.13" + ".jar"), so any jar sharing the prefix and suffix with a plain
 * version in between is treated as an install of that package. Renamed or
 * unrecognized jars simply don't match — no flag, no button — because using
 * an old driver deliberately (old servers need them) must stay first-class.
 */
export interface JarMatch {
  pkg: DriverPackage;
  /** Version parsed out of the jar filename. */
  version: string;
}

/** Split off a ".jreNN" family tail (MSSQL ships jre8/jre11 lines). */
const splitJre = (v: string): [string, string] => {
  const i = v.indexOf(".jre");
  return i < 0 ? [v, ""] : [v.slice(0, i), v.slice(i)];
};

const isPlainNumeric = (v: string) =>
  v.length > 0 && v.split(/[.-]/).every((s) => /^\d+$/.test(s));

/** True when `v` looks like a release version of the same shape the catalog
 * pins (plain numerics, optionally a .jreNN family tail). */
export function versionParses(v: string): boolean {
  return isPlainNumeric(splitJre(v)[0]);
}

/** Numeric segment-wise compare; <0 when a is older than b. Cross-family
 * (.jre8 vs .jre11) versions compare by base only — never report those as
 * stale, since switching family is a JVM-compatibility decision. */
export function compareVersions(a: string, b: string): number {
  const [aBase, aFam] = splitJre(a);
  const [bBase, bFam] = splitJre(b);
  if (aFam !== bFam) return 0;
  const seg = (v: string) => v.split(/[.-]/).map((s) => parseInt(s, 10) || 0);
  const A = seg(aBase);
  const B = seg(bBase);
  for (let i = 0; i < Math.max(A.length, B.length); i++) {
    const d = (A[i] ?? 0) - (B[i] ?? 0);
    if (d !== 0) return d < 0 ? -1 : 1;
  }
  return 0;
}

export function matchJar(packages: DriverPackage[], jarPath: string): JarMatch | null {
  const base = jarPath.split(/[\\/]/).pop() ?? "";
  for (const pkg of packages) {
    const i = pkg.filename.indexOf(pkg.version);
    if (i <= 0) continue;
    const prefix = pkg.filename.slice(0, i);
    const suffix = pkg.filename.slice(i + pkg.version.length);
    if (
      base.length > prefix.length + suffix.length &&
      base.startsWith(prefix) &&
      base.endsWith(suffix)
    ) {
      const version = base.slice(prefix.length, base.length - suffix.length);
      if (versionParses(version)) return { pkg, version };
    }
  }
  return null;
}
