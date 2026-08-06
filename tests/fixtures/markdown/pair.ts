// eslint-disable-next-line no-console -- the banner is this program's output
console.log("notignored");

export function widget(name: string): string {
  return name.trim();
}

// @ts-ignore
export const LEGACY = widget(undefined);

// llmlint: ignore-file[suppressions_justified] fixture input, not production code:
// the reason-less directive on line 8 is what proves an unjustified suppression
// renders as "no reason given" (tests/golden/markdown/count-3.md).
