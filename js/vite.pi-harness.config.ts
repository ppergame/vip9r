import { builtinModules } from "node:module";
import { defineConfig } from "vite";

const nodeBuiltins = new Set([
  ...builtinModules,
  ...builtinModules.map((name) => `node:${name}`),
]);

export default defineConfig({
  build: {
    target: "node22",
    outDir: "dist/pi-harness",
    emptyOutDir: true,
    lib: {
      entry: "src/pi-harness/main.ts",
      formats: ["es"],
      fileName: () => "pi-harness.mjs",
    },
    rollupOptions: {
      external: (id) => nodeBuiltins.has(id),
      output: {
        banner: [
          "#!/usr/bin/env node",
          'import { createRequire as __vip9rCreateRequire } from "node:module";',
          "const require = __vip9rCreateRequire(import.meta.url);",
        ].join("\n"),
        codeSplitting: false,
      },
    },
  },
});
