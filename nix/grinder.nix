{
  pkgs,
  rustToolchain,
  codex,
  armIsaXml,
}: let
  inherit (pkgs) lib;

  bash = lib.getExe pkgs.bashInteractive;
  caBundle = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  env = lib.getExe' pkgs.coreutils "env";
  fuseOverlayfs = lib.getExe pkgs.fuse-overlayfs;
  setsid = lib.getExe' pkgs.util-linux "setsid";
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
      pkg-config
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
      # Threaded-wasm build: stable cargo honors the [unstable] build-std
      # table in rust/.cargo/config.toml only with this in its environment.
      ++ podmanEnv "RUSTC_BOOTSTRAP" "1"
      ++ podmanEnv "CODEX_HOME" "/codex-home"
      ++ podmanEnv "SSL_CERT_FILE" caBundle
      ++ podmanEnv "NIX_SSL_CERT_FILE" caBundle
      ++ ["--tmpfs" "/tmp" "--volume" "/nix/store:/nix/store:ro"]);
in
  pkgs.writeShellApplication {
    name = "grinder";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.git
      pkgs.podman
      pkgs.python3
    ];

    text = ''
      usage() {
        printf '%s\n' \
          'usage:' \
          '  grinder run TASK_FILE --baseline WASM [--device INDEX:PIN]' \
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
        exec "$repo/scripts/codex-events.py" inspect --repo "$repo" "$@"
      fi

      task_file=""
      baseline_file=""
      device_default=""
      command=()
      case "$mode" in
        run)
          task_file="''${1:-}"
          if [[ -z "$task_file" ]]; then
            usage
            exit 2
          fi
          shift
          while [[ $# -gt 0 ]]; do
            case "$1" in
              --baseline)
                baseline_file="''${2:-}"
                shift 2 || { usage; exit 2; }
                ;;
              --device)
                device_default="''${2:-}"
                shift 2 || { usage; exit 2; }
                ;;
              *)
                usage
                exit 2
                ;;
            esac
          done
          # The modal task benches; a run without a stated baseline is an
          # orchestrator mistake best caught at spawn, not mid-session.
          if [[ -z "$baseline_file" ]]; then
            echo "grinder: run requires --baseline WASM" >&2
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
      if [[ -n "$baseline_file" && ! -f "$baseline_file" ]]; then
        echo "baseline file not found: $baseline_file" >&2
        exit 2
      fi
      # Core choice is the orchestrator's call, not the grinder's: a device
      # grant always names its pin.
      if [[ -n "$device_default" && ! "$device_default" =~ ^[0-9]+:.+$ ]]; then
        echo "grinder: --device must be INDEX:PIN" >&2
        exit 2
      fi

      system_prompt="$repo/scripts/grinder-system-prompt.md"
      codex_config="$repo/scripts/grinder-codex-config.toml"
      codex_events="$repo/scripts/codex-events.py"
      perf_submit="$repo/scripts/vip9r-perf-submit.py"
      # Bind the socket's directory, not the socket file: a daemon restart
      # replaces the inode, and a file bind would leave a running sandbox
      # holding the dead socket.
      perf_socket_dir="$repo/temp/perf"

      temp_dir="$repo/temp"
      codex_home="$temp_dir/codex-home"
      cargo_home="$temp_dir/cargo-home"
      mkdir -p "$temp_dir" "$codex_home" "$cargo_home"
      [[ -e "$codex_home/config.toml" ]] || touch "$codex_home/config.toml"
      run="$(mktemp -d -p "$temp_dir" grinder.XXXXXX)"
      mkdir -p "$run/codex-state" "$run/home" "$run/rootfs/bin" "$run/rootfs/usr/bin" "$run/rootfs/bulk/vip9r" "$run/rootfs/specs/arm-isa" "$run/trace"
      git config --file "$run/home/.gitconfig" user.name vip9r-implementor
      git config --file "$run/home/.gitconfig" user.email vip9r-implementor@example.invalid
      ln -s "${sandboxEnv}" "$run/tools"
      ln -s "${bash}" "$run/rootfs/bin/bash"
      ln -s "${bash}" "$run/rootfs/bin/sh"
      ln -s "${env}" "$run/rootfs/usr/bin/env"
      cp -a "$perf_submit" "$run/rootfs/usr/bin/vip9r-perf-submit"

      mkdir -p "$run/rust"
      shopt -s dotglob nullglob
      rust_entries=("$repo/rust"/*)
      shopt -u dotglob nullglob
      for entry in "''${rust_entries[@]}"; do
        [[ "$(basename "$entry")" == target ]] && continue
        cp -a "$entry" "$run/rust/"
      done
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
        --volume "${armIsaXml}:/specs/arm-isa:ro"
        --volume "/bulk/vip9r:/bulk/vip9r:ro"
      )
      mkdir -p "$perf_socket_dir"
      podman_args+=(--volume "$perf_socket_dir:/run/vip9r-perf:rw")
      # Default perf-submit routing for the sandbox: bench baseline and
      # device index:pin ride in as env, overridable per submission.
      if [[ -n "$baseline_file" ]]; then
        cp -a "$baseline_file" "$run/baseline.wasm"
        podman_args+=(--env VIP9R_PERF_BASELINE=/run/baseline.wasm)
      fi
      if [[ -n "$device_default" ]]; then
        podman_args+=(--env VIP9R_PERF_DEVICE="$device_default")
      fi
      # Name the container after the sandbox so it can be stopped by name;
      # TaskStop/SIGTERM on this wrapper must not orphan a running codex.
      container_name="$(basename "$run")"
      podman_args+=(--name "$container_name")
      rootfs_arg="$run/rootfs:O"

      status=0
      case "$mode" in
        run)
          # run mode deliberately leaves "$run" intact: the orchestrator reviews
          # and merges from "$run/rust" before reaping it. "$preserve" is the
          # durable record that outlives "$run".
          preserve="$temp_dir/traces/$(date -u +%Y%m%dT%H%M%SZ)-$(basename "$run")"
          echo "trace -> $preserve" >&2
          echo "container: $container_name" >&2
          # Stop tastefully on TERM/INT: kill the container so codex exits,
          # let the pipeline drain, and fall through to normal trace
          # preservation. The pipeline runs in the background because bash
          # defers traps while a foreground pipeline runs. The kill is
          # detached via setsid because the stop signal usually escalates to
          # KILL on this wrapper before a synchronous podman call returns; a
          # SIGKILLed wrapper then cannot orphan the container. Traces
          # stream to $run/trace continuously, so an immediate kill loses
          # nothing that a graceful stop would save.
          interrupted=0
          trap '
            interrupted=1
            echo "grinder: stop requested; killing $container_name" >&2
            ${setsid} -f podman kill "$container_name" >/dev/null 2>&1 || true
          ' TERM INT
          set +e
          {
            podman "''${podman_args[@]}" "$rootfs_arg" \
              /run/tools/bin/codex exec \
                --json \
                -o /run/trace/final.md \
                - \
                < "$run/task.md"
            echo "$?" > "$run/trace/podman-status"
          } | "$codex_events" stream "$run/trace/codex.jsonl" &
          pipeline_pid=$!
          renderer_status=0
          while :; do
            wait "$pipeline_pid"
            renderer_status=$?
            kill -0 "$pipeline_pid" 2>/dev/null || break
          done
          status="$(cat "$run/trace/podman-status" 2>/dev/null || echo 125)"
          set -e
          trap - TERM INT
          if [[ "$status" -eq 0 && "$renderer_status" -ne 0 ]]; then
            status="$renderer_status"
          fi
          if [[ "$interrupted" -ne 0 ]]; then
            echo "grinder: run stopped by request" >&2
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
            context_summary="$("$codex_events" context "$run/trace/rollout.jsonl" || true)"
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
