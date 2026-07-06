#!/usr/bin/env python3
import os
from pathlib import Path
import shlex
import subprocess
import sys


REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_ROOT = REPO_ROOT / "rust"
JS_ROOT = REPO_ROOT / "js"
DIST = JS_ROOT / "dist" / "web"
PAGES_PROJECT = "vip9r"
PAGES_BRANCH = "main"


def run(command: list[str], *, cwd: Path, env: dict[str, str]) -> int:
    rel = cwd.relative_to(REPO_ROOT)
    print(f"+ cd {shlex.quote(str(rel))} && {shlex.join(command)}", flush=True)
    return subprocess.run(command, cwd=cwd, env=env).returncode


def main() -> int:
    env = dict(os.environ)

    for name in ["CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID"]:
        if not env.get(name):
            print(f"set {name}", file=sys.stderr)
            return 1

    for command, cwd in [
        (["cargo", "build", "--release"], RUST_ROOT),
        (["pnpm", "build"], JS_ROOT),
    ]:
        exit_code = run(command, cwd=cwd, env=env)
        if exit_code != 0:
            return exit_code

    if not (DIST / "index.html").is_file():
        print(f"missing Pages build output: {DIST}", file=sys.stderr)
        return 1

    command = [
        "pnpm",
        "exec",
        "wrangler",
        "pages",
        "deploy",
        str(DIST.relative_to(JS_ROOT)),
        "--project-name",
        PAGES_PROJECT,
        "--branch",
        PAGES_BRANCH,
    ]
    return run(command, cwd=JS_ROOT, env=env)


if __name__ == "__main__":
    sys.exit(main())
