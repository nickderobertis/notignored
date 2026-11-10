// A widget that has to talk to a vendored analytics global.

declare const analytics: { track(name: string): void };

export function mount(node: HTMLElement, label: string): void {
  // eslint-disable-next-line no-console -- the mount path is traced in production
  console.log(`mounting ${label}`);

  // @ts-expect-error the vendored global is declared without its options bag
  analytics.track("widget:mounted", { label });

  node.dataset.label = label;
}

// biome-ignore lint/suspicious/noExplicitAny: the analytics options are untyped
export type MountOptions = Record<string, any>;

// biome-ignore lint/suspicious/noExplicitAny: the analytics options are untyped
export function track(name: string, options: any): void {
  analytics.track(name, options);
}
