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
