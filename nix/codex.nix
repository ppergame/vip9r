{pkgs}: let
  inherit (pkgs) lib;

  version = "0.142.3";

  target = {
    platformTag = "linux-x64";
    targetTriple = "x86_64-unknown-linux-musl";
    hash = "sha256-WsKyW4BPu7Mf7iEVFqEwxJS2r3owUKGOoQpmXMGEjeA=";
  };

  codexNpm = pkgs.fetchzip {
    name = "openai-codex-${version}-${target.platformTag}";
    url = "https://registry.npmjs.org/@openai/codex/-/codex-${version}-${target.platformTag}.tgz";
    hash = target.hash;
  };
in
  pkgs.runCommand "codex-${version}" {
    passthru = {
      inherit version;
      inherit (target) platformTag targetTriple;
    };

    meta = {
      description = "OpenAI Codex CLI";
      homepage = "https://developers.openai.com/codex";
      license = lib.licenses.asl20;
      mainProgram = "codex";
      platforms = ["x86_64-linux"];
    };
  } ''
    package=${codexNpm}/vendor/${target.targetTriple}
    codex="$package/bin/codex"

    if [[ ! -x "$codex" ]]; then
      echo "missing expected Codex executable: $codex" >&2
      exit 1
    fi

    install -Dm755 "$codex" "$out/bin/codex"
  ''
