// Event workload shared by the graph recorder and telemetry validator.
export const eventClasses = [
  ["pr-opened", "pull_request", "opened", "refs/pull/1/merge"],
  ["pr-synchronize", "pull_request", "synchronize", "refs/pull/1/merge"],
  ["pr-reopened", "pull_request", "reopened", "refs/pull/1/merge"],
  ["base-push", "push", "", "refs/heads/main"],
  ["release-merge", "push", "", "refs/heads/main"],
  ["release-tag", "push", "", "refs/tags/v0.2.0"],
  ["release-published", "release", "published", "refs/tags/v0.2.0"],
  ["downstream", "workflow_run", "completed", "refs/heads/main"],
  ["dispatch", "repository_dispatch", "", "refs/heads/main"],
  ["scheduled", "schedule", "", "refs/heads/main"],
  ["manual-default", "workflow_dispatch", "", "refs/heads/main"],
  ["manual-opt-in", "workflow_dispatch", "", "refs/heads/main"],
];

export const diffClasses = ["source", "version", "dependency"];
export const ownershipClasses = [false, true];
export const publishChoices = [false, true];
export function variantKey({ diff, fork, vars, cratePublish }) {
  return `${diff}/${fork ? "fork" : "same"}/${vars.PYPI_PUBLISH}/${vars.NPM_PUBLISH}/${cratePublish}`;
}
export function variants() {
  return publishChoices.flatMap((cratePublish) =>
    diffClasses.flatMap((diff) =>
      ownershipClasses.flatMap((fork) =>
        publishChoices.flatMap((pypi) =>
          publishChoices.map((npm) => ({
            diff,
            fork,
            cratePublish,
            vars: { PYPI_PUBLISH: String(pypi), NPM_PUBLISH: String(npm) },
          })),
        ),
      ),
    ),
  );
}
export const lifecycleEvents = [
  "ordinary-pr-push",
  "ordinary-base-merge",
  "version-only-release-pr",
  "release-base-merge",
  "release-tag",
  "release-published",
  "automatic-smoke",
  "schedule",
];
