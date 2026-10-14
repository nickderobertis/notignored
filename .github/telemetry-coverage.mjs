// Validate the recorder's coverage inventory, never its job conditions or costs.
import { execFileSync } from "node:child_process";
import { isDeepStrictEqual } from "node:util";
import { deliveries } from "./event-classes.mjs";
import {
  calledWorkflow,
  expand,
  resolve,
  triggered,
  workflows,
} from "./workflow-graph.mjs";

export function coverageValidator(root = ".") {
  const graph = workflows(root);
  const inventoryCache = new Map();
  let arches;
  function inventory(name, workflow, stack = []) {
    if (stack.includes(name))
      throw new Error(`recursive coverage call: ${name}`);
    if (inventoryCache.has(name)) return inventoryCache.get(name);
    const rows = [];
    for (const [id, job] of Object.entries(workflow.jobs)) {
      if (job.uses) {
        const called = calledWorkflow(job.uses, root);
        rows.push(
          ...inventory(job.uses, called, [...stack, name]).map((row) => ({
            ...row,
            call: `${id}/${row.call ?? row.job}`,
          })),
        );
      } else {
        const matrix = job.strategy?.matrix ?? {};
        let context = {};
        if (JSON.stringify(matrix).includes("${{")) {
          if (
            name !==
              "nickderobertis/screencomp/.github/workflows/visual-docs-reusable.yml@v0.4.2" ||
            Object.keys(matrix).join() !== "arch" ||
            matrix.arch !== `\${{ fromJSON(needs.arches.outputs.matrix) }}`
          )
            throw new Error(`${name}/${id}: unresolved coverage matrix`);
          // Read the declaration; this does not run the workflow's arches step.
          arches ??= JSON.parse(
            execFileSync(
              "python3",
              [
                "-c",
                'import tomllib,json,sys; print(json.dumps(tomllib.load(open(sys.argv[1],"rb"))["capture"]["arches"]))',
                `${root}/screencomp.toml`,
              ],
              { encoding: "utf8", stdio: "pipe" },
            ),
          );
          if (
            !Array.isArray(arches) ||
            !arches.length ||
            arches.some((arch) => typeof arch !== "string")
          )
            throw new Error("unresolved coverage architectures");
          context = {
            needs: { arches: { outputs: { matrix: JSON.stringify(arches) } } },
          };
        }
        rows.push(
          ...expand(matrix, context).map((matrix) => {
            const runner = resolve(job["runs-on"], { ...context, matrix });
            return {
              job: id,
              matrix,
              runner: Array.isArray(runner) ? runner : [runner],
              target:
                matrix.target ?? matrix.arch ?? job.env?.RELEASE_TARGETS ?? "",
            };
          }),
        );
      }
    }
    inventoryCache.set(name, rows);
    return rows;
  }
  const key = (row) => ({
    job: row.job,
    matrix: row.matrix,
    runner: row.runner,
    target: row.target,
    ...(row.call ? { call: row.call } : {}),
  });
  const sorted = (rows) =>
    rows
      .map(key)
      .sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
  return (scenario, classId) => {
    // Event subscriptions and their declared downstream calls define which
    // workflow inventories must be present, including jobs intentionally skipped.
    const expected = [];
    const pending = deliveries(scenario, classId);
    const seen = new Set();
    for (let i = 0; i < pending.length; i++) {
      const event = pending[i];
      for (const [name, workflow] of Object.entries(graph)) {
        if (!triggered(workflow, event)) continue;
        const identity = `${name}/${event.event}/${event.ref}`;
        if (seen.has(identity))
          throw new Error(`duplicate coverage workflow: ${identity}`);
        seen.add(identity);
        expected.push({ workflow: name, event: event.event, ref: event.ref });
        pending.push({
          ...event,
          event: "workflow_run",
          action: "completed",
          workflow: workflow.name,
        });
      }
    }
    const identities = (runs) =>
      runs
        .map(({ workflow, event, ref }) => `${workflow}/${event}/${ref}`)
        .sort();
    if (!isDeepStrictEqual(identities(scenario.runs), identities(expected)))
      throw new Error(`incomplete workflow coverage: ${scenario.id}`);
    for (const run of scenario.runs) {
      if (
        !isDeepStrictEqual(
          sorted(run.rows),
          sorted(inventory(run.workflow, graph[run.workflow])),
        )
      )
        throw new Error(
          `incomplete job coverage: ${scenario.id}/${run.workflow}`,
        );
    }
  };
}
