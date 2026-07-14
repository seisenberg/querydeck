import assert from "node:assert/strict";
import { compareVersions, matchJar, versionParses } from "./driverstale.ts";
import type { DriverPackage } from "./api.ts";

const pkg = (id: string, version: string, filename: string): DriverPackage => ({
  id,
  title: id,
  version,
  url: "",
  filename,
  driverClass: "",
  urlTemplate: "",
  defaultPort: 0,
  notes: "",
  downloaded: false,
  path: null,
});

const CATALOG = [
  pkg("jdbc-postgresql", "42.7.13", "postgresql-42.7.13.jar"),
  pkg("jdbc-clickhouse", "0.9.8", "clickhouse-jdbc-0.9.8-all.jar"),
  pkg("jdbc-mssql", "13.4.0.jre11", "mssql-jdbc-13.4.0.jre11.jar"),
  pkg("jdbc-informix", "15.0.1.3", "informix-jdbc-15.0.1.3.jar"),
];

// --- compareVersions: numeric segments, not lexicographic.
assert.ok(compareVersions("42.7.2", "42.10.0") < 0, "42.7.2 < 42.10.0");
assert.ok(compareVersions("482", "479") > 0, "482 > 479");
assert.equal(compareVersions("42.7", "42.7.0"), 0, "missing segments are 0");
assert.ok(compareVersions("13.4.0.jre11", "13.6.0.jre11") < 0, "same jre family compares");
// Cross-family is a JVM-compat decision, never reported as stale.
assert.equal(compareVersions("13.4.0.jre8", "13.4.0.jre11"), 0, "cross-family is equal");

// --- versionParses: plain releases only.
assert.ok(versionParses("42.8.0"));
assert.ok(versionParses("13.6.1.jre11"));
assert.ok(!versionParses("42.8.0-rc1"));
assert.ok(!versionParses("sources"));

// --- matchJar: identifies catalog jars in any directory, any version.
{
  const m = matchJar(CATALOG, "/opt/jars/postgresql-42.6.0.jar");
  assert.equal(m?.pkg.id, "jdbc-postgresql");
  assert.equal(m?.version, "42.6.0");
}
{
  // Classifier suffix must survive the version swap.
  const m = matchJar(CATALOG, "/x/clickhouse-jdbc-0.7.2-all.jar");
  assert.equal(m?.pkg.id, "jdbc-clickhouse");
  assert.equal(m?.version, "0.7.2");
}
{
  // Renamed local prefix (Informix) still matches by its local pattern.
  const m = matchJar(CATALOG, "/x/informix-jdbc-4.50.10.jar");
  assert.equal(m?.pkg.id, "jdbc-informix");
  assert.equal(m?.version, "4.50.10");
}
// Non-catalog, renamed, or non-release jars never match (old drivers by
// choice must not be nagged).
assert.equal(matchJar(CATALOG, "/x/my-custom-driver.jar"), null);
assert.equal(matchJar(CATALOG, "/x/postgresql-42.6.0-sources.jar"), null);
assert.equal(matchJar(CATALOG, "/x/clickhouse-jdbc-0.7.2.jar"), null, "missing classifier");

console.log("driverstale: all assertions passed");
