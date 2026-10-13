// The internal routing authority. Unknown PR data always asks for Mac coverage.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { isDeepStrictEqual, parseArgs } from "node:util";

export const releaseBranchPrefix = "release-plz-";
const ownName = (name) =>
  /^notignored(?:-cli|-sdk)?(?:-(?:linux|darwin|win32)-(?:x64|arm64))?$/.test(
    name ?? "",
  );
const version = (value) =>
  typeof value === "string" && /^\d+\.\d+\.\d+(?:[-+.][\w.-]+)?$/.test(value);
function eraseVersion(object) {
  if ("version" in object) {
    if (!version(object.version)) throw new Error("invalid version");
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
  if (path === "Cargo.lock" || path === "python/notignored-sdk/uv.lock") {
    const doc = parseToml(text);
    for (const pkg of doc.package ?? [])
      if (ownName(pkg.name)) eraseVersion(pkg);
    return doc;
  }
  if (
    path === "pyproject.toml" ||
    path === "python/notignored-sdk/pyproject.toml"
  ) {
    const doc = parseToml(text);
    if (!ownName(doc.project?.name)) throw new Error("unexpected project");
    eraseVersion(doc.project);
    doc.project.dependencies = doc.project.dependencies?.map((dep) =>
      dep.replace(
        /^(notignored-cli)==\d+\.\d+\.\d+(?:[-+.][\w.-]+)?$/,
        "$1==<version>",
      ),
    );
    return doc;
  }
  if (
    [
      "package.json",
      "package-lock.json",
      "npm/notignored/package.json",
      "npm/notignored-sdk/package.json",
    ].includes(path)
  ) {
    const doc = JSON.parse(text);
    if (ownName(doc.name)) eraseVersion(doc);
    for (const pkg of Object.values(doc.packages ?? {}))
      if (ownName(pkg.name)) eraseVersion(pkg);
    for (const field of ["dependencies", "optionalDependencies"]) {
      for (const [name, value] of Object.entries(doc[field] ?? {})) {
        if (ownName(name) && version(value)) doc[field][name] = "<version>";
      }
    }
    return doc;
  }
  throw new Error(`non-release file: ${path}`);
}
export function classify(event, base, head, cwd = process.cwd()) {
  const ordinary = (reason) => ({
    isReleaseOnly: false,
    macosRequired: true,
    reason,
  });
  if (!event || typeof event !== "object" || Array.isArray(event))
    return ordinary("malformed PR payload");
  if (event.event_name && event.event_name !== "pull_request")
    return {
      isReleaseOnly: false,
      macosRequired: false,
      reason: "non-PR event",
    };
  if (!event.pull_request) return ordinary("unknown PR payload");
  try {
    const pr = event.pull_request;
    const repository = event.repository?.full_name;
    if (
      !repository ||
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
