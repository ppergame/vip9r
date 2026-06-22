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
  sandboxPackages = (with pkgs; [
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
    pnpm
    python3
    ripgrep
    rustToolchain
    wabt
    wasm-tools

    # Debugging and host inspection.
    strace
  ])
  ++ [codex];
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
      pkgs.rsync
    ];

    text = ''
      usage() {
        cat >&2 <<'EOF'
      usage:
        grinder run TASK_FILE
        grinder shell [COMMAND...]
        grinder inspect [OPTIONS] [SESSION_JSONL_OR_DIR]
      EOF
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
        inspector="$repo/js/dist/pi-harness/grinder-inspect.mjs"
        if [[ ! -e "$inspector" ]]; then
          echo "missing required path: $inspector" >&2
          echo "run: cd js && pnpm build:pi-harness" >&2
          exit 1
        fi
        exec node "$inspector" --repo "$repo" "$@"
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

      rsync -a --exclude=/target "$repo/rust/" "$run/rust/"
      cp "$system_prompt" "$run/system.md"
      if [[ -n "$task_file" ]]; then
        cp "$task_file" "$run/task.md"
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
          podman "''${podman_args[@]}" "$rootfs_arg" \
            /run/tools/bin/codex exec \
              --ephemeral \
              --json \
              -o /run/trace/final.md \
              - \
              < "$run/task.md" \
              > "$run/trace/codex.jsonl" || status=$?
          if [[ -f "$run/trace/final.md" ]]; then
            cat "$run/trace/final.md"
          fi
          mkdir -p "$preserve"
          rsync -a "$run/trace/" "$preserve/"
          cp "$run/task.md" "$preserve/task.md"
          cp "$run/system.md" "$preserve/system.md"
          printf '%s\n' "$status" > "$preserve/exit-status"
          {
            echo
            echo "── grinder ──"
            echo "status: $status"
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
