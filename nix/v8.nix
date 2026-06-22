{pkgs}: let
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

  patchLinuxD8 = source:
    pkgs.runCommand "${source.name}-nix-friendly" {
      nativeBuildInputs = [pkgs.patchelf];
    } ''
      mkdir -p "$out"
      cp -R ${source}/. "$out/"
      chmod -R u+w "$out"

      patchelf \
        --set-interpreter ${pkgs.stdenv.cc.bintools.dynamicLinker} \
        --set-rpath ${pkgs.lib.makeLibraryPath [
        pkgs.glibc
        pkgs.stdenv.cc.cc.lib
      ]} \
        "$out/d8"
    '';
in {
  linux64 = patchLinuxD8 (fetchV8 {
    artifact = "v8-linux64-rel";
    version = "15.1.159";
    generation = "1782060411010946";
    hash = "sha256-bBQvffIseJ4gE8gicvoFxe40dRRHK0JYraUSPRh9eXI=";
  });

  androidArm32 = fetchV8 {
    artifact = "v8-android-arm32-rel";
    version = "15.1.159";
    generation = "1782060443449205";
    hash = "sha256-5kpcZKF9h7XemUk7Cnf+mEg4R8Ghx4P0HLlrd5uvtyw=";
  };

  androidArm64 = fetchV8 {
    artifact = "v8-android-arm64-rel";
    version = "15.1.159";
    generation = "1782060481347578";
    hash = "sha256-kgFkrVYr/z7tQ/hUiFhLiZBKipJ0xkSfvIqXK2QPSNQ=";
  };
}
