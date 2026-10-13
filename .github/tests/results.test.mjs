import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parse } from "yaml";

const ci = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
function outcomes(event, { macos = "true", crate = "true" } = {}) {
  return Object.fromEntries(
    ci.jobs.required.needs.map((id) => {
      const result =
        (id === "install-documented" && event !== "push") ||
        (["pr-title", "llmlint"].includes(id) && event !== "pull_request") ||
        (id === "macos" && macos === "false") ||
        (["cross", "install", "msrv", "deny"].includes(id) && crate === "false")
          ? "skipped"
          : "success";
      return [
        id,
        { result, outputs: id === "changes" ? { crate, macos } : {} },
      ];
    }),
  );
}
function verdict(needs, event = "pull_request") {
  return spawnSync("bash", ["scripts/ci-required.sh"], {
    encoding: "utf8",
    env: {
      ...process.env,
      REQUIRED_NEEDS: JSON.stringify(needs),
      REQUIRED_EVENT: event,
    },
  });
}
test("production required verdict preserves intentional skips and fails every missing, failed or cancelled proof", () => {
  for (const event of ["pull_request", "push"])
    for (const macos of ["true", "false"])
      for (const crate of ["true", "false"]) {
        const needs = outcomes(event, { macos, crate });
        const passed = verdict(needs, event);
        assert.equal(passed.status, 0, passed.stderr);
        if (macos === "false")
          assert.match(
            passed.stdout,
            /omitted by classifier \(no Mac proof claimed\)/,
          );
        for (const result of ["failure", "cancelled"]) {
          for (const job of ci.jobs.required.needs) {
            const broken = structuredClone(needs);
            broken[job].result = result;
            const failed = verdict(broken, event);
            assert.notEqual(
              failed.status,
              0,
              `${job}/${result} became success`,
            );
            assert.match(failed.stderr, new RegExp(`ci-required: ${job}:`));
          }
        }
      }
  const needs = outcomes("pull_request");
  for (const id of ci.jobs.required.needs) {
    const missing = structuredClone(needs);
    delete missing[id];
    assert.notEqual(verdict(missing).status, 0, `${id} missing became success`);
  }
  needs.macos.result = "skipped";
  assert.notEqual(verdict(needs).status, 0, "ordinary PR Mac omission passed");
  delete needs.changes.outputs.macos;
  assert.notEqual(
    verdict(needs).status,
    0,
    "missing classifier authorized skip",
  );
  needs.changes.result = "failure";
  needs.changes.outputs.macos = "false";
  assert.notEqual(
    verdict(needs).status,
    0,
    "failed prerequisite authorized skip",
  );
});
