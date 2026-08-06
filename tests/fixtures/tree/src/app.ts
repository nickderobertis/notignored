// eslint-disable-next-line no-console -- debugging aid
console.log("// eslint-disable-next-line no-console");

// biome-ignore lint/suspicious/noDebugger: stepping through the parser
debugger;

// @ts-expect-error the vendored SDK ships no types
import sdk from "vendor-sdk";
