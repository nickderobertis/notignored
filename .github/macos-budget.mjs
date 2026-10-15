// Read the test tier's measurement; thresholds belong only to onebudgetspec.
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs, isDeepStrictEqual } from "node:util";
import { report } from "@onebudgetspec/sdk";
import {
  eventClasses,
  variants,
  variantKey,
  lifecycleEvents,
  lifecycleKey,
  lifecycleRoutes,
  lifecycleSources,
  macDeferredLifecycleEvents,
  uncoveredSubscriptions,
} from "./event-classes.mjs";
import { coverageValidator } from "./telemetry-coverage.mjs";
import { sourceDigest, workflows } from "./workflow-graph.mjs";

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
    telemetry.eventClasses.length !== eventClasses.length ||
    telemetry.eventClasses.some((value) => typeof value !== "string") ||
    eventClasses.some(([id]) => !telemetry.eventClasses.includes(id)) ||
    !Array.isArray(telemetry.scenarios) ||
    !telemetry.scenarios.length ||
    !Array.isArray(telemetry.lifecycleCounts) ||
    !telemetry.lifecycleCounts.length ||
    !Array.isArray(telemetry.jobCounts) ||
    !telemetry.jobCounts.length
  )
    throw new Error("incomplete telemetry workload");
  // llmlint: ignore-block[boundary_inputs_validated] The reporter must consume recorded enablement without re-evaluating workflow conditions or producing fresh measurements. Source-declared inventory and full recorded-row consistency are validated below; the production-driven graph tier owns condition evaluation.
  const macRows = (run) => run.rows.filter((row) => row.enabled && row.macos);
  const validRuns = (runs) =>
    Array.isArray(runs) &&
    runs.every(
      (run) =>
        run &&
        typeof run.workflow === "string" &&
        typeof run.event === "string" &&
        Array.isArray(run.rows) &&
        run.rows.every(
          (row) =>
            row &&
            typeof row.enabled === "boolean" &&
            typeof row.macos === "boolean" &&
            Array.isArray(row.runner) &&
            row.runner.length === 1 &&
            typeof row.runner[0] === "string" &&
            row.macos === row.runner[0].startsWith("macos-") &&
            typeof row.job === "string",
        ),
    );
  // llmlint: ignore-end[boundary_inputs_validated]
  const sameSet = (actual, expected) =>
    actual.length === expected.length &&
    new Set(actual).size === actual.length &&
    expected.every((id) => actual.includes(id));
  const expectedVariants = variants();
  const expectedScenarioIds = expectedVariants.flatMap((variant) =>
    eventClasses.map(([id]) => `${id}/${variantKey(variant)}`),
  );
  if (
    !sameSet(
      telemetry.scenarios.map((row) => row.id),
      expectedScenarioIds,
    )
  )
    throw new Error("incomplete scenario workload");
  const uncovered = uncoveredSubscriptions(workflows(root));
  if (uncovered.length)
    throw new Error(`unmeasured subscriptions: ${uncovered.join(", ")}`);
  const validateCoverage = coverageValidator(root);
  for (const scenario of telemetry.scenarios) {
    const event = eventClasses.find(([id]) => scenario.id.startsWith(`${id}/`));
    if (
      !validRuns(scenario.runs) ||
      scenario.event !== event[1] ||
      scenario.action !== event[2] ||
      scenario.ref !== event[3] ||
      scenario.id !==
        `${event[0]}/${variantKey({ ...scenario, fork: scenario.ownership === "fork" })}`
    )
      throw new Error("invalid scenario measurement");
    validateCoverage(scenario, event[0]);
  }
  const expectedJobs = telemetry.scenarios.flatMap((scenario) =>
    scenario.runs.map((run) => ({
      scenario: scenario.id,
      workflow: run.workflow,
      event: run.event,
      count: macRows(run).length,
      rows: macRows(run),
    })),
  );
  if (!isDeepStrictEqual(telemetry.jobCounts, expectedJobs))
    throw new Error("job measurement differs from scenario rows");
  if (
    !sameSet(
      telemetry.lifecycleCounts.map(lifecycleKey),
      expectedVariants.flatMap((variant) =>
        Object.keys(lifecycleRoutes).map((route) =>
          lifecycleKey({ ...variant, route }),
        ),
      ),
    )
  )
    throw new Error("incomplete lifecycle workload");
  const comparable = (runs, omitMac = false) =>
    runs.map(({ workflow, event, rows }) => ({
      workflow,
      event,
      rows: rows.map((row) =>
        omitMac && row.macos ? { ...row, enabled: false } : row,
      ),
    }));
  for (const lifecycle of telemetry.lifecycleCounts) {
    if (
      !validRuns(lifecycle.runs) ||
      !isCount(lifecycle.count) ||
      !Object.hasOwn(lifecycleRoutes, lifecycle.route) ||
      !Array.isArray(lifecycle.eventScenarios) ||
      !sameSet(
        lifecycle.eventScenarios.map((event) => event.id),
        lifecycleEvents(lifecycle.route),
      ) ||
      !isDeepStrictEqual(
        lifecycle.runs,
        lifecycle.eventScenarios.flatMap((event) => event.runs),
      )
    )
      throw new Error("invalid lifecycle measurement");
    for (const event of lifecycle.eventScenarios) {
      const releasePr = event.id === "version-only-release-pr";
      const variant = releasePr
        ? { ...lifecycle, diff: "version", fork: false }
        : lifecycle;
      const scenario = telemetry.scenarios.find(
        (row) =>
          row.id === `${lifecycleSources[event.id]}/${variantKey(variant)}`,
      );
      if (
        !validRuns(event.runs) ||
        !isDeepStrictEqual(
          comparable(event.runs),
          comparable(scenario.runs, macDeferredLifecycleEvents.has(event.id)),
        )
      )
        throw new Error("lifecycle event differs from recorded scenarios");
    }
    if (
      lifecycle.count !==
      lifecycle.runs.filter((run) => macRows(run).length).length
    )
      throw new Error("lifecycle count differs from rows");
  }
  if (!isCount(telemetry.jobsPerRun) || !isCount(telemetry.runsPerChange))
    throw new Error("invalid recorded count");
  if (
    telemetry.jobsPerRun !==
      Math.max(...telemetry.jobCounts.map((row) => row.count)) ||
    telemetry.runsPerChange !==
      Math.max(...telemetry.lifecycleCounts.map((row) => row.count))
  )
    throw new Error("aggregate measurement differs from detail");
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
