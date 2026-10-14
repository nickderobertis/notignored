// The internal routing authority. Unknown PR data always asks for Mac coverage.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { isDeepStrictEqual, parseArgs } from "node:util";

import {
  isReleaseVersion,
  isPythonMetadataVersion,
} from "../scripts/release-version.mjs";

export const releaseBranchPrefix = "release-plz-";
// The repository's own packages are identified by where they come from, never
// by name: a registry package may share any name this repository publishes, and
// its version is a dependency choice, not a release bump.
const ownName = (name) =>
  /^notignored(?:-cli|-sdk)?$/.test(typeof name === "string" ? name : "");
// Cargo.lock records `source` only for registry and git packages; a workspace
// or path package — the crate this repository builds — has none.
const cargoOwn = (pkg) =>
  !Object.hasOwn(pkg, "source") && pkg.name === "notignored";
// uv.lock marks the project and its workspace members with a local source.
const uvOwn = (pkg) => {
  const source = pkg.source;
  if (!source || typeof source !== "object" || Array.isArray(source))
    return false;
  const keys = Object.keys(source);
  return (
    keys.length === 1 &&
    ["editable", "virtual", "directory"].includes(keys[0]) &&
    source[keys[0]] === "." &&
    ownName(pkg.name)
  );
};
// package-lock.json keys the root package "" and workspace members by their
// repository path; everything installed from a registry lives under
// node_modules/ and keeps its version.
const npmLockOwn = (key, pkg) =>
  (key === "" || !key.split("/").includes("node_modules")) &&
  !pkg.link &&
  !Object.hasOwn(pkg, "resolved") &&
  ownName(pkg.name);
function eraseVersion(object, accepts = isReleaseVersion) {
  if ("version" in object) {
    if (!accepts(object.version)) throw new Error("invalid version");
    object.version = "<version>";
  }
}
function parseToml(text) {
  return JSON.parse(
    execFileSync(
      "python3",
      [
        "-c",
        "import json,sys,tomllib; print(json.dumps(tomllib.loads(sys.stdin.read())))",
      ],
      { input: text, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] },
    ),
  );
}
function packages(doc) {
  if (doc.package === undefined) return [];
  if (!Array.isArray(doc.package)) throw new Error("malformed lock packages");
  return doc.package;
}
// Each admitted path is the manifest or lockfile of a known repository package;
// only that package's own version is erased. Dependency specifications, however
// they are named, are compared exactly.
function semantic(path, text) {
  if (path === "CHANGELOG.md") {
    if (!/^## \[?v?\d+\.\d+\.\d+/m.test(text))
      throw new Error("changelog has no release heading");
    return "<changelog>";
  }
  if (path === "Cargo.toml") {
    const doc = parseToml(text);
    if (doc.package?.name !== "notignored")
      throw new Error("unexpected package");
    eraseVersion(doc.package);
    return doc;
  }
  if (path === "Cargo.lock") {
    const doc = parseToml(text);
    for (const pkg of packages(doc)) if (cargoOwn(pkg)) eraseVersion(pkg);
    return doc;
  }
  if (path === "python/notignored-sdk/uv.lock") {
    const doc = parseToml(text);
    for (const pkg of packages(doc))
      if (uvOwn(pkg)) eraseVersion(pkg, isPythonMetadataVersion);
    return doc;
  }
  if (
    path === "pyproject.toml" ||
    path === "python/notignored-sdk/pyproject.toml"
  ) {
    const doc = parseToml(text);
    if (!ownName(doc.project?.name)) throw new Error("unexpected project");
    eraseVersion(doc.project, isPythonMetadataVersion);
    return doc;
  }
  if (
    [
      "package.json",
      "npm/notignored/package.json",
      "npm/notignored-sdk/package.json",
    ].includes(path)
  ) {
    const doc = JSON.parse(text);
    if (!doc || typeof doc !== "object" || Array.isArray(doc))
      throw new Error("malformed manifest");
    if (ownName(doc.name)) eraseVersion(doc);
    return doc;
  }
  if (path === "package-lock.json") {
    const doc = JSON.parse(text);
    if (!doc || typeof doc !== "object" || Array.isArray(doc))
      throw new Error("malformed lockfile");
    if (ownName(doc.name)) eraseVersion(doc);
    const entries = doc.packages ?? {};
    if (typeof entries !== "object" || Array.isArray(entries))
      throw new Error("malformed lock packages");
    for (const [key, pkg] of Object.entries(entries))
      if (pkg && typeof pkg === "object" && npmLockOwn(key, pkg))
        eraseVersion(pkg);
    return doc;
  }
  throw new Error(`non-release file: ${path}`);
}
// The automatic and manual non-PR events the workflows subscribe to; none of
// them carries a pull request whose Mac lane could be owed.
const knownNonPrEvents = new Set([
  "push",
  "release",
  "workflow_run",
  "repository_dispatch",
  "schedule",
  "workflow_dispatch",
]);
export function classify(event, base, head, cwd = process.cwd()) {
  const ordinary = (reason) => ({
    isReleaseOnly: false,
    macosRequired: true,
    reason,
  });
  if (!event || typeof event !== "object" || Array.isArray(event))
    return ordinary("malformed PR payload");
  if (Object.hasOwn(event, "event_name")) {
    const name = event.event_name;
    if (typeof name !== "string" || !name)
      return ordinary("malformed event name");
    if (knownNonPrEvents.has(name))
      return {
        isReleaseOnly: false,
        macosRequired: false,
        reason: `non-PR event: ${name}`,
      };
    // An event this policy has never classified fails closed.
    if (name !== "pull_request")
      return ordinary(`unrecognised event: ${JSON.stringify(name)}`);
  }
  if (!event.pull_request) return ordinary("unknown PR payload");
  const draft = event.pull_request.draft;
  // A draft cannot merge, and marking it ready starts a run of its own, so the
  // one Mac job is spent on that run rather than on every draft push. Anything
  // but a real boolean asks for Mac coverage.
  if (typeof draft !== "boolean") return ordinary("malformed draft flag");
  if (draft)
    return {
      isReleaseOnly: false,
      macosRequired: false,
      reason: "draft PR: Mac lane deferred to ready_for_review",
    };
  try {
    const pr = event.pull_request;
    const repository = event.repository?.full_name;
    const name = (value) =>
      typeof value === "string" && /^[^/\s]+\/[^/\s]+$/.test(value);
    if (
      !name(repository) ||
      typeof pr.head?.ref !== "string" ||
      repository !== pr.head?.repo?.full_name ||
      repository !== pr.base?.repo?.full_name ||
      !pr.head?.ref?.startsWith(releaseBranchPrefix)
    )
      return ordinary("ordinary or fork PR");
    if (!base || !head || base.startsWith("-") || head.startsWith("-"))
      throw new Error("missing or invalid refs");
    const git = (args) =>
      execFileSync("git", args, {
        cwd,
        encoding: "utf8",
        stdio: ["pipe", "pipe", "pipe"],
      });
    const mergeBase = git(["merge-base", base, head]).trim();
    const files = git([
      "diff",
      "--name-status",
      "-z",
      mergeBase,
      head,
      "--",
    ]).split("\0");
    let count = 0;
    for (let i = 0; i < files.length - 1; i += 2) {
      const [status, path] = files.slice(i, i + 2);
      if (status !== "M")
        throw new Error(`non-modification: ${status} ${path}`);
      const before = semantic(path, git(["show", `${mergeBase}:${path}`]));
      const after = semantic(path, git(["show", `${head}:${path}`]));
      if (!isDeepStrictEqual(before, after))
        return ordinary(`non-version content: ${path}`);
      count++;
    }
    if (!count) throw new Error("empty diff");
    return {
      isReleaseOnly: true,
      macosRequired: false,
      reason:
        "same-repository release branch with semantic version/changelog-only diff",
    };
  } catch (error) {
    return ordinary(
      `unreadable release classification: ${error.message.split("\n")[0]}`,
    );
  }
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  try {
    const { values, positionals } = parseArgs({
      options: {
        event: { type: "string" },
        base: { type: "string" },
        head: { type: "string" },
      },
      allowPositionals: true,
    });
    if (positionals.join() !== "classify")
      throw new Error("use classify --event JSON --base REF --head REF");
    let event;
    try {
      event = JSON.parse(readFileSync(values.event, "utf8"));
    } catch {
      event = {};
    }
    // The runner owns event identity; payload data cannot authorize a PR skip.
    if (process.env.GITHUB_EVENT_NAME)
      event = { ...event, event_name: process.env.GITHUB_EVENT_NAME };
    console.log(JSON.stringify(classify(event, values.base, values.head)));
  } catch (error) {
    console.error(
      `macos-policy: ${error.message}\nACTION: pass classify --event JSON --base REF --head REF`,
    );
    process.exitCode = 1;
  }
}
