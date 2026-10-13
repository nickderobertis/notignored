import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

// Release identifiers use the npm packager's established grammar.
export const RELEASE_VERSION =
	/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

export const isReleaseVersion = (value) =>
	typeof value === "string" &&
	!/[\r\n]/.test(value) &&
	RELEASE_VERSION.test(value);

// Managed Python metadata already admits dotted forms such as 0.0.0.dev0.
const PYTHON_METADATA_VERSION = /^\d+\.\d+\.\d+(?:[-+.][\w.-]+)?$/;
export const isPythonMetadataVersion = (value) =>
	isReleaseVersion(value) ||
	(typeof value === "string" &&
		!/[\r\n]/.test(value) &&
		PYTHON_METADATA_VERSION.test(value));

if (
	process.argv[1] &&
	fileURLToPath(import.meta.url) === resolve(process.argv[1])
) {
	try {
		const { values, positionals } = parseArgs({
			allowPositionals: true,
			options: { kind: { type: "string" }, value: { type: "string" } },
		});
		// Provider probes use strict registry grammars; the established artifact
		// and classifier field grammar above stays backward compatible.
		// https://semver.org/ and https://packaging.python.org/en/latest/specifications/version-specifiers/
		const semver =
			/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$/;
		const python =
			/^(?:[1-9]\d*!)?(?:0|[1-9]\d*)(?:\.(?:0|[1-9]\d*))*(?:(?:a|b|rc)(?:0|[1-9]\d*))?(?:\.post(?:0|[1-9]\d*))?(?:\.dev(?:0|[1-9]\d*))?(?:\+[a-z0-9]+(?:\.[a-z0-9]+)*)?$/;
		const matches = (pattern, value) =>
			typeof value === "string" && !/[\r\n]/.test(value) && pattern.test(value);
		const validators = {
			tag: (value) =>
				typeof value === "string" &&
				value.startsWith("v") &&
				matches(semver, value.slice(1)),
			release: (value) => matches(semver, value),
			python: (value) => matches(python, value),
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
