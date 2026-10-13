// Native syntax and structural validation for private JavaScript tooling.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { parse } from "yaml";

function files(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.name === "node_modules" || entry.name === ".telemetry") return [];
    const path = `${dir}/${entry.name}`;
    return entry.isDirectory() ? files(path) : [path];
  });
}
try {
  const mode = process.argv[2];
  if (!["format", "format-check", "lint", "schema-check"].includes(mode))
    throw new Error("unknown tooling check");
  for (const path of files(".github")) {
    if (path.endsWith(".mjs") && ["lint", "schema-check"].includes(mode))
      execFileSync("node", ["--check", path], { stdio: "pipe" });
    if (path.endsWith(".json")) {
      const text = readFileSync(path, "utf8");
      const doc = JSON.parse(text);
      const formatted = `${JSON.stringify(doc, null, 2)}\n`;
      if (mode === "format") writeFileSync(path, formatted);
      else if (mode === "format-check" && text !== formatted)
        throw new Error(`${path}: JSON formatting drift`);
    }
    if (/\.ya?ml$/.test(path))
      parse(readFileSync(path, "utf8"), { uniqueKeys: true });
  }
  if (mode === "schema-check") {
    const workflow = parse(readFileSync(".github/workflows/ci.yml", "utf8"));
    if (!Array.isArray(workflow.jobs.required.needs))
      throw new Error("required.needs must be a list of job ids");
    for (const [id, job] of Object.entries(workflow.jobs)) {
      if (typeof job !== "object" || !job || (!job["runs-on"] && !job.uses))
        throw new Error(`invalid job type: ${id}`);
    }
  }
  if (["format", "format-check", "lint"].includes(mode)) {
    const js = files(".github").filter((path) => path.endsWith(".mjs"));
    const argumentsForBiome =
      mode === "lint"
        ? ["lint", "--error-on-warnings"]
        : [
            "format",
            "--indent-style=space",
            "--indent-width=2",
            ...(mode === "format" ? ["--write"] : []),
          ];
    execFileSync(
      "bash",
      ["scripts/dev-tool.sh", "biome", ...argumentsForBiome, ...js],
      { stdio: "pipe" },
    );
  }
  console.log(`github-workflows: ${mode} passed`);
} catch (error) {
  console.error(
    `github-workflows: ${error.message}\nACTION: fix the named syntax/shape, or run github-workflows:format for formatting`,
  );
  process.exitCode = 1;
}
