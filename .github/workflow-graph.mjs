// Private, repository-specific execution model. Unsupported routing is an error.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  readdirSync,
  readFileSync,
  mkdtempSync,
  rmSync,
} from "node:fs";
import { data, Evaluator, Lexer, Parser } from "@actions/expressions";
import { parse } from "yaml";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { subscribedTypes } from "./event-classes.mjs";

const guardCache = new Map();
function guardOutputs(job, tokenPresent, root) {
  const step = job.steps.find((step) => step.id === "check");
  if (!step?.run || Object.keys(step.env ?? {}).join() !== "TOKEN")
    throw new Error("unresolved guard command or environment");
  const key = JSON.stringify([step.run, job.outputs, tokenPresent]);
  if (guardCache.has(key)) return guardCache.get(key);
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(), "macos-guard-"),
  );
  try {
    const output = join(scratch, "outputs");
    execFileSync("bash", ["-e", "-o", "pipefail", "-c", step.run], {
      cwd: root,
      encoding: "utf8",
      stdio: "pipe",
      env: {
        PATH: process.env.PATH,
        TOKEN: tokenPresent ? "present" : "",
        GITHUB_OUTPUT: output,
      },
    });
    const parsed = Object.fromEntries(
      readFileSync(output, "utf8")
        .trim()
        .split("\n")
        .map((line) => line.split("=")),
    );
    const outputs = Object.fromEntries(
      Object.entries(job.outputs).map(([field, expression]) => [
        field,
        resolve(expression, { steps: { check: { outputs: parsed } } }),
      ]),
    );
    if (
      !Object.values(outputs).every((value) =>
        ["true", "false"].includes(value),
      )
    )
      throw new Error("unresolved guard output");
    guardCache.set(key, outputs);
    return outputs;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}
const hostedRunners = new Set([
  "ubuntu-latest",
  "ubuntu-24.04-arm",
  "windows-latest",
  "macos-latest",
]);
const archResolutionScript =
  'set -euo pipefail\nif [ -n "$ARCHES_OVERRIDE" ]; then\n  matrix="$ARCHES_OVERRIDE"   # explicit override (no committed config to read)\nelse\n  matrix="$(screencomp arches --format json)"\nfi\nif [ -z "$matrix" ] || [ "$matrix" = "[]" ]; then\n  echo "::error::no [capture].arches configured in screencomp.toml; add e.g. arches = [\\"x86_64\\"], or pass the workflow\'s \'arches\' input" >&2\n  exit 1\nfi\necho "matrix=$matrix" >>"$GITHUB_OUTPUT"\n';
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
export function calledWorkflow(uses, root = ".") {
  let called;
  if (uses.startsWith("./.github/workflows/"))
    called = yamlFile(`${root}/${uses.slice(2)}`);
  else if (uses === reusable) {
    const lock = JSON.parse(
      readFileSync(`${root}/.github/fixtures/reusable/lock.json`, "utf8"),
    );
    called = yamlFile(
      `${root}/.github/fixtures/reusable/screencomp-v0.4.2.yml`,
    );
    const source = JSON.parse(
      readFileSync(`${root}/.github/fixtures/reusable/source.json`, "utf8"),
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
      lock.call !== uses ||
      !/^[0-9a-f]{40}$/.test(lock.commit) ||
      !/^[0-9a-f]{64}$/.test(lock.sourceSha256) ||
      digest !== lock.semanticSha256
    )
      throw new Error(
        "unresolved reusable workflow: locked provenance differs",
      );
  } else throw new Error(`unresolved reusable workflow: ${uses}`);
  return called;
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
export function resolve(value, context) {
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
  const isMapping = (row) =>
    row !== null && typeof row === "object" && !Array.isArray(row);
  for (const key of ["include", "exclude"])
    if (
      Object.hasOwn(matrix, key) &&
      (!Array.isArray(matrix[key]) || !matrix[key].every(isMapping))
    )
      throw new Error(`unresolved matrix ${key}: expected a list of mappings`);
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
function matches(pattern, value) {
  if (/[?+\[\]\\]/.test(pattern))
    throw new Error("unsupported trigger pattern; resolve explicitly");
  return new RegExp(
    `^${pattern
      .replace(/[.+^\${}()|[\]\\]/g, "\\$&")
      .replace(/\*\*|\*/g, (token) => (token === "**" ? ".*" : "[^/]*"))}$`,
  ).test(value);
}

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
  const config = on[scenario.event] ?? {};
  if (scenario.event === "schedule") {
    if (
      !Array.isArray(config) ||
      !config.length ||
      config.some((entry) => typeof entry?.cron !== "string")
    )
      throw new Error("unresolved schedule trigger: expected cron entries");
    return true;
  }
  if (typeof config !== "object" || Array.isArray(config))
    throw new Error(`unresolved ${scenario.event} trigger configuration`);
  for (const key of ["types", "branches", "tags", "workflows"])
    if (
      Object.hasOwn(config, key) &&
      (!Array.isArray(config[key]) ||
        !config[key].length ||
        config[key].some((value) => typeof value !== "string" || !value))
    )
      throw new Error(
        `unresolved ${scenario.event} ${key} filter: expected a list of names`,
      );
  const types = subscribedTypes(scenario.event, config);
  if (types && !types.includes(scenario.action)) return false;
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
  } else if (
    ["pull_request", "pull_request_target"].includes(scenario.event) &&
    config.branches &&
    !config.tags
  ) {
    // Pull request branch filters match the base branch the PR targets.
    const base = scenario.pr?.base?.ref;
    if (typeof base !== "string" || !base)
      throw new Error(
        `${scenario.event} branch filter needs the pull request base ref`,
      );
    if (!filter(config.branches, base)) return false;
  } else if (config.branches || config.tags)
    throw new Error(
      `unsupported ${scenario.event} branch/tag filter; resolve explicitly`,
    );
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
        // A caller matrix would run the called workflow once per row; no caller
        // here has one, so refuse it rather than count a single instance.
        if (job.strategy !== undefined)
          throw new Error(
            `${name}/${id}: unsupported reusable caller strategy`,
          );
        const called = calledWorkflow(job.uses, root);
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
          if (labels.length !== 1 || !hostedRunners.has(labels[0]))
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
        outputs = guardOutputs(job, scenario.cratePublish ?? true, root);
      else if (id === "guard" && name === "release-plz.yml")
        outputs = guardOutputs(job, scenario.releasePlz ?? true, root);
      else if (id === "arches" && name === reusable) {
        if (
          job.steps.find((step) => step.id === "read")?.run !==
          archResolutionScript
        )
          throw new Error(
            "unresolved reusable architecture resolution command",
          );
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
              { encoding: "utf8", stdio: "pipe" },
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
        if (
          !Array.isArray(arches) ||
          !arches.length ||
          arches.some((arch) => typeof arch !== "string")
        )
          throw new Error("unresolved reusable configured architectures");
        outputs = Object.fromEntries(
          Object.entries(job.outputs).map(([field, expression]) => [
            field,
            resolve(expression, {
              steps: { read: { outputs: { matrix: JSON.stringify(arches) } } },
            }),
          ]),
        );
        if (
          !Object.values(outputs).every(
            (value) => typeof value === "string" && value,
          )
        )
          throw new Error("unresolved reusable architecture output mapping");
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
      "scripts/release-version.mjs",
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
