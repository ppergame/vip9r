{
  pkgs,
  rustToolchain,
  v8,
  codex,
}: let
  inherit (pkgs) lib;

  bash = lib.getExe pkgs.bashInteractive;
  caBundle = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  env = lib.getExe' pkgs.coreutils "env";
  fuseOverlayfs = lib.getExe pkgs.fuse-overlayfs;
  wasmTests = pkgs.writeShellApplication {
    name = "wasm-tests";
    runtimeInputs = [rustToolchain];
    text = ''
      set -euo pipefail

      if [[ $# -ne 0 ]]; then
        echo "usage: wasm-tests" >&2
        exit 2
      fi

      d8="''${D8_LINUX64:?D8_LINUX64 is not set}"
      runner="/run/js/dist/wasm-driver/tests.js"
      if [[ ! -f "$runner" ]]; then
        echo "missing prebuilt wasm driver: $runner" >&2
        echo "ask the orchestrator to build and supply the JS runner artifacts" >&2
        exit 2
      fi

      cargo build -p vip9r --target wasm32-unknown-unknown --release --features wasm-tests
      exec "$d8" "$runner" -- target/wasm32-unknown-unknown/release/vip9r.wasm
    '';
  };
  wasmGolden = pkgs.writeShellApplication {
    name = "wasm-golden";
    runtimeInputs = [rustToolchain];
    text = ''
      set -euo pipefail

      allow_mismatch=0
      if [[ $# -gt 0 && "$1" == "--allow-mismatch" ]]; then
        allow_mismatch=1
        shift
      fi
      if [[ $# -gt 1 ]]; then
        echo "usage: wasm-golden [--allow-mismatch] [INPUT_IVF]" >&2
        exit 2
      fi

      d8="''${D8_LINUX64:?D8_LINUX64 is not set}"
      runner="/run/js/dist/wasm-driver/golden.js"
      input="''${1:-/media/chromium/bear-vp9.ivf}"
      cargo build -p vip9r --target wasm32-unknown-unknown --release
      if [[ "$allow_mismatch" -eq 1 ]]; then
        exec "$d8" "$runner" -- --allow-mismatch target/wasm32-unknown-unknown/release/vip9r.wasm "$input"
      fi
      exec "$d8" "$runner" -- target/wasm32-unknown-unknown/release/vip9r.wasm "$input"
    '';
  };
  sandboxPackages =
    (with pkgs; [
      # Shell and baseline userland.
      bashInteractive
      bc
      coreutils
      curl
      diffutils
      dnsutils
      fd
      file
      findutils
      gawk
      gnugrep
      gnused
      gnupatch
      gnutar
      gzip
      jq
      less
      lsof
      procps
      psmisc
      rsync
      time
      tree
      unzip
      util-linux
      wget
      which
      xz
      zip
      zstd

      # Build, source control, and project-specific tooling.
      binaryen
      clang
      gnumake
      git
      libvpx
      nodejs
      pkg-config
      python3
      ripgrep
      rustToolchain
      wabt
      wasm-tools

      # Debugging and host inspection.
      strace
    ])
    ++ [codex wasmGolden wasmTests];
  sandboxEnv = pkgs.buildEnv {
    name = "vip9r-grinder-env";
    paths = sandboxPackages;
    pathsToLink = ["/bin"];
  };
  podmanEnv = name: value: ["--env" "${name}=${value}"];
  staticPodmanArgs =
    lib.escapeShellArgs
    (["run" "--rm" "-i" "--rootfs" "--userns=keep-id" "--unsetenv-all" "--workdir" "/run/rust"]
      ++ ["--storage-opt" "overlay.mount_program=${fuseOverlayfs}"]
      ++ podmanEnv "PATH" "/run/tools/bin:/bin:/usr/bin"
      ++ podmanEnv "HOME" "/run/home"
      ++ podmanEnv "SHELL" "/bin/bash"
      ++ podmanEnv "CARGO_HOME" "/cargo-home"
      ++ podmanEnv "V8_LINUX64" "${v8.linux64}"
      ++ podmanEnv "D8_LINUX64" "${v8.linux64}/d8"
      ++ podmanEnv "CODEX_HOME" "/codex-home"
      ++ podmanEnv "SSL_CERT_FILE" caBundle
      ++ podmanEnv "NIX_SSL_CERT_FILE" caBundle
      ++ podmanEnv "NODE_EXTRA_CA_CERTS" caBundle
      ++ ["--tmpfs" "/tmp" "--volume" "/nix/store:/nix/store:ro"]);
in
  pkgs.writeShellApplication {
    name = "grinder";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.git
      pkgs.nodejs
      pkgs.podman
    ];

    text = ''
      usage() {
        printf '%s\n' \
          'usage:' \
          '  grinder run TASK_FILE' \
          '  grinder shell [COMMAND...]' \
          '  grinder inspect [OPTIONS] [SESSION_JSONL_OR_DIR]' >&2
      }

      repo="$PWD"

      mode="''${1:-}"
      case "$mode" in
        run|shell|inspect) shift ;;
        -h|--help)
          usage
          exit 0
          ;;
        *)
          usage
          exit 2
          ;;
      esac

      if [[ "$mode" == inspect ]]; then
        exec node "$repo/scripts/codex-events.mjs" inspect --repo "$repo" "$@"
      fi

      task_file=""
      command=()
      case "$mode" in
        run)
          task_file="''${1:-}"
          if [[ -z "$task_file" ]]; then
            usage
            exit 2
          fi
          shift
          if [[ $# -gt 0 ]]; then
            usage
            exit 2
          fi
          ;;
        shell)
          command=("''${@}")
          ;;
      esac
      if [[ -n "$task_file" && ! -f "$task_file" ]]; then
        echo "task file not found: $task_file" >&2
        exit 2
      fi

      system_prompt="$repo/scripts/grinder-system-prompt.md"
      codex_config="$repo/scripts/grinder-codex-config.toml"
      codex_events="$repo/scripts/codex-events.mjs"
      wasm_driver_dist="$repo/js/dist/wasm-driver"
      for artifact in golden.js tests.js; do
        if [[ ! -f "$wasm_driver_dist/$artifact" ]]; then
          echo "missing prebuilt wasm driver artifact: $wasm_driver_dist/$artifact" >&2
          echo "run: cd js && pnpm build:wasm-driver" >&2
          exit 2
        fi
      done

      temp_dir="$repo/temp"
      codex_home="$temp_dir/codex-home"
      cargo_home="$temp_dir/cargo-home"
      mkdir -p "$temp_dir" "$codex_home" "$cargo_home"
      [[ -e "$codex_home/config.toml" ]] || touch "$codex_home/config.toml"
      run="$(mktemp -d -p "$temp_dir" grinder.XXXXXX)"
      mkdir -p "$run/codex-state" "$run/home" "$run/rootfs/bin" "$run/rootfs/usr/bin" "$run/trace"
      git config --file "$run/home/.gitconfig" user.name vip9r-implementor
      git config --file "$run/home/.gitconfig" user.email vip9r-implementor@example.invalid
      ln -s "${sandboxEnv}" "$run/tools"
      ln -s "${bash}" "$run/rootfs/bin/bash"
      ln -s "${bash}" "$run/rootfs/bin/sh"
      ln -s "${env}" "$run/rootfs/usr/bin/env"

      mkdir -p "$run/rust"
      shopt -s dotglob nullglob
      rust_entries=("$repo/rust"/*)
      shopt -u dotglob nullglob
      for entry in "''${rust_entries[@]}"; do
        [[ "$(basename "$entry")" == target ]] && continue
        cp -a "$entry" "$run/rust/"
      done
      mkdir -p "$run/js/dist"
      cp -a "$wasm_driver_dist" "$run/js/dist/"

      cp -a "$system_prompt" "$run/system.md"
      if [[ -n "$task_file" ]]; then
        cp -a "$task_file" "$run/task.md"
      else
        printf 'Sandbox shell.\n' > "$run/task.md"
      fi

      git -C "$run/rust" init -q
      git -C "$run/rust" add .
      git -C "$run/rust" \
        -c user.name=vip9r-orchestrator \
        -c user.email=vip9r@example.invalid \
        commit -q -m baseline
      git -C "$run/rust" tag orchestrator-base

      podman_args=(
        ${staticPodmanArgs}
        --user "$(id -u):$(id -g)"
        --volume "$run:/run:rw"
        --volume "$codex_home:/codex-home:rw"
        --volume "$codex_config:/codex-home/config.toml:ro"
        --volume "$cargo_home:/cargo-home:rw"
        --volume "$repo/docs/specs:/specs:ro"
        --volume "/bulk/vip9r:/media:ro"
      )
      rootfs_arg="$run/rootfs:O"

      status=0
      case "$mode" in
        run)
          # run mode deliberately leaves "$run" intact: the orchestrator reviews
          # and merges from "$run/rust" before reaping it. "$preserve" is the
          # durable record that outlives "$run".
          preserve="$temp_dir/traces/$(date -u +%Y%m%dT%H%M%SZ)-$(basename "$run")"
          echo "trace -> $preserve" >&2
          set +e
          podman "''${podman_args[@]}" "$rootfs_arg" \
            /run/tools/bin/codex exec \
              --json \
              -o /run/trace/final.md \
              - \
              < "$run/task.md" \
            | node "$codex_events" stream "$run/trace/codex.jsonl"
          pipeline_status=("''${PIPESTATUS[@]}")
          status="''${pipeline_status[0]}"
          renderer_status="''${pipeline_status[1]:-0}"
          set -e
          if [[ "$status" -eq 0 && "$renderer_status" -ne 0 ]]; then
            status="$renderer_status"
          fi
          thread_id=""
          if [[ -s "$run/trace/codex.jsonl" ]]; then
            thread_id="$(sed -n 's/^{"type":"thread.started","thread_id":"\([^"]*\)".*/\1/p' "$run/trace/codex.jsonl" | head -n 1)"
          fi
          if [[ -n "$thread_id" ]]; then
            shopt -s nullglob
            rollout_matches=("$codex_home"/sessions/*/*/*/rollout-*-"$thread_id".jsonl)
            shopt -u nullglob
            if [[ ''${#rollout_matches[@]} -eq 1 ]]; then
              cp -a "''${rollout_matches[0]}" "$run/trace/rollout.jsonl" || echo "rollout: copy failed for $thread_id" >&2
            else
              echo "rollout: expected 1 match for $thread_id, found ''${#rollout_matches[@]}" >&2
            fi
          else
            echo "rollout: missing thread id in codex.jsonl" >&2
          fi
          context_summary=""
          if [[ -s "$run/trace/rollout.jsonl" ]]; then
            context_summary="$(node "$codex_events" context "$run/trace/rollout.jsonl" || true)"
          fi
          if [[ -f "$run/trace/final.md" ]]; then
            cat "$run/trace/final.md"
          fi
          mkdir -p "$preserve"
          cp -a "$run/trace/." "$preserve/"
          cp -a "$run/task.md" "$preserve/task.md"
          cp -a "$run/system.md" "$preserve/system.md"
          printf '%s\n' "$status" > "$preserve/exit-status"
          {
            echo
            echo "── grinder ──"
            echo "status: $status"
            if [[ -n "$context_summary" ]]; then
              echo "context: $context_summary"
            fi
            echo "source: $run/rust"
            echo "trace:  $preserve"
          } >&2
          ;;
        shell)
          trap 'rm -rf -- "$run"' EXIT
          if [[ ''${#command[@]} -gt 0 ]]; then
            podman "''${podman_args[@]}" "$rootfs_arg" "''${command[@]}" || status=$?
          else
            podman "''${podman_args[@]}" -t "$rootfs_arg" /bin/bash -i || status=$?
          fi
          ;;
      esac

      exit "$status"
    '';
  }
