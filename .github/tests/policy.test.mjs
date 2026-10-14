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
// A registry crate that happens to share the repository crate's name. Only its
// source tells it apart from the path package this repository builds.
const crates = "registry+https://github.com/rust-lang/crates.io-index";
const registryCrate = `[[package]]\nname = "notignored"\nversion = "0.0.9"\nsource = "${crates}"\n`;
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
    `version = 4\n[[package]]\nname = "notignored"\nversion = "0.1.0"\n${registryCrate}[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "${crates}"\n`,
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
      optionalDependencies: { "notignored-cli-darwin-arm64": "0.0.0-managed" },
    }),
  );
  const extra = {
    "pyproject.toml":
      '[project]\nname="notignored-cli"\nversion="0.1.0"\ndescription="CLI"\n',
    "python/notignored-sdk/pyproject.toml":
      '[project]\nname="notignored-sdk"\nversion="0.1.0"\ndependencies=["notignored-cli==0.0.9", "serde>=1"]\n',
    "python/notignored-sdk/uv.lock":
      'version=1\n[[package]]\nname="notignored-sdk"\nversion="0.1.0"\nsource={editable="."}\n[[package]]\nname="notignored-cli"\nversion="0.0.9"\nsource={registry="https://pypi.org/simple"}\n[[package]]\nname="serde"\nversion="1.0.0"\nsource={registry="https://pypi.org/simple"}\n',
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
        "npm/notignored-sdk": { name: "notignored-sdk", version: "0.1.0" },
        "node_modules/notignored-sdk": {
          resolved: "npm/notignored-sdk",
          link: true,
        },
        "node_modules/cli-alias": {
          name: "notignored-cli",
          version: "0.0.9",
          resolved:
            "https://registry.npmjs.org/notignored-cli/-/notignored-cli-0.0.9.tgz",
        },
        "node_modules/serde": { version: "1.0.0" },
      },
    }),
    "npm/notignored-sdk/package.json": JSON.stringify({
      name: "notignored-sdk",
      version: "0.1.0",
      description: "CLI",
      devDependencies: { "notignored-cli": "0.0.9" },
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
        `version = 4\n[[package]]\nname = "notignored"\nversion = "0.2.0"\n${registryCrate}[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "${crates}"\n`,
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
          optionalDependencies: {
            "notignored-cli-darwin-arm64": "0.0.0-managed",
          },
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
export function payload(
  branch = "release-plz-2026-10-13",
  fork = false,
  draft = false,
) {
  const repo = { full_name: "nickderobertis/notignored" };
  return {
    repository: repo,
    pull_request: {
      draft,
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
  env = {},
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
      {
        cwd: h.dir,
        encoding: "utf8",
        env: { ...process.env, GITHUB_EVENT_NAME: "", ...env },
      },
    ),
  );
}
test("real classifier over git histories admits only same-repository semantic release diffs", () => {
  const h = history();
  try {
    assert.equal(classifyHistory(h, payload()).isReleaseOnly, true);
    for (const event of [{}, "{"])
      assert.equal(
        classifyHistory(h, event, h.heads.source, h.base, {
          GITHUB_EVENT_NAME: "push",
        }).macosRequired,
        false,
      );
    assert.equal(
      classifyHistory(
        h,
        { ...payload("feature"), event_name: "schedule" },
        h.heads.source,
        h.base,
        { GITHUB_EVENT_NAME: "pull_request" },
      ).macosRequired,
      true,
    );
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
    for (const name of [
      "push",
      "release",
      "workflow_run",
      "repository_dispatch",
      "schedule",
      "workflow_dispatch",
    ]) {
      const known = classifyHistory(h, {}, h.heads.source, h.base, {
        GITHUB_EVENT_NAME: name,
      });
      assert.equal(known.macosRequired, false, name);
      assert.match(known.reason, new RegExp(`non-PR event: ${name}`));
    }
    for (const name of ["merge_group", "pull_request_target", "invented"]) {
      const unknown = classifyHistory(h, {}, h.heads.source, h.base, {
        GITHUB_EVENT_NAME: name,
      });
      assert.equal(unknown.macosRequired, true, name);
      assert.equal(unknown.isReleaseOnly, false, name);
      assert.match(unknown.reason, new RegExp(`unrecognised event: "${name}"`));
    }
    // Identity fields must be owner/name strings; matching non-strings never
    // authorize a release-only omission.
    for (const forged of [7, true, "", "notignored"]) {
      const event = payload();
      event.repository = { full_name: forged };
      event.pull_request.head.repo = { full_name: forged };
      event.pull_request.base.repo = { full_name: forged };
      assert.equal(
        classifyHistory(h, event).macosRequired,
        true,
        JSON.stringify(forged),
      );
    }
    const numericRef = payload();
    numericRef.pull_request.head.ref = 7;
    assert.equal(classifyHistory(h, numericRef).macosRequired, true);
    for (const name of [7, "", null, ["push"]])
      assert.equal(
        classifyHistory(h, { ...payload(), event_name: name }).macosRequired,
        true,
        JSON.stringify(name),
      );
  } finally {
    h.close();
  }
});

test("all admitted manifest forms reject non-version content and added/deleted/renamed files", () => {
  const h = history();
  try {
    for (const [path, change] of [
      [
        "Cargo.toml",
        (text) =>
          text.replace('version = "0.1.0"', 'version = "not-a-version"'),
      ],
      ["CHANGELOG.md", () => "Release notes without a version heading\n"],
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
    h.git("checkout", "-q", h.base);
    const cargoPath = join(h.dir, "Cargo.toml");
    writeFileSync(
      cargoPath,
      readFileSync(cargoPath, "utf8").replace(
        'version = "0.1.0"',
        'version = "1.2.3-rc.1+build.2"',
      ),
    );
    h.git("add", "-u");
    h.git("commit", "-qm", "combined prerelease and build version");
    assert.equal(
      classifyHistory(h, payload(), h.git("rev-parse", "HEAD")).isReleaseOnly,
      true,
    );
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

test("same-named registry dependencies keep their versions; only repository packages are erased", () => {
  const h = history();
  try {
    // The repository's own packages, identified by source/path, bump alone.
    assert.equal(classifyHistory(h, payload()).isReleaseOnly, true);
    const admitted = [];
    // Each edit moves only a same-named registry or dependency entry's version;
    // its source and every other field stay as they were.
    for (const [path, from, to] of [
      [
        "Cargo.lock",
        'version = "0.0.9"\nsource = "registry+',
        'version = "0.0.10"\nsource = "registry+',
      ],
      [
        "python/notignored-sdk/uv.lock",
        'version="0.0.9"\nsource={registry=',
        'version="0.0.10"\nsource={registry=',
      ],
      [
        "python/notignored-sdk/pyproject.toml",
        "notignored-cli==0.0.9",
        "notignored-cli==0.0.10",
      ],
      ["package-lock.json", '"version":"0.0.9"', '"version":"0.0.10"'],
      [
        "npm/notignored-sdk/package.json",
        '"notignored-cli":"0.0.9"',
        '"notignored-cli":"0.0.10"',
      ],
      [
        "npm/notignored/package.json",
        '"notignored-cli-darwin-arm64":"0.0.0-managed"',
        '"notignored-cli-darwin-arm64":"0.0.10"',
      ],
    ]) {
      for (const release of [false, true]) {
        h.git("checkout", "-q", release ? h.heads.version : h.base);
        const file = join(h.dir, path);
        const text = readFileSync(file, "utf8");
        assert.ok(text.includes(from), `${path} fixture lacks ${from}`);
        writeFileSync(file, text.replace(from, to));
        h.git("add", "-u");
        h.git("commit", "-qm", `registry ${path}`);
        const head = h.git("rev-parse", "HEAD");
        const verdict = classifyHistory(h, payload(), head);
        if (!verdict.macosRequired || verdict.isReleaseOnly)
          admitted.push(`${path}${release ? " beside a release bump" : ""}`);
      }
    }
    assert.deepEqual(admitted, [], "registry version edits authorized a skip");
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
  mkdirSync(join(h.dir, "scripts"), { recursive: true });
  writeFileSync(
    join(h.dir, "scripts/release-version.mjs"),
    readFileSync(resolve("scripts/release-version.mjs")),
  );
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
      GITHUB_EVENT_NAME: eventName,
      POLICY_BASE: h.base,
      POLICY_HEAD: head,
    },
  });
  return Object.fromEntries(
    readFileSync(output, "utf8")
      .trim()
      .split("\n")
      .map((line) => [
        line.slice(0, line.indexOf("=")),
        line.slice(line.indexOf("=") + 1),
      ]),
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
      const policy = classifyHistory(h, event);
      const step = workflowClassify(h, event);
      assert.equal(step.macos, String(policy.macosRequired));
      assert.equal(step.reason, policy.reason);
    }
    const draft = workflowClassify(h, payload("feature", false, true));
    assert.equal(draft.macos, "false");
    assert.equal(
      draft.reason,
      "draft PR: Mac lane deferred to ready_for_review",
    );
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

test("a draft PR defers its Mac lane to ready_for_review; a malformed draft flag never does", () => {
  const h = history();
  try {
    // Every head, ownership and branch: a draft never requests Mac coverage.
    for (const [branch, fork] of [
      ["feature", false],
      ["feature", true],
      [undefined, false],
      [undefined, true],
    ])
      for (const head of [
        h.heads.source,
        h.heads.version,
        h.heads.dependency,
      ]) {
        const deferred = classifyHistory(
          h,
          payload(branch, fork, true),
          head,
          h.base,
          { GITHUB_EVENT_NAME: "pull_request" },
        );
        assert.deepEqual(deferred, {
          isReleaseOnly: false,
          macosRequired: false,
          reason: "draft PR: Mac lane deferred to ready_for_review",
        });
      }
    // Marking it ready delivers the same pull request with draft: false, which
    // is an ordinary pull request again.
    const ready = { ...payload("feature"), action: "ready_for_review" };
    assert.equal(
      classifyHistory(h, ready, h.heads.source, h.base, {
        GITHUB_EVENT_NAME: "pull_request",
      }).macosRequired,
      true,
    );
    assert.equal(
      classifyHistory(
        h,
        { ...payload(), action: "ready_for_review" },
        h.heads.version,
        h.base,
        { GITHUB_EVENT_NAME: "pull_request" },
      ).isReleaseOnly,
      true,
    );
    // Absent or non-boolean draft flags fail closed, even on a release branch
    // whose diff would otherwise be release-only.
    for (const flag of [undefined, null, "true", 1, {}]) {
      const event = payload();
      if (flag === undefined) delete event.pull_request.draft;
      else event.pull_request.draft = flag;
      const policy = classifyHistory(h, event, h.heads.version, h.base, {
        GITHUB_EVENT_NAME: "pull_request",
      });
      assert.deepEqual(
        policy,
        {
          isReleaseOnly: false,
          macosRequired: true,
          reason: "malformed draft flag",
        },
        JSON.stringify(flag),
      );
    }
  } finally {
    h.close();
  }
});
