// XR Web Runtime — Test Runner
//
// Runs all XR web runtime test suites. Modeled after web/test-js/run.ts.
//
// Usage:
//   deno run --allow-read xr/web/test-js/run.ts

import { summary } from "./harness.ts";
import { testXRInput } from "./xr-input.test.ts";
import { testXRPanel } from "./xr-panel.test.ts";
import { testXRRasterize, testXRRasterizeAsync } from "./xr-rasterize.test.ts";
import { testXRRuntime } from "./xr-runtime.test.ts";
import { testXRTypes } from "./xr-types.test.ts";

async function run(): Promise<void> {
	console.log("mojo-gui XR web runtime tests\n");

	// Synchronous test suites
	testXRTypes();
	testXRPanel();
	testXRInput();
	testXRRasterize();

	// Async test suites (XR runtime uses async initialize/destroy,
	// rasterize async tests use async updateDirtyTextures)
	await testXRRasterizeAsync();
	await testXRRuntime();

	summary();
}

run().catch((err: unknown) => {
	console.error("Fatal error:", err);
	Deno.exit(2);
});
