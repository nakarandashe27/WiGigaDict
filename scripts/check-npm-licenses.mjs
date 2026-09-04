import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import process from "node:process";

const allowedLicenses = new Set([
  "(BSD-2-Clause OR MIT OR Apache-2.0)",
  "(MIT OR WTFPL)",
  "Apache-2.0",
  "Apache-2.0 OR MIT",
  "BlueOak-1.0.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "CC-BY-3.0",
  "CC-BY-4.0",
  "CC0-1.0",
  "ISC",
  "MIT",
  "Python-2.0",
]);

const repoRoot = resolve(import.meta.dirname, "..");
const lockPath = resolve(repoRoot, "apps/desktop/package-lock.json");
const outputPath = resolve(
  process.argv[2] ?? resolve(repoRoot, "artifacts/reports/npm-licenses.json"),
);
const lock = JSON.parse(await readFile(lockPath, "utf8"));

const packages = Object.entries(lock.packages)
  .filter(([packagePath]) => packagePath.includes("node_modules/"))
  .map(([packagePath, metadata]) => ({
    name: packagePath.replace(/^.*node_modules\//u, ""),
    version: metadata.version ?? null,
    license: metadata.license ?? null,
    development: metadata.dev === true,
    optional: metadata.optional === true,
  }))
  .sort((left, right) =>
    `${left.name}@${left.version}`.localeCompare(
      `${right.name}@${right.version}`,
    ),
  );

const violations = packages.filter(
  ({ license }) => typeof license !== "string" || !allowedLicenses.has(license),
);
const report = {
  schema: "wigigadict-license-inventory/v1",
  source: "apps/desktop/package-lock.json",
  policy: [...allowedLicenses].sort(),
  packages,
  violations,
};

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");

if (violations.length > 0) {
  console.error(`npm license policy rejected ${violations.length} package(s)`);
  process.exitCode = 1;
} else {
  console.log(`npm license policy accepted ${packages.length} package(s)`);
}
