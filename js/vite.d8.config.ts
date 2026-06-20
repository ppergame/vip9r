import { defineConfig } from "vitest/config";

export default defineConfig({
  build: {
    target: "es2022",
    minify: false,
    outDir: "dist/d8",
    emptyOutDir: true,
    lib: {
      entry: "src/d8/main.ts",
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
