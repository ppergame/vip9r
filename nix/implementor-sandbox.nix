{
  pkgs,
  rustToolchain,
  v8,
}: let
  inherit (pkgs) lib;

  bash = lib.getExe pkgs.bashInteractive;
  env = lib.getExe' pkgs.coreutils "env";
  rootfs = pkgs.runCommand "implementor-sandbox-rootfs" {} ''
    mkdir -p $out/bin $out/usr/bin
    ln -s ${bash} $out/bin/bash
    ln -s ${bash} $out/bin/sh
    ln -s ${env} $out/usr/bin/env
  '';
  sandboxPath = lib.makeBinPath (with pkgs; [
    bashInteractive
    binaryen
    coreutils
    git
    libvpx
    nodejs
    ripgrep
    rustToolchain
    wabt
    wasm-tools
  ]);
  podmanEnv = name: value: ["--env" "${name}=${value}"];
  staticPodmanArgs =
    lib.escapeShellArgs
    (["run" "--rm" "-i" "--rootfs" "--userns=keep-id" "--unsetenv-all" "--workdir" "/run/rust"]
      ++ podmanEnv "PATH" "${sandboxPath}:/bin:/usr/bin"
      ++ podmanEnv "HOME" "/run/home"
      ++ podmanEnv "SHELL" "/bin/bash"
      ++ podmanEnv "V8_LINUX64" "${v8.linux64}"
      ++ podmanEnv "V8_ANDROID_ARM32" "${v8.androidArm32}"
      ++ podmanEnv "V8_ANDROID_ARM64" "${v8.androidArm64}"
      ++ podmanEnv "D8_LINUX64" "${v8.linux64}/d8"
      ++ podmanEnv "D8_ANDROID_ARM32" "${v8.androidArm32}/d8"
      ++ podmanEnv "D8_ANDROID_ARM64" "${v8.androidArm64}/d8"
      ++ ["--tmpfs" "/tmp" "--volume" "/nix/store:/nix/store:ro"]);
in
  pkgs.writeShellApplication {
    name = "implementor-sandbox";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.git
      pkgs.podman
    ];

    text = ''
      usage() {
        cat >&2 <<'EOF'
      usage:
        implementor-sandbox [--repo DIR] run TASK_FILE
        implementor-sandbox [--repo DIR] shell [TASK_FILE]
      EOF
      }

      repo="''${VIP9R_REPO:-$PWD}"
      if [[ "''${1:-}" == "--repo" ]]; then
        if [[ $# -lt 2 ]]; then
          usage
          exit 2
        fi
        repo="$2"
        shift 2
      fi
      repo="$(cd -- "$repo" && pwd)"

      mode="''${1:-}"
      case "$mode" in
        run|shell) shift || true ;;
        -h|--help)
          usage
          exit 0
          ;;
        *)
          usage
          exit 2
          ;;
      esac

      task_file="''${1:-}"
      if [[ "$mode" == "run" && -z "$task_file" ]]; then
        usage
        exit 2
      fi
      if [[ -n "$task_file" && ! -f "$task_file" ]]; then
        echo "task file not found: $task_file" >&2
        exit 2
      fi
      if [[ $# -gt 1 ]]; then
        usage
        exit 2
      fi

      system_prompt="$repo/scripts/implementor-system-prompt.md"
      harness_dir="$repo/js/dist/pi-harness"
      for path in "$repo/rust" "$repo/docs/specs" "$system_prompt" "$harness_dir/pi-harness.mjs"; do
        if [[ ! -e "$path" ]]; then
          echo "missing required path: $path" >&2
          exit 1
        fi
      done

      temp_dir="$repo/temp"
      auth_dir="$temp_dir/pi-auth"
      mkdir -p "$temp_dir" "$auth_dir"
      run="$(mktemp -d -p "$temp_dir" implementor.XXXXXX)"
      mkdir -p "$run/home" "$run/rootfs"
      cp -a --no-preserve=ownership "${rootfs}/." "$run/rootfs"
      chmod -R u+w "$run/rootfs"

      cp -a --reflink=auto "$repo/rust" "$run/rust"
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
        --volume "$repo/docs/specs:/specs:ro"
        --volume "/bulk/vip9r:/media:ro"
        --volume "$harness_dir:/harness:ro"
        "$run/rootfs:O"
      )

      status=0
      case "$mode" in
        run)
          podman "''${podman_args[@]}" node /harness/pi-harness.mjs || status=$?
          ;;
        shell)
          podman "''${podman_args[@]}" /bin/bash -i || status=$?
          ;;
      esac

      echo "source: $run/rust" >&2
      exit "$status"
    '';
  }
