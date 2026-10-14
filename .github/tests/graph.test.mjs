import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  mkdirSync,
  readFileSync,
  writeFileSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
} from "node:fs";
import { test } from "node:test";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { parse, stringify } from "yaml";
import { createHash } from "node:crypto";
import {
  evaluate,
  expand,
  fanout,
  runWorkflow,
  sourceDigest,
  workflows,
} from "../workflow-graph.mjs";
import { classifyHistory, history, payload } from "./policy.test.mjs";

import {
  deliveries,
  eventClasses,
  lifecycleSources,
  uncoveredSubscriptions,
  variants,
  variantKey,
} from "../event-classes.mjs";

function macRows(run) {
  return run.rows.filter((row) => row.enabled && row.macos);
}

// llmlint: ignore-block[shell_test_tiers_stay_split] The task requires this same github-workflows:test tier to drive real git histories through the classifier and then resolve the actual graph. These offline local repositories are the trust boundary; separating them would break that combined proof and adds no paid probe or packaging build.
test("actual workflow/call graph routes the complete event and publish workload and records telemetry", () => {
  const h = history();
  try {
    const graph = workflows();
    assert.deepEqual(uncoveredSubscriptions(graph), []);
    const scenarios = [],
      lifecycles = [];
    for (const { diff, fork, vars, cratePublish } of variants()) {
      const ordinary = payload("feature/mac-policy", fork);
      const release = payload();
      const ordinaryPolicy = classifyHistory(h, ordinary, h.heads[diff]);
      const releasePolicy = classifyHistory(h, release, h.heads.version);
      for (const [id, event, action, ref] of eventClasses) {
        const scenario = {
          id: `${id}/${variantKey({ diff, fork, vars, cratePublish })}`,
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
        const runs = deliveries(scenario, id).flatMap((delivery) =>
          fanout({ ...scenario, ...delivery }),
        );
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
      ].map((s) => ({
        id: s.id,
        runs: deliveries(s, lifecycleSources[s.id]).flatMap((delivery) =>
          fanout({ ...s, ...delivery, vars, cratePublish, inputs: {} }),
        ),
      }));
      lifecycles.push({
        diff,
        fork,
        vars,
        cratePublish,
        eventScenarios: lifecycle,
        runs: lifecycle.flatMap((event) => event.runs),
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
    // Reconcile every dependent measurement so only source coverage can refuse
    // these omissions, rather than a stale count or inconsistent lifecycle.
    for (const omission of ["workflow", "job"]) {
      const omitted = structuredClone(validTelemetry);
      const prune = (runs) =>
        runs
          .filter((run) => omission !== "workflow" || run.workflow !== "ci.yml")
          .map((run) => ({
            ...run,
            rows: run.rows.filter(
              (row) =>
                omission !== "job" ||
                run.workflow !== "ci.yml" ||
                row.job !== "macos",
            ),
          }));
      for (const scenario of omitted.scenarios)
        scenario.runs = prune(scenario.runs);
      for (const lifecycle of omitted.lifecycleCounts) {
        for (const event of lifecycle.eventScenarios)
          event.runs = prune(event.runs);
        lifecycle.runs = lifecycle.eventScenarios.flatMap(
          (event) => event.runs,
        );
        lifecycle.count = lifecycle.runs.filter(
          (run) => macRows(run).length,
        ).length;
      }
      omitted.jobCounts = omitted.scenarios.flatMap((scenario) =>
        scenario.runs.map((run) => ({
          scenario: scenario.id,
          workflow: run.workflow,
          event: run.event,
          count: macRows(run).length,
          rows: macRows(run),
        })),
      );
      omitted.jobsPerRun = Math.max(
        ...omitted.jobCounts.map((run) => run.count),
      );
      omitted.runsPerChange = Math.max(
        ...omitted.lifecycleCounts.map((run) => run.count),
      );
      const path = `${h.dir}/omitted-${omission}.json`;
      writeFileSync(path, JSON.stringify(omitted));
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
      assert.notEqual(
        result.status,
        0,
        `reporter accepted omitted ${omission}`,
      );
      assert.match(result.stderr, /incomplete (workflow|job) coverage/);
    }
    for (const malformed of [
      { ...validTelemetry, eventClasses: "abcdefghijkl" },
      { ...validTelemetry, scenarios: "not-an-array" },
      { ...validTelemetry, lifecycleCounts: [] },
      { ...validTelemetry, jobCounts: undefined },
      { ...validTelemetry, scenarios: validTelemetry.scenarios.slice(1) },
      { ...validTelemetry, jobCounts: validTelemetry.jobCounts.slice(1) },
      {
        ...validTelemetry,
        lifecycleCounts: validTelemetry.lifecycleCounts.slice(1),
      },
      {
        ...validTelemetry,
        jobCounts: validTelemetry.jobCounts.map((r, i) =>
          i
            ? r
            : { ...r, rows: r.rows.map((row) => ({ ...row, macos: false })) },
        ),
      },
      {
        ...validTelemetry,
        lifecycleCounts: validTelemetry.lifecycleCounts.map((r, i) =>
          i ? r : { ...r, eventScenarios: r.eventScenarios.slice(1) },
        ),
      },
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
    // The budgets entry point turns the analyzer's verdicts into its exit
    // status: the real registrations over fresh telemetry, the same commands
    // registered over a tighter limit, and a registration it cannot read.
    const checkBudgets = (cwd) =>
      spawnSync("node", [resolve(".github/check-budgets.mjs")], {
        cwd,
        encoding: "utf8",
      });
    const within = checkBudgets(".");
    assert.equal(within.status, 0, within.stderr);
    assert.match(within.stdout, /notignored-macos-jobs-per-run: within/);
    assert.match(within.stdout, /notignored-macos-runs-per-change: within/);
    const registrations = parse(readFileSync(".github/budgets.yaml", "utf8"));
    for (const budget of registrations.budgets) {
      budget.command = budget.command.map((part) =>
        part.endsWith(".mjs") || part.endsWith(".json")
          ? resolve(part.startsWith(".github/") ? part : `.github/${part}`)
          : part,
      );
      budget.threshold -= 1;
    }
    const tighter = `${h.dir}/tighter`;
    mkdirSync(`${tighter}/.github`, { recursive: true });
    writeFileSync(`${tighter}/.github/budgets.yaml`, stringify(registrations));
    const over = checkBudgets(tighter);
    assert.equal(over.status, 1, over.stderr);
    assert.match(over.stdout, /notignored-macos-jobs-per-run: over \(1\/0\)/);
    assert.match(
      over.stdout,
      /notignored-macos-runs-per-change: over \(2\/1\)/,
    );
    writeFileSync(
      `${tighter}/.github/budgets.yaml`,
      "schema_version: 1\nbudgets:\n  - id: broken\n",
    );
    const malformed = checkBudgets(tighter);
    assert.equal(malformed.status, 1);
    assert.match(malformed.stderr, /ACTION: run github-workflows:test/);
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
  for (const matrix of [
    { include: [7] },
    { include: { os: "macos-latest" } },
    { os: ["ubuntu-latest"], exclude: [null] },
    { os: ["ubuntu-latest"], include: [["macos-latest"]] },
  ])
    assert.throws(
      () => expand(matrix),
      /unresolved matrix (include|exclude): expected a list of mappings/,
      JSON.stringify(matrix),
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
    { "runs-on": ["self-hosted", "ubuntu-latest"] },
    { "runs-on": "ubuntu-invented" },
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

// llmlint: ignore-block[shell_test_tiers_stay_split] The task requires the unified workflow graph proof to reject reusable provenance and architecture-output drift. These mutations drive the same real TOML declaration reader as production graph resolution, not a separate host installer or paid probe.
test("reusable calls reject corrupted locked provenance and source before counting", () => {
  const root = mkdtempSync(
    join(
      process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(),
      "macos-reusable-",
    ),
  );
  try {
    mkdirSync(`${root}/.github/fixtures/reusable`, { recursive: true });
    const originals = Object.fromEntries(
      ["lock.json", "source.json", "screencomp-v0.4.2.yml"].map((name) => [
        name,
        readFileSync(`.github/fixtures/reusable/${name}`, "utf8"),
      ]),
    );
    const graph = workflows();
    const scenario = {
      event: "pull_request",
      ref: "refs/pull/1/merge",
      policy: { macosRequired: true },
      pr: payload("feature").pull_request,
    };
    writeFileSync(`${root}/screencomp.toml`, readFileSync("screencomp.toml"));
    for (const mutation of [
      "commit",
      "sourceSha256",
      "semanticSha256",
      "source",
      "fixture",
      "arch-command",
      "arch-output",
    ]) {
      for (const [name, text] of Object.entries(originals))
        writeFileSync(`${root}/.github/fixtures/reusable/${name}`, text);
      if (mutation.startsWith("arch-")) {
        const workflow = parse(originals["screencomp-v0.4.2.yml"]);
        if (mutation === "arch-command")
          workflow.jobs.arches.steps.find((step) => step.id === "read").run =
            workflow.jobs.arches.steps
              .find((step) => step.id === "read")
              .run.replace('-n "$ARCHES_OVERRIDE"', '-z "$ARCHES_OVERRIDE"');
        else
          workflow.jobs.arches.outputs.matrix =
            "$" + "{{ steps.read.outputs.missing }}";
        const text = stringify(workflow);
        const source = JSON.parse(originals["source.json"]);
        const lock = JSON.parse(originals["lock.json"]);
        source.source = text;
        lock.sourceSha256 = createHash("sha256").update(text).digest("hex");
        lock.semanticSha256 = createHash("sha256")
          .update(JSON.stringify(parse(text)))
          .digest("hex");
        writeFileSync(
          `${root}/.github/fixtures/reusable/source.json`,
          JSON.stringify(source),
        );
        writeFileSync(
          `${root}/.github/fixtures/reusable/lock.json`,
          JSON.stringify(lock),
        );
        writeFileSync(
          `${root}/.github/fixtures/reusable/screencomp-v0.4.2.yml`,
          text,
        );
      } else if (mutation === "source") {
        const source = JSON.parse(originals["source.json"]);
        source.source += "\n# changed source\n";
        writeFileSync(
          `${root}/.github/fixtures/reusable/source.json`,
          JSON.stringify(source),
        );
      } else if (mutation === "fixture")
        writeFileSync(
          `${root}/.github/fixtures/reusable/screencomp-v0.4.2.yml`,
          originals["screencomp-v0.4.2.yml"].replace(
            "ubuntu-latest",
            "macos-latest",
          ),
        );
      else {
        const lock = JSON.parse(originals["lock.json"]);
        lock[mutation] = "corrupted";
        writeFileSync(
          `${root}/.github/fixtures/reusable/lock.json`,
          JSON.stringify(lock),
        );
      }
      assert.throws(
        () =>
          runWorkflow(
            "visual-docs.yml",
            graph["visual-docs.yml"],
            scenario,
            root,
          ),
        mutation.startsWith("arch-")
          ? /unresolved reusable architecture/
          : /unresolved reusable workflow/,
        mutation,
      );
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// llmlint: ignore-end[shell_test_tiers_stay_split]

test("real YAML graph traverses downstream runs and refuses unsupported filters and cycles", () => {
  const root = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(), "macos-fanout-"),
  );
  try {
    mkdirSync(`${root}/.github/workflows`, { recursive: true });
    const entry = {
      name: "Entry",
      on: { push: { branches: ["main"] } },
      jobs: { proof: { "runs-on": "macos-latest" } },
    };
    const downstream = {
      name: "Downstream",
      on: { workflow_run: { workflows: ["Entry"], types: ["completed"] } },
      jobs: { proof: { "runs-on": "ubuntu-latest" } },
    };
    const write = () => {
      writeFileSync(`${root}/.github/workflows/entry.yml`, stringify(entry));
      writeFileSync(
        `${root}/.github/workflows/downstream.yml`,
        stringify(downstream),
      );
    };
    const scenario = {
      id: "synthetic-downstream",
      event: "push",
      ref: "refs/heads/main",
      policy: { macosRequired: false },
    };
    write();
    assert.deepEqual(
      fanout(scenario, root).map((run) => [run.workflow, run.event]),
      [
        ["entry.yml", "push"],
        ["downstream.yml", "workflow_run"],
      ],
    );
    downstream.on.workflow_run.workflows.push("Downstream");
    write();
    assert.throws(() => fanout(scenario, root), /fan-out cycle/);
    downstream.on.workflow_run.workflows.pop();
    entry.on.push["paths-ignore"] = ["docs/**"];
    write();
    assert.throws(() => fanout(scenario, root), /unsupported trigger filter/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("actual YAML branch and tag subscriptions honor inclusion, negation and re-inclusion", () => {
  const root = mkdtempSync(
    join(
      process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(),
      "macos-filters-",
    ),
  );
  try {
    mkdirSync(`${root}/.github/workflows`, { recursive: true });
    const workflow = {
      name: "Filtered",
      on: {
        push: {
          branches: [
            "release/**",
            "!release/private/**",
            "release/private/approved",
            "hotfix/*",
          ],
          tags: ["v*", "!v*-rc*", "v2.0.0-rc.1"],
        },
      },
      jobs: { proof: { "runs-on": "ubuntu-latest" } },
    };
    const path = `${root}/.github/workflows/filtered.yml`;
    writeFileSync(path, stringify(workflow));
    for (const [ref, included] of [
      ["refs/heads/release/public", true],
      ["refs/heads/release/private/denied", false],
      ["refs/heads/release/private/approved", true],
      ["refs/heads/hotfix/fix", true],
      ["refs/heads/hotfix/fix/nested", false],
      ["refs/heads/main", false],
      ["refs/tags/v1.2.3", true],
      ["refs/tags/v1.2.3-rc.1", false],
      ["refs/tags/v2.0.0-rc.1", true],
      ["refs/tags/other", false],
    ]) {
      const runs = fanout(
        { event: "push", ref, policy: { macosRequired: false } },
        root,
      );
      assert.deepEqual(
        runs.map((run) => run.workflow),
        included ? ["filtered.yml"] : [],
        ref,
      );
    }
    workflow.on = {
      pull_request: { branches: ["main", "!main-private"] },
      workflow_run: { workflows: ["CI"], branches: ["main"] },
    };
    writeFileSync(path, stringify(workflow));
    const pr = (base) => ({
      event: "pull_request",
      action: "opened",
      ref: "refs/pull/1/merge",
      pr: { ...payload().pull_request, base: { ref: base } },
      policy: { macosRequired: false },
    });
    assert.deepEqual(
      fanout(pr("main"), root).map((run) => run.workflow),
      ["filtered.yml"],
    );
    assert.deepEqual(fanout(pr("main-private"), root), []);
    assert.throws(
      () => fanout({ ...pr("main"), pr: { base: {} } }, root),
      /pull_request branch filter needs the pull request base ref/,
    );
    assert.throws(
      () =>
        fanout(
          {
            event: "workflow_run",
            action: "completed",
            workflow: "CI",
            ref: "refs/heads/main",
          },
          root,
        ),
      /unsupported workflow_run branch\/tag filter/,
    );
    // A subscription no event class delivers is named, never left unmeasured.
    workflow.on = {
      pull_request: { types: ["opened", "labeled"] },
      merge_group: null,
    };
    assert.deepEqual(uncoveredSubscriptions({ "filtered.yml": workflow }), [
      "filtered.yml: pull_request.labeled",
      "filtered.yml: merge_group",
    ]);
    // Without listed types a pull request fires only on GitHub's defaults.
    workflow.on = { pull_request: null };
    writeFileSync(path, stringify(workflow));
    for (const [action, fires] of [
      ["opened", true],
      ["closed", false],
    ])
      assert.equal(
        fanout({ ...pr("main"), action }, root).length,
        fires ? 1 : 0,
        action,
      );
    for (const config of [
      { types: "opened" },
      { types: [] },
      { branches: "main" },
      { branches: [7] },
    ]) {
      workflow.on = { pull_request: config };
      writeFileSync(path, stringify(workflow));
      assert.throws(
        () => fanout(pr("main"), root),
        /filter: expected a list of names/,
        JSON.stringify(config),
      );
    }
    workflow.on = { schedule: "weekly" };
    writeFileSync(path, stringify(workflow));
    assert.throws(
      () => fanout({ event: "schedule", ref: "refs/heads/main" }, root),
      /expected cron entries/,
    );
    workflow.on = { push: { branches: ["release/?"] } };
    writeFileSync(path, stringify(workflow));
    assert.throws(
      () => fanout({ event: "push", ref: "refs/heads/release/a" }, root),
      /unsupported trigger pattern/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// llmlint: ignore-block[shell_test_tiers_stay_split] The task requires the unified real workflow graph proof, including reconciliation with the production changes calculation. This drives its actual just/Nx read-only boundary over local histories, not a separate installer or expensive product journey.
test("production affected step and modeled crate routing agree over real source, release, SDK and unknown-base histories", () => {
  const root = mkdtempSync(
    join(
      process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(),
      "macos-affected-",
    ),
  );
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: root,
      encoding: "utf8",
      stdio: "pipe",
    }).trim();
  try {
    const configs = execFileSync(
      "git",
      ["ls-files", "-z", "*project.json", "*package.json"],
      { encoding: "utf8" },
    )
      .split("\0")
      .filter(Boolean);
    for (const path of [
      ...configs,
      "nx.json",
      "package-lock.json",
      ".gitignore",
      "justfile",
      "Cargo.toml",
      "Cargo.lock",
      "scripts/nx.sh",
      "scripts/nx-affected.sh",
      "scripts/ci-gate-tier.sh",
      "scripts/preserved-log.sh",
    ]) {
      mkdirSync(join(root, path, ".."), { recursive: true });
      writeFileSync(join(root, path), readFileSync(path));
    }
    symlinkSync(resolve("node_modules"), join(root, "node_modules"), "dir");
    git("init", "-q");
    git("config", "user.name", "Affected journey");
    git("config", "user.email", "affected@example.invalid");
    git("add", ".");
    git("commit", "-qm", "base");
    const base = git("rev-parse", "HEAD");
    const ci = workflows()["ci.yml"];
    const script = ci.jobs.changes.steps.find(
      (step) => step.id === "affected",
    ).run;
    const tierScript = ci.jobs.changes.steps.find(
      (step) => step.id === "tier",
    ).run;
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([name]) => !name.startsWith("NX_")),
    );
    for (const [path, expected, tier] of [
      ["src/lib.rs", true, "affected"],
      ["Cargo.lock", true, "affected"],
      ["CHANGELOG.md", true, "full"],
      ["python/notignored-sdk/README.md", false, "affected"],
    ]) {
      git("checkout", "-q", base);
      mkdirSync(join(root, path, ".."), { recursive: true });
      writeFileSync(
        join(root, path),
        readFileSync(join(root, path), { encoding: "utf8", flag: "a+" }) +
          (path.endsWith(".rs") ? "\n// changed\n" : "\n# changed\n"),
      );
      git("add", ".");
      git("commit", "-qm", `edit ${path}`);
      for (const namedBase of [
        base,
        "1111111111111111111111111111111111111111",
      ]) {
        const output = join(root, ".git", "outputs");
        writeFileSync(output, "");
        const event = tier === "full" ? "pull_request" : "push";
        const branch = tier === "full" ? "release-plz-drift" : "feature";
        const decision = spawnSync(
          "bash",
          ["-e", "-o", "pipefail", "-c", tierScript],
          {
            cwd: root,
            encoding: "utf8",
            env: {
              ...env,
              GATE_EVENT: event,
              GATE_HEAD_REF: branch,
              GATE_BEFORE: namedBase,
              GITHUB_OUTPUT: output,
            },
          },
        );
        assert.equal(decision.status, 0, decision.stderr);
        const selected = Object.fromEntries(
          readFileSync(output, "utf8")
            .trim()
            .split("\n")
            .map((line) => line.split("=")),
        );
        assert.equal(selected.tier, tier);
        writeFileSync(output, "");
        const calculation = spawnSync(
          "bash",
          ["-e", "-o", "pipefail", "-c", script],
          {
            cwd: root,
            encoding: "utf8",
            env: {
              ...env,
              CI: "1",
              TIER: selected.tier,
              NOTIGNORED_NX_BASE_SHA: selected.base,
              GITHUB_OUTPUT: output,
              GITHUB_BASE_REF: "",
              NOTIGNORED_NX_BASE_REF: "",
            },
          },
        );
        assert.equal(calculation.status, 0, calculation.stderr);
        const actual = readFileSync(output, "utf8").trim();
        const required = namedBase === base ? expected : true;
        assert.equal(
          actual,
          `crate=${required}`,
          `${path}: ${calculation.stderr}`,
        );
        if (namedBase === base && tier === "affected")
          assert.doesNotMatch(
            calculation.stderr,
            /no merge base|could not list/,
          );
        const rows = runWorkflow("ci.yml", ci, {
          event,
          ref: event === "push" ? "refs/heads/main" : "refs/pull/1/merge",
          policy: { macosRequired: true },
          pr: payload(branch).pull_request,
          ...(required ? {} : { crate: false }),
        });
        for (const row of rows.filter((row) =>
          ["cross", "install", "msrv", "deny"].includes(row.job),
        ))
          assert.equal(
            row.enabled,
            required,
            `${path}/${row.job} differs from production calculation`,
          );
      }
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
// llmlint: ignore-end[shell_test_tiers_stay_split]

// llmlint: ignore-block[shell_test_tiers_stay_split] The task requires one github-workflows proof to traverse real reusable matrices, whose architecture declaration is TOML. Parsing that declaration is part of the offline graph boundary, not a separate host installer journey or paid probe.
test("real reusable resolver accepts an explicit architecture and rejects malformed overrides and configuration", () => {
  const root = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(), "macos-arches-"),
  );
  try {
    mkdirSync(`${root}/.github/fixtures/reusable`, { recursive: true });
    for (const name of ["lock.json", "source.json", "screencomp-v0.4.2.yml"])
      writeFileSync(
        `${root}/.github/fixtures/reusable/${name}`,
        readFileSync(`.github/fixtures/reusable/${name}`),
      );
    writeFileSync(`${root}/screencomp.toml`, readFileSync("screencomp.toml"));
    const workflow = structuredClone(workflows()["visual-docs.yml"]);
    const caller = Object.values(workflow.jobs).find((job) => job.uses);
    const scenario = {
      event: "pull_request",
      ref: "refs/pull/1/merge",
      policy: { macosRequired: true },
      pr: payload("feature").pull_request,
    };
    caller.with.arches = '["aarch64"]';
    const capture = runWorkflow(
      "visual-docs.yml",
      workflow,
      scenario,
      root,
    ).find((row) => row.call?.endsWith("/capture"));
    assert.equal(capture.matrix.arch, "aarch64");
    assert.deepEqual(capture.runner, ["ubuntu-24.04-arm"]);
    for (const value of ["[", "[]", "[1]"]) {
      caller.with.arches = value;
      assert.throws(() =>
        runWorkflow("visual-docs.yml", workflow, scenario, root),
      );
    }
    delete caller.with.arches;
    writeFileSync(`${root}/screencomp.toml`, "[capture]\narches=[]\n");
    assert.throws(
      () => runWorkflow("visual-docs.yml", workflow, scenario, root),
      /configured architectures/,
    );
    writeFileSync(`${root}/screencomp.toml`, "not valid TOML[");
    assert.throws(
      () => runWorkflow("visual-docs.yml", workflow, scenario, root),
      /TOML/,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// llmlint: ignore-end[shell_test_tiers_stay_split]
