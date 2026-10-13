// Read the test tier's measurement; thresholds belong only to onebudgetspec.
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { report } from "@onebudgetspec/sdk";
import { sourceDigest } from "./workflow-graph.mjs";

try {
  const { values } = parseArgs({
    options: { metric: { type: "string" }, telemetry: { type: "string" } },
  });
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  process.chdir(root);
  const telemetry = JSON.parse(readFileSync(values.telemetry, "utf8"));
  if (telemetry.sourceDigest !== sourceDigest())
    throw new Error("stale telemetry: source digest differs");
  const isCount = (value) => Number.isSafeInteger(value) && value >= 0;
  if (
    !Array.isArray(telemetry.eventClasses) ||
    telemetry.eventClasses.length !== 12 ||
    telemetry.eventClasses.some((value) => typeof value !== "string") ||
    new Set(telemetry.eventClasses).size !== 12 ||
    !Array.isArray(telemetry.scenarios) ||
    !telemetry.scenarios.length ||
    !Array.isArray(telemetry.lifecycleCounts) ||
    !telemetry.lifecycleCounts.length ||
    !Array.isArray(telemetry.jobCounts) ||
    !telemetry.jobCounts.length
  )
    throw new Error("incomplete telemetry workload");
  for (const row of telemetry.jobCounts) {
    if (
      !row ||
      typeof row.workflow !== "string" ||
      typeof row.event !== "string" ||
      !Array.isArray(row.rows) ||
      !isCount(row.count) ||
      row.count !== row.rows.length
    )
      throw new Error("invalid workflow job measurement");
  }
  for (const lifecycle of telemetry.lifecycleCounts) {
    if (
      !lifecycle ||
      !Array.isArray(lifecycle.runs) ||
      !isCount(lifecycle.count)
    )
      throw new Error("invalid lifecycle measurement");
  }
  if (!isCount(telemetry.jobsPerRun) || !isCount(telemetry.runsPerChange))
    throw new Error("invalid recorded count");
  const field = {
    "jobs-per-run": "jobsPerRun",
    "runs-per-change": "runsPerChange",
  }[values.metric];
  if (!field || !Number.isFinite(telemetry[field]))
    throw new Error("unknown metric or missing measurement");
  const detail = JSON.stringify(
    values.metric === "jobs-per-run"
      ? telemetry.jobCounts
      : telemetry.lifecycleCounts,
  );
  report(telemetry[field], detail);
  console.log(
    `${values.metric}: ${telemetry[field]} (recorded workflow/event/architecture detail)`,
  );
} catch (error) {
  console.error(
    `macos-budget: ${error.message}\nACTION: run github-workflows:test to record current telemetry, then rerun the budget`,
  );
  process.exitCode = 1;
}
