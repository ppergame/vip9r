{
  description = "vip9r";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    rust-overlay,
    ...
  }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [rust-overlay.overlays.default];
      config.android_sdk.accept_license = true;
      config.allowUnfreePredicate = pkg: let
        name = nixpkgs.lib.getName pkg;
      in
        name == "android-sdk-ndk"
        || name == "ndk"
        || nixpkgs.lib.hasPrefix "system-image-" name
        || nixpkgs.lib.hasPrefix "aarch64-unknown-linux-android-" name
        || nixpkgs.lib.hasPrefix "android-sdk-" name
        || nixpkgs.lib.hasPrefix "android-ndk" name
        || nixpkgs.lib.hasPrefix "platform-tools" name;
    };
    v8 = import ./nix/v8.nix {inherit pkgs;};
    androidRuntimeRoots = import ./nix/android-runtime-roots.nix {inherit pkgs;};
    ffmpegVp9 = import ./nix/ffmpeg-vp9.nix {inherit pkgs;};
    armIsaXml = import ./nix/arm-isa-xml.nix {inherit pkgs;};
    codex = import ./nix/codex.nix {inherit pkgs;};
    rustToolchain =
      pkgs.rust-bin.stable.latest.default.override
      {
        extensions = [
          "llvm-tools-preview"
          "rust-analyzer"
          "rust-src"
        ];
        targets = [
          "x86_64-unknown-linux-gnu"
          "wasm32-unknown-unknown"
        ];
      };
    npmUnavailable = pkgs.writeShellScriptBin "npm" ''
            cat >&2 <<'EOF'
      use pnpm
      EOF
            exit 1
    '';
    wasmTools = import ./nix/wasm-tools.nix {
      inherit pkgs rustToolchain v8;
    };
    grinder = import ./nix/grinder.nix {
      inherit pkgs rustToolchain codex armIsaXml;
    };
  in {
    packages.${system} = {
      inherit grinder;
      wasm-golden = wasmTools.wasmGolden;
      wasm-microbench = wasmTools.wasmMicrobench;
      wasm-tests = wasmTools.wasmTests;
    };

    devShells.${system}.default = pkgs.mkShell {
      V8_LINUX64 = "${v8.linux64}";
      V8_ANDROID_ARM32 = "${v8.androidArm32}";
      V8_ANDROID_ARM64 = "${v8.androidArm64}";

      D8_LINUX64 = "${v8.linux64}/d8";
      D8_ANDROID_ARM32 = "${v8.androidArm32}/d8";
      D8_ANDROID_ARM64 = "${v8.androidArm64}/d8";
      ANDROID_RUNTIME_ROOT_ARM32 = "${androidRuntimeRoots.arm32}";
      ANDROID_RUNTIME_ROOT_ARM64 = "${androidRuntimeRoots.arm64}";
      OBJDUMP_MULTIARCH = "${pkgs.binutils-unwrapped-all-targets}/bin/objdump";
      FFMPEG_VP9_LINUX64 = "${ffmpegVp9.linux64}/bin/ffmpeg";
      FFMPEG_VP9_ANDROID_ARM64 = "${ffmpegVp9.androidArm64}/bin/ffmpeg";
      SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      NODE_EXTRA_CA_CERTS = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

      shellHook = ''
        export PATH="${npmUnavailable}/bin:$PATH"
        alias npm='${npmUnavailable}/bin/npm'
      '';

      packages = with pkgs;
        [
          android-tools
          binaryen
          bubblewrap
          cacert
          git
          grinder
          libvpx
          nodejs
          pnpm
          python3
          qemu
          ripgrep
          rustToolchain
          wabt
          wasm-tools
          which
        ]
        ++ [
          npmUnavailable
          wasmTools.wasmGolden
          wasmTools.wasmMicrobench
          wasmTools.wasmTests
        ];
    };
  };
}
