export const eventClasses = [
  ["pr-opened", "pull_request", "opened", "refs/pull/1/merge"],
  ["pr-synchronize", "pull_request", "synchronize", "refs/pull/1/merge"],
  ["pr-reopened", "pull_request", "reopened", "refs/pull/1/merge"],
  ["pr-ready-for-review", "pull_request", "ready_for_review", "refs/pull/1/merge"],
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
// The event class whose recorded scenario each lifecycle event must reproduce.
export const lifecycleSources = {
  "ordinary-pr-push": "pr-synchronize",
  "ordinary-base-merge": "base-push",
  "version-only-release-pr": "pr-opened",
  "release-base-merge": "release-merge",
  "release-tag": "release-tag",
  "release-published": "release-published",
  "automatic-smoke": "downstream",
  schedule: "scheduled",
};
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

// Merging a pull request into the base branch is two deliveries: the push to
// main and the merged pull request's `closed` event.
const mergedClasses = new Set(["base-push", "release-merge"]);
export const mergeClosed = {
  event: "pull_request",
  action: "closed",
  ref: "refs/pull/1/merge",
};
export function deliveries(scenario, classId) {
  const root = {
    event: scenario.event,
    action: scenario.action,
    ref: scenario.ref,
    workflow: scenario.workflow,
  };
  return mergedClasses.has(classId) ? [root, mergeClosed] : [root];
}

// GitHub's documented default activity types for the subscriptions whose
// unlisted types still filter what fires.
const defaultTypes = {
  pull_request: ["opened", "synchronize", "reopened"],
  pull_request_target: ["opened", "synchronize", "reopened"],
  workflow_run: ["completed", "requested"],
};
export function subscribedTypes(event, config) {
  if (config?.types !== undefined) return config.types;
  return defaultTypes[event];
}

// Every event and activity type a workflow subscribes to must be delivered by
// some event class, or the measured workload silently omits it.
export function uncoveredSubscriptions(graph) {
  const delivered = eventClasses.flatMap(([id, event, action, ref]) =>
    deliveries({ event, action, ref }, id),
  );
  const missing = [];
  for (const [name, workflow] of Object.entries(graph)) {
    const on =
      typeof workflow.on === "string"
        ? { [workflow.on]: null }
        : Array.isArray(workflow.on)
          ? Object.fromEntries(workflow.on.map((event) => [event, null]))
          : (workflow.on ?? {});
    for (const [event, config] of Object.entries(on)) {
      if (event === "workflow_call") continue;
      const types = subscribedTypes(event, config);
      for (const action of types ?? [undefined])
        if (
          !delivered.some(
            (d) =>
              d.event === event &&
              (action === undefined || d.action === action),
          )
        )
          missing.push(`${name}: ${event}${action ? `.${action}` : ""}`);
    }
  }
  return missing;
}
