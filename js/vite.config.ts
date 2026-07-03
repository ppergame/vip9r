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
        let etag: string;
        try {
          const stat = statSync(file);
          if (!stat.isFile()) {
            next();
            return;
          }
          size = stat.size;
          etag = `"${stat.size.toString(16)}-${Math.trunc(stat.mtimeMs).toString(16)}"`;
        } catch {
          next();
          return;
        }

        res.setHeader("etag", etag);
        if (req.headers["if-none-match"] === etag) {
          res.statusCode = 304;
          res.end();
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
    rollupOptions: {
      input: {
        index: path.join(repoRoot, "js/index.html"),
        citygen: path.join(repoRoot, "js/citygen.html"),
        citygenDecant: path.join(repoRoot, "js/citygen-decant.html"),
      },
    },
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
