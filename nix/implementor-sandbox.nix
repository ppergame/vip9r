{
  pkgs,
  rustToolchain,
  v8,
}: let
  inherit (pkgs) lib;

  bash = lib.getExe pkgs.bashInteractive;
  env = lib.getExe' pkgs.coreutils "env";
  sandboxPath = lib.makeBinPath [
    pkgs.bashInteractive
    pkgs.binaryen
    pkgs.coreutils
    pkgs.git
    pkgs.libvpx
    pkgs.nodejs
    pkgs.ripgrep
    rustToolchain
    pkgs.wabt
    pkgs.wasm-tools
  ];
  certBundle = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  setenv = name: value: ["--setenv" name value];
  symlink = source: dest: ["--symlink" source dest];
  roBindTry = path: ["--ro-bind-try" path path];
  staticBwrapArgs =
    lib.escapeShellArgs
    (["--die-with-parent" "--clearenv" "--chdir" "/run/rust" "--ro-bind" "/nix" "/nix"]
      ++ setenv "PATH" "${sandboxPath}:/bin:/usr/bin"
      ++ setenv "HOME" "/run/home"
      ++ setenv "SHELL" "/bin/bash"
      ++ setenv "TMPDIR" "/tmp"
      ++ setenv "XDG_CACHE_HOME" "/run/home/.cache"
      ++ setenv "V8_LINUX64" "${v8.linux64}"
      ++ setenv "V8_ANDROID_ARM32" "${v8.androidArm32}"
      ++ setenv "V8_ANDROID_ARM64" "${v8.androidArm64}"
      ++ setenv "D8_LINUX64" "${v8.linux64}/d8"
      ++ setenv "D8_ANDROID_ARM32" "${v8.androidArm32}/d8"
      ++ setenv "D8_ANDROID_ARM64" "${v8.androidArm64}/d8"
      ++ setenv "SSL_CERT_FILE" certBundle
      ++ setenv "NIX_SSL_CERT_FILE" certBundle
      ++ setenv "NODE_EXTRA_CA_CERTS" certBundle
      ++ ["--tmpfs" "/tmp" "--proc" "/proc" "--dev" "/dev"]
      ++ ["--dir" "/bin" "--dir" "/usr" "--dir" "/usr/bin" "--dir" "/etc" "--dir" "/etc/ssl"]
      ++ symlink bash "/bin/bash"
      ++ symlink bash "/bin/sh"
      ++ symlink env "/usr/bin/env"
      ++ roBindTry "/etc/resolv.conf"
      ++ roBindTry "/etc/hosts"
      ++ roBindTry "/etc/nsswitch.conf"
      ++ roBindTry "/etc/ssl/certs");
in
  pkgs.writeShellApplication {
    name = "implementor-sandbox";
    runtimeInputs = [
      pkgs.bubblewrap
      pkgs.coreutils
      pkgs.git
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
      mkdir -p "$run/home"

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

      bwrap_args=(
        ${staticBwrapArgs}
        --bind "$run" /run
        --bind "$auth_dir" /auth
        --ro-bind "$repo/docs/specs" /specs
        --ro-bind "$harness_dir" /harness
      )

      status=0
      case "$mode" in
        run)
          bwrap "''${bwrap_args[@]}" node /harness/pi-harness.mjs || status=$?
          ;;
        shell)
          bwrap "''${bwrap_args[@]}" "${bash}" -i || status=$?
          ;;
      esac

      echo "source: $run/rust" >&2
      exit "$status"
    '';
  }
