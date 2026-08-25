// llmlint: ignore[no_debug_prints] the banner below is this module's own output
// eslint-disable-next-line no-console
console.log("notignored");

// llmlint: ignore[no_debug_prints] the breakpoint below is left in on purpose
// biome-ignore lint/suspicious/noDebugger: paused on purpose
debugger;

// llmlint: ignore[errors_are_contextualized] the vendored global carries no context
// @ts-expect-error the analytics global is declared without its options bag
export const legacy = analytics.options;
