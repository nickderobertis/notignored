// A chart that reports its renders to the vendored analytics global.

declare const analytics: { track(name: string, options?: unknown): void };

export function draw(label: string): void {
  analytics.track("chart:drawn", { label });
}
