import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { test } from "node:test";
import { parse } from "yaml";
import {
  evaluate,
  expand,
  fanout,
  runWorkflow,
  sourceDigest,
  workflows,
} from "../workflow-graph.mjs";
import { classifyHistory, history, payload } from "./policy.test.mjs";

import { eventClasses } from "../event-classes.mjs";

function macRows(run) {
  return run.rows.filter((row) => row.enabled && row.macos);
}

// llmlint: ignore-block[shell_test_tiers_stay_split] The task requires this same github-workflows:test tier to drive real git histories through the classifier and then resolve the actual graph. These offline local repositories are the trust boundary; separating them would break that combined proof and adds no paid probe or packaging build.
test("actual workflow/call graph routes the complete event and publish workload and records telemetry", () => {
  const h = history();
  try {
    const graph = workflows();
    const scenarios = [],
      lifecycles = [];
    for (const cratePublish of [false, true])
      for (const diff of ["source", "version", "dependency"])
        for (const fork of [false, true])
          for (const pypi of [false, true])
            for (const npm of [false, true]) {
              const vars = {
                PYPI_PUBLISH: String(pypi),
                NPM_PUBLISH: String(npm),
              };
              const ordinary = payload("feature/mac-policy", fork);
              const release = payload();
              const ordinaryPolicy = classifyHistory(
                h,
                ordinary,
                h.heads[diff],
              );
              const releasePolicy = classifyHistory(
                h,
                release,
                h.heads.version,
              );
              for (const [id, event, action, ref] of eventClasses) {
                const scenario = {
                  id: `${id}/${diff}/${fork ? "fork" : "same"}/${pypi}/${npm}/${cratePublish}`,
                  event,
                  action,
                  ref,
                  vars,
                  policy: ordinaryPolicy,
                  pr: ordinary.pull_request,
                  diff,
                  ownership: fork ? "fork" : "same",
                  inputs: { diagnostic: id === "manual-opt-in" },
                  cratePublish,
                  workflow: "Release",
                };
                const runs = fanout(scenario);
                for (const run of runs) {
                  for (const row of macRows(run))
                    assert.ok(
                      (run.workflow === "ci.yml" &&
                        event === "pull_request" &&
                        row.job === "macos") ||
                        (run.workflow === "release.yml" &&
                          event === "push" &&
                          ref.startsWith("refs/tags/") &&
                          row.job === "macos-build") ||
                        (run.workflow === "published-smoke.yml" &&
                          event === "workflow_dispatch" &&
                          scenario.inputs.diagnostic &&
                          row.job === "macos-diagnostic"),
                      `unauthorized Mac location ${JSON.stringify({ scenario, row })}`,
                    );
                }
                scenarios.push({ ...scenario, runs });
              }
              const lifecycle = [
                {
                  id: "ordinary-pr-push",
                  event: "pull_request",
                  action: "synchronize",
                  ref: "refs/pull/1/merge",
                  pr: ordinary.pull_request,
                  policy: ordinaryPolicy,
                },
                {
                  id: "ordinary-base-merge",
                  event: "push",
                  ref: "refs/heads/main",
                  policy: ordinaryPolicy,
                },
                {
                  id: "version-only-release-pr",
                  event: "pull_request",
                  action: "opened",
                  ref: "refs/pull/2/merge",
                  pr: release.pull_request,
                  policy: releasePolicy,
                },
                {
                  id: "release-base-merge",
                  event: "push",
                  ref: "refs/heads/main",
                  policy: releasePolicy,
                },
                {
                  id: "release-tag",
                  event: "push",
                  ref: "refs/tags/v0.2.0",
                  policy: releasePolicy,
                },
                {
                  id: "release-published",
                  event: "release",
                  action: "published",
                  ref: "refs/tags/v0.2.0",
                  policy: releasePolicy,
                },
                {
                  id: "automatic-smoke",
                  event: "workflow_run",
                  action: "completed",
                  workflow: "Release",
                  ref: "refs/heads/main",
                  policy: releasePolicy,
                },
                {
                  id: "schedule",
                  event: "schedule",
                  ref: "refs/heads/main",
                  policy: releasePolicy,
                },
              ].flatMap((s) =>
                fanout({ ...s, vars, cratePublish, inputs: {} }),
              );
              lifecycles.push({
                diff,
                fork,
                vars,
                cratePublish,
                runs: lifecycle,
              });
            }
    for (const kind of ["source", "dependency", "metadata"]) {
      const scenario = {
        id: `forged-release-${kind}`,
        event: "pull_request",
        action: "opened",
        ref: "refs/pull/3/merge",
        pr: payload().pull_request,
        policy: classifyHistory(h, payload(), h.heads[kind]),
        vars: {},
      };
      assert.equal(
        macRows(fanout(scenario).find((r) => r.workflow === "ci.yml")).some(
          (r) => r.job === "macos",
        ),
        true,
      );
    }
    const releaseOnly = {
      id: "release-only",
      event: "pull_request",
      action: "opened",
      ref: "refs/pull/2/merge",
      pr: payload().pull_request,
      policy: classifyHistory(h, payload()),
      vars: {},
    };
    assert.equal(
      macRows(fanout(releaseOnly).find((r) => r.workflow === "ci.yml")).length,
      0,
    );
    for (const policy of [{ macosRequired: true }, classifyHistory(h, "{")]) {
      assert.ok(
        macRows(
          fanout({ ...releaseOnly, policy }).find(
            (r) => r.workflow === "ci.yml",
          ),
        ).length,
      );
    }
    for (const scenario of [
      releaseOnly,
      {
        ...releaseOnly,
        pr: payload("feature").pull_request,
        policy: { macosRequired: true },
      },
    ]) {
      const rows = runWorkflow("ci.yml", graph["ci.yml"], scenario);
      assert.equal(
        rows.filter((row) => row.display === "required" && row.enabled).length,
        1,
      );
    }
    const jobCounts = scenarios.flatMap((s) =>
      s.runs.map((run) => ({
        scenario: s.id,
        workflow: run.workflow,
        event: run.event,
        count: macRows(run).length,
        rows: macRows(run),
      })),
    );
    const lifecycleCounts = lifecycles.map((l) => ({
      ...l,
      count: l.runs.filter((run) => macRows(run).length).length,
    }));
    mkdirSync(".github/.telemetry", { recursive: true });
    writeFileSync(
      ".github/.telemetry/macos-policy.json",
      JSON.stringify(
        {
          sourceDigest: sourceDigest(),
          eventClasses: eventClasses.map((row) => row[0]),
          scenarios,
          jobsPerRun: Math.max(...jobCounts.map((r) => r.count)),
          runsPerChange: Math.max(...lifecycleCounts.map((l) => l.count)),
          jobCounts,
          lifecycleCounts,
        },
        null,
        2,
      ),
    );
    for (const metric of ["jobs-per-run", "runs-per-change"]) {
      const resultFile = `${h.dir}/${metric}.json`;
      const measurement = spawnSync(
        "node",
        [
          ".github/macos-budget.mjs",
          "--metric",
          metric,
          "--telemetry",
          ".github/.telemetry/macos-policy.json",
        ],
        {
          encoding: "utf8",
          env: { ...process.env, ONEBUDGETSPEC_RESULT: resultFile },
        },
      );
      assert.equal(measurement.status, 0, measurement.stderr);
      const recorded = JSON.parse(readFileSync(resultFile, "utf8"));
      assert.ok(Number.isFinite(recorded.value));
      assert.match(recorded.detail, /workflow/);
    }
    const validTelemetry = JSON.parse(
      readFileSync(".github/.telemetry/macos-policy.json", "utf8"),
    );
    for (const malformed of [
      { ...validTelemetry, eventClasses: "abcdefghijkl" },
      { ...validTelemetry, scenarios: "not-an-array" },
      { ...validTelemetry, lifecycleCounts: [] },
      { ...validTelemetry, jobCounts: undefined },
      { ...validTelemetry, jobsPerRun: undefined },
      { ...validTelemetry, jobsPerRun: -1 },
      { ...validTelemetry, jobsPerRun: 0 },
      { ...validTelemetry, runsPerChange: 0 },
      {
        ...validTelemetry,
        lifecycleCounts: validTelemetry.lifecycleCounts.map((row) => ({
          ...row,
          count: row.count + 1,
        })),
      },
      { ...validTelemetry, runsPerChange: 1.5 },
      {
        ...validTelemetry,
        jobCounts: [
          { workflow: "ci.yml", event: "pull_request", count: 0, rows: [{}] },
        ],
      },
    ]) {
      const path = `${h.dir}/malformed-telemetry.json`;
      writeFileSync(path, JSON.stringify(malformed));
      const result = spawnSync(
        "node",
        [
          ".github/macos-budget.mjs",
          "--metric",
          "jobs-per-run",
          "--telemetry",
          path,
        ],
        { encoding: "utf8" },
      );
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /ACTION: run github-workflows:test/);
    }
    const unknown = spawnSync(
      "node",
      [
        ".github/macos-budget.mjs",
        "--metric",
        "unknown",
        "--telemetry",
        ".github/.telemetry/macos-policy.json",
      ],
      { encoding: "utf8" },
    );
    assert.notEqual(unknown.status, 0);
    assert.match(unknown.stderr, /unknown metric/);
    const stale = `${h.dir}/stale.json`;
    writeFileSync(stale, JSON.stringify({ sourceDigest: "stale" }));
    for (const telemetry of [stale, `${h.dir}/missing.json`]) {
      const result = spawnSync(
        "node",
        [
          ".github/macos-budget.mjs",
          "--metric",
          "jobs-per-run",
          "--telemetry",
          telemetry,
        ],
        { encoding: "utf8" },
      );
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /ACTION: run github-workflows:test/);
    }
  } finally {
    h.close();
  }
});

// llmlint: ignore-end[shell_test_tiers_stay_split]

test("matrix expansion and published expression evaluator resolve runners; unresolved routing is an error", () => {
  assert.deepEqual(
    expand({
      os: ["ubuntu-latest", "windows-latest"],
      include: [{ os: "macos-latest" }],
      exclude: [{ os: "windows-latest" }],
    }),
    [{ os: "ubuntu-latest" }, { os: "macos-latest" }],
  );
  assert.deepEqual(
    expand({
      os: ["ubuntu-latest", "windows-latest"],
      exclude: [{ os: "ubuntu-latest" }],
      include: [
        { os: "windows-latest", target: "x64" },
        { os: "macos-latest" },
      ],
    }),
    [{ os: "windows-latest", target: "x64" }, { os: "macos-latest" }],
  );
  assert.equal(
    evaluate(
      "needs.mac.result == 'success' && !cancelled()",
      { needs: { mac: { result: "failure" } } },
      ["failure"],
    ).coerceString(),
    "false",
  );
  const scenario = {
    event: "push",
    ref: "refs/heads/main",
    policy: { macosRequired: false },
  };
  for (const job of [
    // biome-ignore lint/suspicious/noTemplateCurlyInString: This fixture is an Actions expression, not a JavaScript template.
    { "runs-on": "${{ needs.unknown.outputs.os }}" },
    { uses: "unknown/repo/.github/workflows/call.yml@v1" },
    { "runs-on": "self-hosted" },
    { "runs-on": "ubuntu-latest", if: "invented()" },
  ]) {
    assert.throws(
      () => runWorkflow("broken.yml", { jobs: { broken: job } }, scenario),
      /unresolved|unclassified|Unrecognized/,
    );
  }
});

test("Linux and Windows commands and event lanes remain in their original jobs", () => {
  const graph = workflows();
  for (const name of ["ci", "release", "published-smoke"]) {
    const baseline = parse(
      readFileSync(`.github/fixtures/baseline/${name}.yml`, "utf8"),
    );
    const final = graph[`${name}.yml`];
    for (const [id, old] of Object.entries(baseline.jobs)) {
      if (id === "changes" || id === "required") continue;
      const originalRows = expand(old.strategy?.matrix);
      const nonMac = originalRows.filter(
        (row) => !String(row.os ?? old["runs-on"]).startsWith("macos-"),
      );
      if (!nonMac.length) continue;
      const current = final.jobs[id];
      assert.ok(current, `lost ${name}/${id}`);
      assert.equal(current.if, old.if, `changed Linux/Windows routing ${id}`);
      const normalize = (steps) =>
        steps.map((step) => ({
          ...step,
          ...(step.run
            ? {
                run: step.run.replace(
                  'throw new Error("installed SDK returned " + JSON.stringify(report));',
                  // biome-ignore lint/suspicious/noTemplateCurlyInString: Match the baseline workflow source literally; evaluating its template would change the assertion.
                  "throw new Error(`installed SDK returned ${JSON.stringify(report)}`);",
                ),
              }
            : {}),
        }));
      assert.deepEqual(
        normalize(current.steps),
        normalize(old.steps),
        `changed Linux/Windows commands ${id}`,
      );
      const previousNeeds =
        typeof old.needs === "string" ? [old.needs] : (old.needs ?? []);
      const currentNeeds =
        typeof current.needs === "string"
          ? [current.needs]
          : (current.needs ?? []);
      for (const need of previousNeeds)
        assert.ok(
          currentNeeds.includes(need),
          `lost prerequisite ${name}/${id}/${need}`,
        );
      for (const row of nonMac)
        assert.ok(
          expand(current.strategy?.matrix).some(
            (now) => JSON.stringify(now) === JSON.stringify(row),
          ),
          `lost ${name}/${id}/${JSON.stringify(row)}`,
        );
    }
  }
});
