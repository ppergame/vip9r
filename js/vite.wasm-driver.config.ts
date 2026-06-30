import { defineConfig } from "vitest/config";

export default defineConfig({
  build: {
    target: "es2022",
    minify: false,
    outDir: "dist/wasm-driver",
    emptyOutDir: true,
    lib: {
      entry: {
        golden: "src/wasm-driver/main.ts",
        microbench: "src/wasm-driver/microbench.ts",
        tests: "src/wasm-tests/main.ts",
      },
      formats: ["es"],
      fileName: (_format, entryName) => `${entryName}.js`,
    },
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
