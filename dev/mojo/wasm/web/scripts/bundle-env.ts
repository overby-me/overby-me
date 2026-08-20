// Bundle the browser env glue — TypeScript → browser-ready JavaScript.
//
// examples/lib/env.js used to be a hand-written JavaScript port of
// runtime/env.ts and runtime/memory.ts, maintained beside the TypeScript it
// copied. The two drifted by one import symbol and the browser suites failed
// with a LinkError while the Deno suites, which instantiate the TypeScript,
// stayed green. Generating the JavaScript removes the second copy: the
// TypeScript is the only source, and examples/lib/env.js is a build artifact.
//
// Usage:
//   deno run --allow-read --allow-write --allow-env --allow-run scripts/bundle-env.ts
//
// The public surface is unchanged, so examples/lib/{app,boot,strings}.js keep
// importing "./env.js" exactly as before.

import { dirname, fromFileUrl, resolve } from "jsr:@std/path";
import { build, stop } from "npm:esbuild@0.25.5";

const webDir = resolve(dirname(fromFileUrl(import.meta.url)), "..");
const entry = resolve(webDir, "runtime/browser.ts");
const outfile = resolve(webDir, "../examples/lib/env.js");

const result = await build({
	entryPoints: [entry],
	bundle: true,
	outfile,
	format: "esm",
	platform: "browser",
	target: ["es2020"],
	sourcemap: false,
	minify: false, // readable in devtools, and diffable when it changes
	treeShaking: true,
	resolveExtensions: [".ts", ".js", ".mjs"],
	banner: {
		js: [
			"// GENERATED — do not edit.",
			"// Source: web/runtime/browser.ts (and what it re-exports)",
			"// Regenerate: just build   (or deno run ... scripts/bundle-env.ts)",
			"",
		].join("\n"),
	},
});

await stop();

if (result.errors.length > 0) {
	for (const err of result.errors) {
		console.error(`${err.text} (${err.location?.file}:${err.location?.line})`);
	}
	Deno.exit(1);
}

console.log(`[bundle-env] → ${outfile}`);
