import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parse } from "yaml";
import { workflows, runWorkflow } from "../workflow-graph.mjs";

test("the Darwin release is native arm64 only, with exact-run artifacts and pre-publication installers", () => {
  const release = workflows()["release.yml"].jobs;
  const mac = release["macos-build"];
  const darwin = mac.env.RELEASE_TARGETS.split(/\s+/).sort();
  const baseline = parse(
    readFileSync(".github/fixtures/baseline/release.yml", "utf8"),
  );
  // Intel macOS is discontinued: of the baseline's Darwin targets only arm64
  // remains, and every other baseline target is still released.
  const baselineTargets = baseline.jobs.upload.strategy.matrix.include.map(
    (row) => row.target,
  );
  assert.ok(baselineTargets.includes("x86_64-apple-darwin"));
  assert.deepEqual(
    darwin,
    baselineTargets
      .filter((target) => target.endsWith("apple-darwin"))
      .filter((target) => target !== "x86_64-apple-darwin")
      .sort(),
  );
  assert.deepEqual(darwin, ["aarch64-apple-darwin"]);
  for (const id of ["upload", "build-wheels", "build-npm"])
    assert.deepEqual(
      [
        ...release[id].strategy.matrix.include.map((row) => row.target),
        ...darwin,
      ].sort(),
      baselineTargets
        .filter((target) => target !== "x86_64-apple-darwin")
        .sort(),
      `${id} lost or gained a released target`,
    );
  const run = mac.steps.map((step) => step.run ?? "").join("\n");
  for (const target of darwin)
    assert.ok(run.includes(`scripts/stage-release.sh staged/${target}`));
  assert.match(run, /test "\$\(uname -m\)" = arm64/);
  const source = readFileSync(".github/workflows/release.yml", "utf8");
  for (const intel of [
    /x86_64-apple-darwin/,
    /arch -x86_64/,
    /install-rosetta/,
    /macos-15-intel/,
    /cpython-3\.12-macos-x86_64/,
  ])
    assert.doesNotMatch(source, intel);
  assert.ok(!mac.steps.some((step) => step.with?.architecture === "x64"));
  for (const id of [
    "publish-release",
    "publish-pypi",
    "publish-npm",
    "upload-darwin",
  ])
    assert.ok(
      release[id].needs.includes("macos-build"),
      `${id} can precede Mac verification`,
    );
  for (const step of mac.steps.filter((step) =>
    step.uses?.startsWith("actions/download-artifact"),
  )) {
    assert.equal(step.with["run-id"], undefined, "download must use this run");
    assert.equal(
      step.with["github-token"],
      undefined,
      "download must not search other runs",
    );
    assert.ok(
      release["build-sdk-npm"].steps.some(
        (upload) => upload.with?.name === step.with.name,
      ),
      `unresolved exact-head artifact ${step.with.name}`,
    );
  }
  for (const [name, path] of [
    ["wheels-darwin", "release-wheels/*.whl"],
    ["npm-darwin", "release-npm/*.tgz"],
    ["archives-darwin", "release-archives/*"],
  ]) {
    assert.ok(
      mac.steps.some(
        (step) => step.with?.name === name && step.with.path === path,
      ),
      `lost flat ${name} delivery`,
    );
  }
});

test("modeled guard and arch outputs remain tied to the real commands and output expressions", () => {
  const graph = workflows();
  const guard = graph["release.yml"].jobs.guard;
  assert.match(
    guard.outputs.publish,
    /^\$\{\{ steps\.check\.outputs\.publish \}\}$/,
  );
  assert.match(guard.steps[0].run, /echo "publish=true"/);
  assert.match(guard.steps[0].run, /echo "publish=false"/);
  const plz = graph["release-plz.yml"].jobs.guard;
  assert.match(
    plz.outputs.enabled,
    /^\$\{\{ steps\.check\.outputs\.enabled \}\}$/,
  );
  assert.match(
    plz.steps.find((step) => step.id === "check").run,
    /echo "enabled=true"/,
  );
  assert.match(
    plz.steps.find((step) => step.id === "check").run,
    /echo "enabled=false"/,
  );
  const changes = graph["ci.yml"].jobs.changes;
  assert.match(
    changes.outputs.macos,
    /^\$\{\{ steps\.policy\.outputs\.macos \}\}$/,
  );
  assert.match(
    changes.outputs.crate,
    /^\$\{\{ steps\.affected\.outputs\.crate \}\}$/,
  );
  assert.match(
    changes.steps.find((step) => step.id === "affected").run,
    /just affected-crate/,
  );
  const reusable = readFileSync(
    ".github/fixtures/reusable/screencomp-v0.4.2.yml",
    "utf8",
  );
  assert.match(reusable, /ARCHES_OVERRIDE: \$\{\{ inputs.arches \}\}/);
  assert.match(reusable, /matrix: \$\{\{ steps.read.outputs.matrix \}\}/);
  assert.match(reusable, /screencomp arches --format json/);
});

// llmlint: ignore-block[shell_test_tiers_stay_split] The task requires the graph proof to evaluate actual job conditions; release.yml's and release-plz.yml's guard outputs are decided by their own few-line `[ -n "$TOKEN" ]` step, so the offline model runs that step under bash with no network, installer, or packaging build. Moving it to another tier would leave this tier's routing resting on a guessed output.
test("real guard commands decide modeled prerequisite outputs, including changed conditions", () => {
  const graph = workflows();
  const scenario = {
    event: "push",
    ref: "refs/tags/v0.2.0",
    policy: { macosRequired: false },
    cratePublish: false,
    releasePlz: false,
  };
  const enabled = (name, job, input = graph[name]) =>
    runWorkflow(name, input, scenario).find((row) => row.job === job).enabled;
  assert.equal(enabled("release.yml", "publish-crate"), false);
  assert.equal(enabled("release-plz.yml", "release-plz"), false);
  for (const [name, job] of [
    ["release.yml", "publish-crate"],
    ["release-plz.yml", "release-plz"],
  ]) {
    const changed = structuredClone(graph[name]);
    changed.jobs.guard.steps[0].run = changed.jobs.guard.steps[0].run.replace(
      '[ -n "$TOKEN" ]',
      '[ -z "$TOKEN" ]',
    );
    assert.equal(
      enabled(name, job, changed),
      true,
      "actual changed condition must control routing",
    );
  }
  // The secret each guard reads comes from its own binding: with every modeled
  // secret set, a binding to one the scenario does not model reads empty.
  const present = { ...scenario, cratePublish: true, releasePlz: true };
  for (const [name, job] of [
    ["release.yml", "publish-crate"],
    ["release-plz.yml", "release-plz"],
  ]) {
    const run = (input) =>
      runWorkflow(name, input, present).find((row) => row.job === job).enabled;
    assert.equal(run(graph[name]), true);
    const rebound = structuredClone(graph[name]);
    rebound.jobs.guard.steps[0].env.TOKEN = `\${{ secrets.UNMODELED_TOKEN }}`;
    assert.equal(run(rebound), false, `${name} guard ignored its binding`);
  }
});
// llmlint: ignore-end[shell_test_tiers_stay_split]
