import { defineConfig } from "vitest/config";

export default defineConfig({
  build: {
    target: "es2022",
    minify: false,
    outDir: "dist/wasm-driver",
    emptyOutDir: true,
    lib: {
      entry: "src/wasm-driver/main.ts",
      formats: ["es"],
      fileName: () => "main.js",
    },
    rollupOptions: {
      output: {
        codeSplitting: false,
      },
    },
  },
  test: {
    include: ["src/webm/**/*.test.ts"],
  },
});
