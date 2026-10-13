import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { parse } from "yaml";

test("real manual diagnostic steps validate provider metadata before emitting outputs or installing npm", () => {
  const dir = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir(), "diagnostic-"),
  );
  try {
    for (const provider of ["gh", "pip", "npm"]) {
      writeFileSync(
        join(dir, provider),
        `#!/usr/bin/env bash
case "$1" in
  install) printf '%s' installed >> "$INSTALL_RECORD"; exit 0 ;;
  show) printf 'Version: %s' "$PROVIDER_VERSION" ;;
  *) printf '%s' "$PROVIDER_VERSION" ;;
esac
`,
        { mode: 0o755 },
      );
    }
    const steps = parse(
      readFileSync(".github/workflows/published-smoke.yml", "utf8"),
    ).jobs["macos-diagnostic"].steps;
    for (const [id, good] of [
      ["release", "v1.2.3-rc.1+build.2"],
      ["pypi-install", "1.2.3.dev0"],
      ["npm-install", "1.2.3-rc.1+build.2"],
    ]) {
      const script = steps.find((step) => step.id === id).run;
      const validVersions =
        id === "pypi-install"
          ? [good, "1.2.3rc1+build.2", "1.2.3.post1"]
          : [
              good,
              ...(id === "release"
                ? ["v1.0.0+001", "v1.0.0-x-y-z.--"]
                : ["1.0.0+001", "1.0.0-x-y-z.--"]),
            ];
      for (const version of [
        ...validVersions,
        "",
        "invalid",
        "1.2.3\nextra=unsafe",
        ...(id === "pypi-install"
          ? ["1.2.3.foo", "1.2.3+_"]
          : [
              id === "release" ? "v01.2.3" : "01.2.3",
              id === "release" ? "v1.2.3-.." : "1.2.3-..",
              id === "release" ? "v1.2.3-01" : "1.2.3-01",
            ]),
      ]) {
        const output = join(dir, "outputs");
        const installs = join(dir, "installs");
        writeFileSync(output, "");
        writeFileSync(installs, "");
        const result = spawnSync(
          "bash",
          ["-e", "-o", "pipefail", "-c", script],
          {
            encoding: "utf8",
            env: {
              ...process.env,
              PATH: `${dir}:${process.env.PATH}`,
              PROVIDER_VERSION:
                id === "pypi-install"
                  ? version.replaceAll("\n", "\nVersion: ")
                  : version,
              INSTALL_RECORD: installs,
              GITHUB_OUTPUT: output,
              GITHUB_REPOSITORY: "nickderobertis/notignored",
              RUNNER_OS: "macOS",
              RUNNER_ARCH: "ARM64",
            },
          },
        );
        if (validVersions.includes(version)) {
          assert.equal(result.status, 0, result.stderr);
          assert.equal(
            readFileSync(output, "utf8"),
            `${id === "release" ? "tag" : "version"}=${version}\n`,
          );
        } else {
          assert.notEqual(
            result.status,
            0,
            `${id} accepted ${JSON.stringify(version)}`,
          );
          assert.match(result.stderr, /ACTION:/);
          assert.equal(readFileSync(output, "utf8"), "");
          if (id === "npm-install")
            assert.equal(readFileSync(installs, "utf8"), "");
        }
      }
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
