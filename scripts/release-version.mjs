import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

// One grammar per ecosystem, shared by the packagers, the classifier and the
// provider probes so none of them admits a version the others would refuse.
// https://semver.org/ and https://packaging.python.org/en/latest/specifications/version-specifiers/
export const RELEASE_VERSION =
	/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$/;
const PYTHON_METADATA_VERSION =
	/^(?:[1-9]\d*!)?(?:0|[1-9]\d*)(?:\.(?:0|[1-9]\d*))*(?:(?:a|b|rc)(?:0|[1-9]\d*))?(?:\.post(?:0|[1-9]\d*))?(?:\.dev(?:0|[1-9]\d*))?(?:\+[a-z0-9]+(?:\.[a-z0-9]+)*)?$/;

const matches = (pattern, value) =>
	typeof value === "string" && !/[\r\n]/.test(value) && pattern.test(value);

export const isReleaseVersion = (value) => matches(RELEASE_VERSION, value);
export const isPythonMetadataVersion = (value) =>
	matches(PYTHON_METADATA_VERSION, value);

if (
	process.argv[1] &&
	fileURLToPath(import.meta.url) === resolve(process.argv[1])
) {
	try {
		const { values, positionals } = parseArgs({
			allowPositionals: true,
			options: { kind: { type: "string" }, value: { type: "string" } },
		});
		const validators = {
			tag: (value) =>
				typeof value === "string" &&
				value.startsWith("v") &&
				isReleaseVersion(value.slice(1)),
			release: isReleaseVersion,
			python: isPythonMetadataVersion,
		};
		if (
			positionals.join() !== "validate" ||
			!Object.hasOwn(validators, values.kind) ||
			!validators[values.kind](values.value)
		)
			throw new Error("invalid release version or validation arguments");
	} catch (error) {
		console.error(`release-version: ${error.message}
ACTION: inspect the provider's single-line version metadata before retrying`);
		process.exitCode = 1;
	}
}
