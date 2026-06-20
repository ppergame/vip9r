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
    };
    fetchV8 = {
      artifact,
      version,
      generation,
      hash,
    }:
      pkgs.fetchzip {
        name = "${artifact}-${version}";
        url = "https://storage.googleapis.com/chromium-v8/official/canary/${artifact}-${version}.zip?generation=${generation}";
        inherit hash;
        extension = "zip";
        stripRoot = false;
      };
    v8 = {
      linux64 = fetchV8 {
        artifact = "v8-linux64-rel";
        version = "15.1.158";
        generation = "1781886252619364";
        hash = "sha256-CaEY/v7j9uyAC8Q/zI4AnpXVFCR/BEB33Ht6RobygTw=";
      };
      androidArm32 = fetchV8 {
        artifact = "v8-android-arm32-rel";
        version = "14.1.63";
        generation = "1755083075015458";
        hash = "sha256-3wz9GMovbOS2MBCbzHK6grl/aApu8xZU1K1XfJl/6SU=";
      };
      androidArm64 = fetchV8 {
        artifact = "v8-android-arm64-rel";
        version = "14.1.63";
        generation = "1755083039326956";
        hash = "sha256-P4erFsvQ8//3wj/SD8mNrJjn5NLRNmxmH254/5bVcH4=";
      };
    };
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
  in {
    devShells.${system}.default = pkgs.mkShell {
      V8_LINUX64 = "${v8.linux64}";
      V8_ANDROID_ARM32 = "${v8.androidArm32}";
      V8_ANDROID_ARM64 = "${v8.androidArm64}";

      D8_LINUX64 = "${v8.linux64}/d8";
      D8_ANDROID_ARM32 = "${v8.androidArm32}/d8";
      D8_ANDROID_ARM64 = "${v8.androidArm64}/d8";

      packages = with pkgs; [
        binaryen
        nodejs
        rustToolchain
        wabt
        wasm-tools
      ];
    };
  };
}
