{
  pkgs,
  rustToolchain,
  v8,
}: let
  inherit (pkgs) lib;

  bash = lib.getExe pkgs.bashInteractive;
  caBundle = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  env = lib.getExe' pkgs.coreutils "env";
  fuseOverlayfs = lib.getExe pkgs.fuse-overlayfs;
  sandboxPackages = with pkgs; [
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
  ];
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
        run|shell|inspect) shift || true ;;
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
          shift || true
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
      harness_dir="$repo/js/dist/pi-harness"
      for path in "$repo/rust" "$repo/docs/specs" "$system_prompt" "$harness_dir/pi-harness.mjs"; do
        if [[ ! -e "$path" ]]; then
          echo "missing required path: $path" >&2
          exit 1
        fi
      done

      temp_dir="$repo/temp"
      auth_dir="$temp_dir/pi-auth"
      cargo_home="$temp_dir/cargo-home"
      mkdir -p "$temp_dir" "$auth_dir" "$cargo_home"
      run="$(mktemp -d -p "$temp_dir" grinder.XXXXXX)"
      mkdir -p "$run/home" "$run/rootfs/bin" "$run/rootfs/usr/bin" "$run/trace"
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
        --volume "$auth_dir:/auth:rw"
        --volume "$cargo_home:/cargo-home:rw"
        --volume "$repo/docs/specs:/specs:ro"
        --volume "/bulk/vip9r:/media:ro"
        --volume "$harness_dir:/harness:ro"
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
          # Inherit stdout/stderr: the final response streams on stdout, the
          # progress feed on stderr. The durable transcript is session.jsonl.
          podman "''${podman_args[@]}" "$rootfs_arg" node /harness/pi-harness.mjs || status=$?
          mkdir -p "$preserve"
          rsync -a --exclude=/pi-sessions "$run/trace/" "$preserve/"
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
