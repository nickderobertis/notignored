// Release identifiers use the npm packager's established grammar.
export const RELEASE_VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

export const isReleaseVersion = (value) =>
  typeof value === "string" && RELEASE_VERSION.test(value);

// Managed Python metadata already admits dotted forms such as 0.0.0.dev0.
const PYTHON_METADATA_VERSION = /^\d+\.\d+\.\d+(?:[-+.][\w.-]+)?$/;
export const isPythonMetadataVersion = (value) =>
  isReleaseVersion(value) ||
  (typeof value === "string" && PYTHON_METADATA_VERSION.test(value));
