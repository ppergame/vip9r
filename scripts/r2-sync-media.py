#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import urllib.request


SOURCE = "/bulk/vip9r/"
DESTINATION = ":s3:media/vip9r/"
CACHE_CONTROL = "public, max-age=31536000, immutable"


def r2_credentials(token: str) -> tuple[str, str]:
    request = urllib.request.Request(
        "https://api.cloudflare.com/client/v4/user/tokens/verify",
        headers={"Authorization": f"Bearer {token}"},
    )
    with urllib.request.urlopen(request) as response:
        payload = json.load(response)

    if not payload["success"]:
        raise RuntimeError(payload)

    access_key_id = payload["result"]["id"]
    secret_access_key = hashlib.sha256(token.encode()).hexdigest()
    return access_key_id, secret_access_key


def main() -> int:
    if not Path(SOURCE).is_dir():
        print(f"missing media source: {SOURCE}", file=sys.stderr)
        return 1

    env = dict(os.environ)
    if "CLOUDFLARE_API_TOKEN" not in env:
        print("set CLOUDFLARE_API_TOKEN", file=sys.stderr)
        return 1
    token = env["CLOUDFLARE_API_TOKEN"]

    if "CLOUDFLARE_ACCOUNT_ID" not in env:
        print("set CLOUDFLARE_ACCOUNT_ID", file=sys.stderr)
        return 1
    account_id = env["CLOUDFLARE_ACCOUNT_ID"]

    env["AWS_ACCESS_KEY_ID"], env["AWS_SECRET_ACCESS_KEY"] = r2_credentials(token)

    command = [
        "rclone",
        "sync",
        SOURCE,
        DESTINATION,
        "--s3-provider",
        "Cloudflare",
        "--s3-env-auth",
        "--s3-region",
        "auto",
        "--s3-endpoint",
        f"https://{account_id}.r2.cloudflarestorage.com",
        "--header-upload",
        f"Cache-Control: {CACHE_CONTROL}",
        "--progress",
        *sys.argv[1:],
    ]
    return subprocess.run(command, env=env).returncode


if __name__ == "__main__":
    sys.exit(main())
