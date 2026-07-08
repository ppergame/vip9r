# Bounds-check-free rustc for the wasm module (docs/design.md, "Unchecked
# core"). The sysroot is a symlink farm over the stock toolchain with two core
# source files edited so slice/array OOB lowers to unreachable_unchecked();
# build-std recompiles core from $sysroot/lib/rustlib/src/rust/library, so
# LLVM deletes every bounds-check branch module-wide. The wasm sandbox is the
# containment story: OOB becomes in-module garbage instead of a trap.
#
# The edits live in disarm-core-checks.py: anchor-matched disarm-and-append,
# which fails this derivation loudly if a toolchain bump moves the anchors.
{
  pkgs,
  rustToolchain,
}: let
  sysroot = pkgs.runCommandLocal "rust-sysroot-unchecked" {
    nativeBuildInputs = [pkgs.python3];
  } ''
    mkdir $out
    cp -rs --no-preserve=mode ${rustToolchain}/* $out/
    lib=$out/lib/rustlib/src/rust/library
    for f in core/src/panicking.rs core/src/slice/index.rs; do
      rm "$lib/$f"
      cp --no-preserve=mode ${rustToolchain}/lib/rustlib/src/rust/library/$f "$lib/$f"
    done
    python3 ${./disarm-core-checks.py} "$lib"
  '';
in
  pkgs.writeShellScriptBin "rustc-unchecked" ''
    exec ${rustToolchain}/bin/rustc --sysroot ${sysroot} "$@"
  ''
