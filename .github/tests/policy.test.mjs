// llmlint: ignore-file[shell_test_tiers_stay_split] The task explicitly requires subprocess git-history journeys in github-workflows:test. These local git repositories are the classifier trust boundary, offline and deterministic, and introduce no paid probe or packaging build.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { parse } from "yaml";

const policy = resolve(".github/macos-policy.mjs");
export function history() {
  const dir = mkdtempSync(
    join(
      process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(),
      "macos-history-",
    ),
  );
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: dir,
      encoding: "utf8",
      stdio: ["pipe", "pipe", "pipe"],
    }).trim();
  git("init", "-q");
  git("config", "user.name", "Policy journey");
  git("config", "user.email", "policy@example.invalid");
  writeFileSync(
    join(dir, "Cargo.toml"),
    '[package]\nname = "notignored"\nversion = "0.1.0"\n[dependencies]\nserde = "1"\n',
  );
  writeFileSync(
    join(dir, "Cargo.lock"),
    'version = 4\n[[package]]\nname = "notignored"\nversion = "0.1.0"\n[[package]]\nname = "serde"\nversion = "1.0.0"\n',
  );
  writeFileSync(join(dir, "CHANGELOG.md"), "## [0.1.0]\nFirst release.\n");
  writeFileSync(join(dir, "src.rs"), "fn main() {}\n");
  mkdirSync(join(dir, "npm/notignored"), { recursive: true });
  writeFileSync(
    join(dir, "npm/notignored/package.json"),
    JSON.stringify({
      name: "notignored-cli",
      version: "0.1.0",
      description: "CLI",
      optionalDependencies: { "notignored-cli-darwin-x64": "0.1.0" },
    }),
  );
  const extra = {
    "pyproject.toml":
      '[project]\nname="notignored-cli"\nversion="0.1.0"\ndescription="CLI"\n',
    "python/notignored-sdk/pyproject.toml":
      '[project]\nname="notignored-sdk"\nversion="0.1.0"\ndependencies=["notignored-cli==0.1.0", "serde>=1"]\n',
    "python/notignored-sdk/uv.lock":
      'version=1\n[[package]]\nname="notignored-sdk"\nversion="0.1.0"\n[[package]]\nname="serde"\nversion="1.0.0"\n',
    "package.json": JSON.stringify({
      name: "notignored",
      version: "0.1.0",
      description: "CLI",
    }),
    "package-lock.json": JSON.stringify({
      name: "notignored",
      version: "0.1.0",
      lockfileVersion: 3,
      packages: {
        "": { name: "notignored", version: "0.1.0" },
        "node_modules/serde": { name: "serde", version: "1.0.0" },
      },
    }),
    "npm/notignored-sdk/package.json": JSON.stringify({
      name: "notignored-sdk",
      version: "0.1.0",
      description: "CLI",
    }),
  };
  for (const [path, text] of Object.entries(extra)) {
    mkdirSync(join(dir, path, ".."), { recursive: true });
    writeFileSync(join(dir, path), text);
  }
  git("add", ".");
  git("commit", "-qm", "initial");
  const base = git("rev-parse", "HEAD");
  const heads = {};
  for (const kind of ["version", "source", "dependency", "metadata"]) {
    git("checkout", "-q", base);
    if (kind === "source")
      writeFileSync(join(dir, "src.rs"), 'fn main() { panic!("edited"); }\n');
    else if (kind === "dependency")
      writeFileSync(
        join(dir, "Cargo.toml"),
        '[package]\nname = "notignored"\nversion = "0.2.0"\n[dependencies]\nserde = "2"\n',
      );
    else if (kind === "metadata")
      writeFileSync(
        join(dir, "npm/notignored/package.json"),
        JSON.stringify({
          name: "notignored-cli",
          version: "0.2.0",
          description: "changed content",
        }),
      );
    else {
      for (const [path, text] of Object.entries(extra))
        writeFileSync(join(dir, path), text.replaceAll("0.1.0", "0.2.0"));
      writeFileSync(
        join(dir, "Cargo.toml"),
        '[package]\nname = "notignored"\nversion = "0.2.0"\n[dependencies]\nserde = "1"\n',
      );
      writeFileSync(
        join(dir, "Cargo.lock"),
        'version = 4\n[[package]]\nname = "notignored"\nversion = "0.2.0"\n[[package]]\nname = "serde"\nversion = "1.0.0"\n',
      );
      writeFileSync(
        join(dir, "CHANGELOG.md"),
        "## [0.2.0]\nNext release.\n## [0.1.0]\nFirst release.\n",
      );
      writeFileSync(
        join(dir, "npm/notignored/package.json"),
        JSON.stringify({
          name: "notignored-cli",
          version: "0.2.0",
          description: "CLI",
          optionalDependencies: { "notignored-cli-darwin-x64": "0.2.0" },
        }),
      );
    }
    git("add", ".");
    git("commit", "-qm", kind);
    heads[kind] = git("rev-parse", "HEAD");
  }
  return {
    dir,
    base,
    heads,
    git,
    close: () => rmSync(dir, { recursive: true, force: true }),
  };
}
export function payload(branch = "release-plz-2026-10-13", fork = false) {
  const repo = { full_name: "nickderobertis/notignored" };
  return {
    repository: repo,
    pull_request: {
      title: "chore: release",
      user: { login: "nickderobertis" },
      head: {
        ref: branch,
        repo: fork ? { full_name: "outside/notignored" } : repo,
      },
      base: { repo },
    },
  };
}
export function classifyHistory(
  h,
  event,
  head = h.heads.version,
  base = h.base,
) {
  const path = join(h.dir, "event.json");
  writeFileSync(
    path,
    typeof event === "string" ? event : JSON.stringify(event),
  );
  return JSON.parse(
    execFileSync(
      "node",
      [policy, "classify", "--event", path, "--base", base, "--head", head],
      { cwd: h.dir, encoding: "utf8" },
    ),
  );
}
test("real classifier over git histories admits only same-repository semantic release diffs", () => {
  const h = history();
  try {
    assert.equal(classifyHistory(h, payload()).isReleaseOnly, true);
    for (const event of [
      payload("feature"),
      payload(undefined, true),
      {
        ...payload("feature"),
        pull_request: {
          ...payload("feature").pull_request,
          user: { login: "github-actions[bot]" },
        },
      },
    ])
      assert.equal(classifyHistory(h, event).macosRequired, true);
    for (const kind of ["source", "dependency", "metadata"])
      assert.equal(
        classifyHistory(h, payload(), h.heads[kind]).macosRequired,
        true,
        kind,
      );
    for (const event of ["{", {}, null])
      assert.equal(classifyHistory(h, event).macosRequired, true);
    assert.equal(
      classifyHistory(h, payload(), h.heads.version, "missing").macosRequired,
      true,
    );
    assert.equal(classifyHistory(h, payload(), h.base).macosRequired, true);
    assert.equal(
      classifyHistory(h, { event_name: "schedule" }).macosRequired,
      false,
    );
  } finally {
    h.close();
  }
});

test("all admitted manifest forms reject non-version content and added/deleted/renamed files", () => {
  const h = history();
  try {
    for (const [path, change] of [
      ["pyproject.toml", (text) => text.replace("CLI", "changed")],
      [
        "python/notignored-sdk/pyproject.toml",
        (text) => text.replace("serde>=1", "serde>=2"),
      ],
      [
        "python/notignored-sdk/uv.lock",
        (text) => text.replace("1.0.0", "2.0.0"),
      ],
      ["package.json", (text) => text.replace("CLI", "changed")],
      ["package-lock.json", (text) => text.replace("1.0.0", "2.0.0")],
      [
        "npm/notignored-sdk/package.json",
        (text) => text.replace("CLI", "changed"),
      ],
      ["Cargo.lock", (text) => text.replace("1.0.0", "2.0.0")],
      ["npm/notignored/package.json", () => "{"],
    ]) {
      h.git("checkout", "-q", h.base);
      writeFileSync(
        join(h.dir, path),
        change(readFileSync(join(h.dir, path), "utf8")),
      );
      h.git("add", "-u");
      h.git("commit", "-qm", "non-version content");
      assert.equal(
        classifyHistory(h, payload(), h.git("rev-parse", "HEAD")).macosRequired,
        true,
        path,
      );
    }
    for (const mutation of ["added", "deleted", "renamed"]) {
      h.git("checkout", "-q", h.base);
      if (mutation === "added") {
        writeFileSync(join(h.dir, "added.rs"), "fn main(){}");
        h.git("add", "added.rs");
      } else if (mutation === "deleted") rmSync(join(h.dir, "src.rs"));
      else renameSync(join(h.dir, "src.rs"), join(h.dir, "renamed.rs"));
      if (mutation === "renamed") h.git("add", "renamed.rs");
      h.git("add", "-u");
      h.git("commit", "-qm", mutation);
      assert.equal(
        classifyHistory(h, payload(), h.git("rev-parse", "HEAD")).macosRequired,
        true,
        mutation,
      );
    }
  } finally {
    h.close();
  }
});

export function workflowClassify(
  h,
  event,
  head = h.heads.version,
  eventName = "pull_request",
) {
  const path = join(h.dir, "event.json");
  writeFileSync(
    path,
    typeof event === "string" ? event : JSON.stringify(event),
  );
  mkdirSync(join(h.dir, ".github"), { recursive: true });
  writeFileSync(join(h.dir, ".github/macos-policy.mjs"), readFileSync(policy));
  const workflow = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
  const step = workflow.jobs.changes.steps.find((step) => step.id === "policy");
  const output = join(h.dir, "workflow-output");
  writeFileSync(output, "");
  execFileSync("bash", ["-e", "-o", "pipefail", "-c", step.run], {
    cwd: h.dir,
    encoding: "utf8",
    env: {
      ...process.env,
      GITHUB_EVENT_PATH: path,
      GITHUB_OUTPUT: output,
      RUNNER_TEMP: h.dir,
      POLICY_EVENT: eventName,
      POLICY_BASE: h.base,
      POLICY_HEAD: head,
    },
  });
  return Object.fromEntries(
    readFileSync(output, "utf8")
      .trim()
      .split("\n")
      .map((line) => line.split("=")),
  );
}
test("actual changes policy step agrees with the classifier, including unknown PRs and non-PR events", () => {
  const h = history();
  try {
    for (const event of [
      payload(),
      payload("feature"),
      payload(undefined, true),
      {},
      "{",
    ]) {
      assert.equal(
        workflowClassify(h, event).macos,
        String(classifyHistory(h, event).macosRequired),
      );
    }
    assert.equal(
      workflowClassify(h, {}, h.heads.source, "push").macos,
      "false",
    );
    assert.equal(
      workflowClassify(h, payload(), h.heads.dependency).macos,
      "true",
    );
  } finally {
    h.close();
  }
});
