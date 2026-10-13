// Private, repository-specific execution model. Unsupported routing is an error.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { data, Evaluator, Lexer, Parser } from "@actions/expressions";
import { parse } from "yaml";

const archCache = new Map();
const yamlCache = new Map();
function yamlFile(path) {
  const text = readFileSync(path, "utf8");
  const previous = yamlCache.get(path);
  if (previous?.text === text) return previous.value;
  const value = parse(text);
  yamlCache.set(path, { text, value });
  return value;
}
const reusable =
  "nickderobertis/screencomp/.github/workflows/visual-docs-reusable.yml@v0.4.2";
export function workflows(root = ".") {
  return Object.fromEntries(
    readdirSync(`${root}/.github/workflows`)
      .filter((p) => /\.ya?ml$/.test(p))
      .sort()
      .map((p) => [p, yamlFile(`${root}/.github/workflows/${p}`)]),
  );
}
function datum(value) {
  if (value === null || value === undefined) return new data.Null();
  if (Array.isArray(value)) return new data.Array(...value.map(datum));
  if (typeof value === "object")
    return new data.Dictionary(
      ...Object.entries(value).map(([key, value]) => ({
        key,
        value: datum(value),
      })),
    );
  if (typeof value === "boolean") return new data.BooleanData(value);
  if (typeof value === "number") return new data.NumberData(value);
  return new data.StringData(value);
}
export function evaluate(expression, context, results = []) {
  const functions = new Map(
    ["always", "success", "failure", "cancelled"].map((name) => [
      name,
      {
        name,
        minArgs: 0,
        maxArgs: 0,
        call: () =>
          datum(
            name === "always" ||
              (name === "success" && results.every((r) => r === "success")) ||
              (name === "failure" && results.includes("failure")) ||
              (name === "cancelled" && results.includes("cancelled")),
          ),
      },
    ]),
  );
  const source = String(expression).replace(/^\$\{\{\s*|\s*\}\}$/g, "");
  const ast = new Parser(new Lexer(source).lex().tokens, Object.keys(context), [
    ...functions.values(),
  ]).parse();
  return new Evaluator(ast, datum(context), functions).evaluate();
}
function resolve(value, context) {
  if (typeof value !== "string") return value;
  if (/^\$\{\{[\s\S]*\}\}$/.test(value)) {
    const result = evaluate(value, context);
    return JSON.parse(JSON.stringify(result, data.replacer));
  }
  return value.replace(/\$\{\{([\s\S]*?)\}\}/g, (_, expr) =>
    evaluate(expr, context).coerceString(),
  );
}
export function expand(matrix = {}, context = {}) {
  if (typeof matrix === "string") matrix = resolve(matrix, context);
  if (!matrix || typeof matrix !== "object" || Array.isArray(matrix))
    throw new Error("unresolved matrix");
  if (
    Object.keys(matrix).every((k) => ["include", "exclude"].includes(k)) &&
    matrix.include?.length
  )
    return matrix.include;
  let rows = [{}];
  for (const [key, raw] of Object.entries(matrix)) {
    if (["include", "exclude"].includes(key)) continue;
    const values = resolve(raw, context);
    if (!Array.isArray(values) || !values.length)
      throw new Error(`unresolved matrix axis: ${key}`);
    rows = rows.flatMap((row) =>
      values.map((value) => ({ ...row, [key]: value })),
    );
  }
  const retained = rows.filter(
    (row) =>
      !(matrix.exclude ?? []).some((ex) =>
        Object.entries(ex).every(([key, value]) => row[key] === value),
      ),
  );
  const expanded = retained.map((original) => ({
    original,
    value: { ...original },
  }));
  const added = [];
  for (const inc of matrix.include ?? []) {
    let matched = false;
    for (const entry of expanded) {
      if (
        Object.entries(inc).every(
          ([key, value]) =>
            !(key in entry.original) || entry.original[key] === value,
        )
      ) {
        entry.value = { ...entry.value, ...inc };
        matched = true;
      }
    }
    if (!matched) added.push(inc);
  }
  rows = [...expanded.map((entry) => entry.value), ...added];
  return rows;
}
const matches = (pattern, value) =>
  new RegExp(
    `^${pattern
      .replace(/[.+^${}()|[\]\\]/g, "\\$&")
      .replaceAll("*", ".*")
      .replaceAll("?", ".")}$`,
  ).test(value);
function filter(patterns, value) {
  let allowed = false;
  for (const pattern of patterns) {
    if (matches(pattern.replace(/^!/, ""), value))
      allowed = !pattern.startsWith("!");
  }
  return allowed;
}
export function triggered(workflow, scenario) {
  const on =
    typeof workflow.on === "string"
      ? { [workflow.on]: null }
      : Array.isArray(workflow.on)
        ? Object.fromEntries(workflow.on.map((e) => [e, null]))
        : workflow.on;
  if (!on || !Object.hasOwn(on, scenario.event)) return false;
  const config = on[scenario.event];
  if (!config) return true;
  if (config.types && !config.types.includes(scenario.action)) return false;
  if (
    scenario.event === "workflow_run" &&
    config.workflows &&
    !config.workflows.includes(scenario.workflow)
  )
    return false;
  if (scenario.event === "push") {
    const tag = scenario.ref.startsWith("refs/tags/");
    if (tag && config.branches && !config.tags) return false;
    if (!tag && config.tags && !config.branches) return false;
    const patterns = tag ? config.tags : config.branches;
    if (
      patterns &&
      !filter(patterns, scenario.ref.replace(/^refs\/(heads|tags)\//, ""))
    )
      return false;
  }
  if (
    config.paths ||
    config["paths-ignore"] ||
    config["branches-ignore"] ||
    config["tags-ignore"]
  )
    throw new Error("unsupported trigger filter; resolve explicitly");
  return true;
}
function defaults(workflow) {
  return Object.fromEntries(
    Object.entries(workflow.on?.workflow_call?.inputs ?? {}).map(
      ([key, spec]) => [
        key,
        spec.default ?? (spec.type === "boolean" ? false : ""),
      ],
    ),
  );
}
export function runWorkflow(name, workflow, scenario, root = ".", stack = []) {
  if (stack.includes(name)) throw new Error(`recursive workflow call: ${name}`);
  const rows = [],
    completed = {};
  const github = {
    event_name: scenario.event,
    event: {
      action: scenario.action,
      pull_request: scenario.pr,
      head_commit: { message: scenario.message ?? "" },
    },
    repository: "nickderobertis/notignored",
    ref: scenario.ref,
    head_ref: scenario.pr?.head?.ref ?? "",
    workflow: scenario.workflow,
  };
  const pending = new Set(Object.keys(workflow.jobs));
  while (pending.size) {
    let progress = false;
    for (const id of pending) {
      const job = workflow.jobs[id];
      const dependencies =
        typeof job.needs === "string" ? [job.needs] : (job.needs ?? []);
      if (dependencies.some((n) => !Object.hasOwn(workflow.jobs, n)))
        throw new Error(`${name}/${id}: missing needs`);
      if (dependencies.some((n) => !completed[n])) continue;
      const needs = Object.fromEntries(
        dependencies.map((n) => [n, completed[n]]),
      );
      const context = {
        github,
        needs,
        inputs: scenario.inputs ?? {},
        vars: scenario.vars ?? {},
        matrix: {},
        secrets: {},
      };
      const results = dependencies.map((n) => completed[n].result);
      const condition = job.if ?? "success()";
      const explicitStatus = /\b(always|success|failure|cancelled)\s*\(/.test(
        condition,
      );
      const enabled =
        (explicitStatus || results.every((r) => r === "success")) &&
        evaluate(condition, context, results).coerceString() === "true";
      if (job.uses) {
        let called;
        if (job.uses.startsWith("./.github/workflows/"))
          called = yamlFile(`${root}/${job.uses.slice(2)}`);
        else if (job.uses === reusable) {
          const lock = JSON.parse(
            readFileSync(`${root}/.github/fixtures/reusable/lock.json`, "utf8"),
          );
          called = yamlFile(
            `${root}/.github/fixtures/reusable/screencomp-v0.4.2.yml`,
          );
          const source = JSON.parse(
            readFileSync(
              `${root}/.github/fixtures/reusable/source.json`,
              "utf8",
            ),
          );
          const sourceDigest = createHash("sha256")
            .update(source.source)
            .digest("hex");
          const sourceSemantic = createHash("sha256")
            .update(JSON.stringify(parse(source.source)))
            .digest("hex");
          if (
            source.commit !== lock.commit ||
            source.url !==
              `https://raw.githubusercontent.com/nickderobertis/screencomp/${lock.commit}/.github/workflows/visual-docs-reusable.yml` ||
            sourceDigest !== lock.sourceSha256 ||
            sourceSemantic !== lock.semanticSha256
          )
            throw new Error(
              "unresolved reusable workflow: source provenance differs",
            );
          const digest = createHash("sha256")
            .update(JSON.stringify(called))
            .digest("hex");
          if (
            lock.call !== job.uses ||
            !/^[0-9a-f]{40}$/.test(lock.commit) ||
            !/^[0-9a-f]{64}$/.test(lock.sourceSha256) ||
            digest !== lock.semanticSha256
          )
            throw new Error(
              "unresolved reusable workflow: locked provenance differs",
            );
        } else throw new Error(`unresolved reusable workflow: ${job.uses}`);
        const inputs = {
          ...defaults(called),
          ...Object.fromEntries(
            Object.entries(job.with ?? {}).map(([k, v]) => [
              k,
              resolve(v, context),
            ]),
          ),
        };
        const nested = runWorkflow(
          job.uses,
          called,
          { ...scenario, inputs },
          root,
          [...stack, name],
        );
        rows.push(
          ...nested.map((row) => ({
            ...row,
            workflow: name,
            call: `${id}/${row.call ?? row.job}`,
            enabled: enabled && row.enabled,
          })),
        );
      } else {
        for (const matrix of expand(job.strategy?.matrix, context)) {
          const runner = resolve(job["runs-on"], { ...context, matrix });
          const labels = Array.isArray(runner) ? runner : [runner];
          if (
            !labels.length ||
            labels.some(
              (label) =>
                typeof label !== "string" || !label || label.includes("${{"),
            )
          )
            throw new Error(
              `${name}/${id}: unresolved runner ${JSON.stringify(runner)}`,
            );
          if (!labels.some((label) => /^(ubuntu|windows|macos)-/.test(label)))
            throw new Error(
              `${name}/${id}: unclassified runner ${JSON.stringify(runner)}`,
            );
          rows.push({
            workflow: name,
            job: id,
            display: resolve(job.name ?? id, { ...context, matrix }),
            target:
              matrix.target ?? matrix.arch ?? job.env?.RELEASE_TARGETS ?? "",
            matrix,
            runner: labels,
            macos: labels.some((label) => label.startsWith("macos-")),
            enabled,
          });
        }
      }
      let outputs = {};
      if (id === "changes" && name === "ci.yml")
        outputs = {
          crate: String(scenario.crate ?? true),
          macos: String(
            scenario.policy.macosRequired && scenario.event === "pull_request",
          ),
        };
      else if (id === "guard" && name === "release.yml")
        outputs = { publish: String(scenario.cratePublish ?? true) };
      else if (id === "guard" && name === "release-plz.yml")
        outputs = { enabled: "true" };
      else if (id === "arches" && name === reusable) {
        const config = readFileSync(`${root}/screencomp.toml`, "utf8");
        let arches = archCache.get(config);
        if (!arches) {
          arches = JSON.parse(
            execFileSync(
              "python3",
              [
                "-c",
                'import tomllib,json,sys; print(json.dumps(tomllib.load(open(sys.argv[1],"rb"))["capture"]["arches"]))',
                `${root}/screencomp.toml`,
              ],
              { encoding: "utf8" },
            ),
          );
          archCache.set(config, arches);
        }
        if (context.inputs.arches) {
          arches = JSON.parse(context.inputs.arches);
          if (
            !Array.isArray(arches) ||
            !arches.length ||
            arches.some((arch) => typeof arch !== "string")
          )
            throw new Error("unresolved reusable arch override");
        }
        outputs.matrix = JSON.stringify(arches);
      } else if (job.outputs)
        throw new Error(`${name}/${id}: unresolved job outputs`);
      completed[id] = { result: enabled ? "success" : "skipped", outputs };
      pending.delete(id);
      progress = true;
    }
    if (!progress) throw new Error(`${name}: cyclic needs`);
  }
  return rows;
}
export function eventRuns(scenario, root = ".") {
  const graph = workflows(root);
  const runs = [];
  for (const [name, workflow] of Object.entries(graph)) {
    if (triggered(workflow, scenario))
      runs.push({
        workflow: name,
        event: scenario.event,
        scenario: scenario.id,
        ref: scenario.ref,
        rows: runWorkflow(name, workflow, scenario, root),
      });
  }
  return runs;
}
export function fanout(scenario, root = ".") {
  const runs = eventRuns(scenario, root);
  const seen = new Set();
  for (let i = 0; i < runs.length; i++) {
    const run = runs[i];
    const key = `${run.workflow}:${run.event}:${run.ref}`;
    if (seen.has(key))
      throw new Error(`automatic workflow fan-out cycle: ${key}`);
    seen.add(key);
    runs.push(
      ...eventRuns(
        {
          ...scenario,
          id: `${scenario.id}/completed/${run.workflow}`,
          event: "workflow_run",
          action: "completed",
          workflow: workflows(root)[run.workflow].name,
        },
        root,
      ),
    );
    // The release publication uses GITHUB_TOKEN (verified by release contracts),
    // which does not trigger Actions. Explicit release events are still modeled.
  }
  return runs;
}
export function sourceDigest(root = ".") {
  const files = execFileSync(
    "git",
    [
      "ls-files",
      "--cached",
      "--others",
      "--exclude-standard",
      ".github",
      "scripts/ci-required.sh",
      "scripts/ci-gate-tier.sh",
      "scripts/stage-release.py",
      "scripts/stage-release.sh",
      "screencomp.toml",
      "Cargo.toml",
      "Cargo.lock",
      "pyproject.toml",
      "scripts/npm-build.mjs",
      "npm/notignored/package.json",
      ".maturin-version",
    ],
    { cwd: root, encoding: "utf8" },
  )
    .trim()
    .split("\n")
    .sort()
    .filter((path) => existsSync(`${root}/${path}`));
  const hash = createHash("sha256");
  for (const path of files)
    hash
      .update(path)
      .update("\0")
      .update(readFileSync(`${root}/${path}`));
  return hash.digest("hex");
}
