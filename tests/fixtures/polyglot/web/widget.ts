// llmlint: ignore-file[tool_output_is_signal] a demo widget whose whole job is to narrate its boot

// @ts-expect-error the vendored SDK ships no types
import sdk from "vendor-sdk";

// eslint-disable-next-line no-console -- the boot banner is this widget's output
console.log("widget booting");

// biome-ignore lint/suspicious/noDebugger: stepping through the layout pass
debugger;

/* eslint-disable-next-line no-alert
   -- the consent prompt is required by the vendor's terms,
      and their SDK offers no other entry point */
alert(sdk.consent());
