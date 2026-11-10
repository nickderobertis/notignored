// A chart that reports its renders to the vendored analytics global.

declare const analytics: { track(name: string, options?: unknown): void };

export function draw(label: string): void {
  analytics.track("chart:drawn", { label });
}

// biome-ignore lint/suspicious/noExplicitAny: the analytics options are untyped
export function redraw(options: any): void {
  analytics.track("chart:redrawn", options);
}
