{
  pkgs,
  rustToolchain,
  v8,
  rustcUnchecked,
}: let
  common = ''
    # Threaded-wasm build: stable cargo honors the [unstable] build-std table
    # in rust/.cargo/config.toml only with this in its environment.
    export RUSTC_BOOTSTRAP=1

    # Bounds-check-free wasm builds (nix/unchecked-rustc.nix). Exported here,
    # not inherited, so `nix run` outside the devshell builds the same module.
    export RUSTC=${rustcUnchecked}/bin/rustc-unchecked

    locate_project() {
      local git_root
      git_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
      if [[ -z "$git_root" ]]; then
        echo "could not find vip9r git checkout" >&2
        exit 2
      fi

      if [[ -f "$git_root/rust/Cargo.toml" ]]; then
        project_root="$git_root"
        rust_root="$git_root/rust"
      elif [[ -f "$git_root/Cargo.toml" ]]; then
        rust_root="$git_root"
        project_root="''${git_root%/*}"
      else
        echo "could not find vip9r Cargo workspace from $git_root" >&2
        exit 2
      fi
    }

    select_wasm_tests_target_dir() {
      # The default and wasm-tests feature builds produce the same cdylib name.
      # Keep the feature build separate from the ambient release artifact used
      # by the web dev server and golden/perf wrappers.
      target_dir="$rust_root/target/wasm-tests"
      wasm_path="$target_dir/wasm32-unknown-unknown/release/vip9r.wasm"
    }

    require_runner() {
      if [[ ! -f "$1" ]]; then
        echo "missing prebuilt wasm driver: $1" >&2
        echo "run: cd <project>/js && pnpm build:wasm-driver" >&2
        exit 2
      fi
    }
  '';
in {
  wasmTests = pkgs.writeShellApplication {
    name = "wasm-tests";
    runtimeInputs = [pkgs.git rustToolchain];
    text = ''
      set -euo pipefail

      for arg in "$@"; do
        if [[ "$arg" == "-h" || "$arg" == "--help" ]]; then
          echo "usage: wasm-tests [--json] [TEST_SUBSTRING]" >&2
          exit 0
        fi
      done

      ${common}
      locate_project
      select_wasm_tests_target_dir
      runner="$project_root/js/dist/wasm-driver/tests.js"
      require_runner "$runner"

      # cd: cargo resolves .cargo/config.toml (target-feature flags) from cwd,
      # not --manifest-path.
      (cd "$rust_root" && cargo build --target-dir "$target_dir" \
        --target wasm32-unknown-unknown -p vip9r --release --features wasm-tests)
      exec "${v8.linux64}/d8" --module "$runner" -- "$wasm_path" "$@"
    '';
  };

  wasmMicrobench = pkgs.writeShellApplication {
    name = "wasm-microbench";
    runtimeInputs = [pkgs.git rustToolchain];
    text = ''
      set -euo pipefail

      show_help=0
      for arg in "$@"; do
        if [[ "$arg" == "-h" || "$arg" == "--help" ]]; then
          show_help=1
        fi
      done

      ${common}
      locate_project
      wasm_path="$rust_root/target/wasm32-unknown-unknown/release/vip9r.wasm"
      runner="$project_root/js/dist/wasm-driver/microbench.js"
      require_runner "$runner"
      if [[ "$show_help" -eq 1 ]]; then
        exec "${v8.linux64}/d8" --no-liftoff --module "$runner" -- --help
      fi

      # cd: cargo resolves .cargo/config.toml (target-feature flags) from cwd,
      # not --manifest-path.
      (cd "$rust_root" && cargo build --target wasm32-unknown-unknown -p vip9r --release)
      exec "${v8.linux64}/d8" --no-liftoff --module "$runner" -- "$wasm_path" "$@"
    '';
  };

  wasmGolden = pkgs.writeShellApplication {
    name = "wasm-golden";
    runtimeInputs = [pkgs.git rustToolchain];
    text = ''
      set -euo pipefail

      show_help=0
      for arg in "$@"; do
        if [[ "$arg" == "-h" || "$arg" == "--help" ]]; then
          show_help=1
        fi
      done

      ${common}
      locate_project
      wasm_path="$rust_root/target/wasm32-unknown-unknown/release/vip9r.wasm"
      runner="$project_root/js/dist/wasm-driver/golden.js"
      require_runner "$runner"
      if [[ "$show_help" -eq 1 ]]; then
        exec "${v8.linux64}/d8" --no-liftoff --module "$runner" -- --help
      fi

      # cd: cargo resolves .cargo/config.toml (target-feature flags) from cwd,
      # not --manifest-path.
      (cd "$rust_root" && cargo build --target wasm32-unknown-unknown -p vip9r --release)
      exec "${v8.linux64}/d8" --no-liftoff --module "$runner" -- "$wasm_path" "$@"
    '';
  };
}
