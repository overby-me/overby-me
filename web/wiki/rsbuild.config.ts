import { defineConfig } from "@rsbuild/core";
import { pluginNodePolyfill } from "@rsbuild/plugin-node-polyfill";
import { pluginReact } from "@rsbuild/plugin-react";
import { pluginSvgr } from "@rsbuild/plugin-svgr";

export default defineConfig({
	plugins: [pluginReact(), pluginNodePolyfill(), pluginSvgr()],
	html: {
		template: "./public/index.html",
	},
	source: {
		entry: {
			index: "./src/index.tsx",
		},
	},
	server: {
		historyApiFallback: true,
	},
	output: {
		assetPrefix: "/",
		sourceMap: {
			js:
				process.env.NODE_ENV === "production"
					? "hidden-source-map"
					: "cheap-module-source-map",
		},
	},
});
