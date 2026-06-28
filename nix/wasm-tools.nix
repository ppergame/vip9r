{
  pkgs,
  rustToolchain,
  v8,
}: let
  common = ''
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

      # The default and wasm-tests feature builds produce the same cdylib name.
      # Keep their Cargo caches separate so alternating wrappers does not rebuild.
      target_base="$rust_root/target"
    }

    select_wasm_target_dir() {
      target_dir="$target_base/$1"
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

      if [[ $# -eq 1 && ( "$1" == "-h" || "$1" == "--help" ) ]]; then
        echo "usage: wasm-tests [TEST_SUBSTRING]" >&2
        exit 0
      fi

      if [[ $# -gt 1 ]]; then
        echo "usage: wasm-tests [TEST_SUBSTRING]" >&2
        exit 2
      fi

      ${common}
      locate_project
      select_wasm_target_dir wasm-tests
      runner="$project_root/js/dist/wasm-driver/tests.js"
      require_runner "$runner"

      cargo build --manifest-path "$rust_root/Cargo.toml" --target-dir "$target_dir" \
        --target wasm32-unknown-unknown -p vip9r --release --features wasm-tests
      if [[ $# -eq 1 ]]; then
        exec "${v8.linux64}/d8" "$runner" -- "$wasm_path" "$1"
      fi
      exec "${v8.linux64}/d8" "$runner" -- "$wasm_path"
    '';
  };

  wasmGolden = pkgs.writeShellApplication {
    name = "wasm-golden";
    runtimeInputs = [pkgs.git rustToolchain];
    text = ''
      set -euo pipefail

      allow_mismatch=0
      if [[ $# -gt 0 && "$1" == "--allow-mismatch" ]]; then
        allow_mismatch=1
        shift
      fi
      if [[ $# -gt 1 ]]; then
        echo "usage: wasm-golden [--allow-mismatch] [INPUT_IVF_OR_WEBM]" >&2
        exit 2
      fi

      ${common}
      locate_project
      select_wasm_target_dir wasm-golden
      runner="$project_root/js/dist/wasm-driver/golden.js"
      input="''${1:-/bulk/vip9r/chromium/bear-vp9.ivf}"
      require_runner "$runner"

      cargo build --manifest-path "$rust_root/Cargo.toml" --target-dir "$target_dir" \
        --target wasm32-unknown-unknown -p vip9r --release
      if [[ "$allow_mismatch" -eq 1 ]]; then
        exec "${v8.linux64}/d8" "$runner" -- --allow-mismatch "$wasm_path" "$input"
      fi
      exec "${v8.linux64}/d8" "$runner" -- "$wasm_path" "$input"
    '';
  };
}
