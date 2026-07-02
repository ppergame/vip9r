import { createReadStream, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vitest/config";

const repoRoot = fileURLToPath(new URL("..", import.meta.url));

const MEDIA_ROOT = "/bulk/vip9r";

// Dev-only: serve the local media corpus at /media/<path>. The deployed bundle
// fetches media from wherever it is hosted; only the base URL differs.
function mediaCorpus(): Plugin {
  return {
    name: "vip9r-media-corpus",
    configureServer(server) {
      server.middlewares.use("/media", (req, res, next) => {
        const pathname = decodeURIComponent(
          new URL(req.url ?? "/", "http://localhost").pathname,
        );
        const file = path.normalize(path.join(MEDIA_ROOT, pathname));
        if (!file.startsWith(MEDIA_ROOT + path.sep)) {
          res.statusCode = 403;
          res.end("forbidden");
          return;
        }

        let size: number;
        try {
          const stat = statSync(file);
          if (!stat.isFile()) {
            next();
            return;
          }
          size = stat.size;
        } catch {
          next();
          return;
        }

        res.setHeader(
          "content-type",
          file.endsWith(".webm") ? "video/webm" : "application/octet-stream",
        );
        res.setHeader("content-length", String(size));
        createReadStream(file).pipe(res);
      });
    },
  };
}

export default defineConfig({
  plugins: [mediaCorpus()],
  server: {
    fs: {
      // The wasm module is imported by URL from rust/target, outside the js
      // workspace root.
      allow: [repoRoot],
    },
  },
  build: {
    target: "es2022",
    outDir: "dist/web",
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
