import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parse } from "yaml";
import { workflows } from "../workflow-graph.mjs";

test("both Darwin architectures retain exact-run artifacts and pre-publication native installers", () => {
  const release = workflows()["release.yml"].jobs;
  const mac = release["macos-build"];
  const darwin = mac.env.RELEASE_TARGETS.split(/\s+/).sort();
  const baseline = parse(
    readFileSync(".github/fixtures/baseline/release.yml", "utf8"),
  );
  assert.deepEqual(
    darwin,
    baseline.jobs.upload.strategy.matrix.include
      .map((row) => row.target)
      .filter((target) => target.endsWith("apple-darwin"))
      .sort(),
  );
  const run = mac.steps.map((step) => step.run ?? "").join("\n");
  for (const target of darwin)
    assert.ok(run.includes(`scripts/stage-release.sh staged/${target}`));
  assert.match(run, /arch -x86_64 \/bin\/bash scripts\/stage-release.sh/);
  assert.match(run, /uv python find cpython-3.12-macos-x86_64-none/);
  assert.match(run, /softwareupdate --install-rosetta/);
  assert.ok(mac.steps.some((step) => step.with?.architecture === "x64"));
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
