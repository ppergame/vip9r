import { defineConfig } from "vitest/config";

export default defineConfig({
  build: {
    target: "es2022",
    outDir: "dist/web",
  },
  test: {
    include: ["src/webm/**/*.test.ts"],
  },
});
