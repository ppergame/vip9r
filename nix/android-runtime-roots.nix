# Minimal Android runtime roots for running Android d8 under qemu-user.
#
# Each root provides exactly what the Android dynamic linker needs to start a
# PIE Android ELF: /system/bin/linker{,64} plus Bionic libc/libdl/libm. The
# files are extracted from pinned Android SDK emulator system images; NDK
# prebuilts are not a substitute because they lack the linker itself.
#
# Consumers bind the outputs into a sandbox, e.g.:
#   bwrap --ro-bind $root/system /system [--ro-bind $root/apex /apex] ...
{pkgs}: let
  systemImage = args: attrs:
    builtins.head (pkgs.androidenv.composeAndroidPackages ({
        includeSystemImages = true;
      }
      // args))
    .system-images
    + "/libexec/android-sdk/system-images/${attrs.path}/system.img";

  # The arm32 d8 is linked --hash-style=gnu, so the Bionic linker must be
  # API >= 23 (M added DT_GNU_HASH). Bionic >= 23 also requires a working
  # personality(2); see the perf daemon's seccomp shim.
  arm32SystemImg = systemImage {
    platformVersions = ["24"];
    abiVersions = ["armeabi-v7a"];
    systemImageTypes = ["default"];
  } {path = "android-24/default/armeabi-v7a";};

  # Modern arm64 images keep the runtime linker inside
  # com.android.runtime.apex within the dynamic "super" partition.
  arm64SystemImg = systemImage {
    platformVersions = ["36"];
    abiVersions = ["arm64-v8a"];
    systemImageTypes = ["google_apis"];
  } {path = "android-36/google_apis/arm64-v8a";};
in {
  arm32 = pkgs.runCommand "android-runtime-root-arm32-api24" {
    nativeBuildInputs = [pkgs.e2fsprogs];
  } ''
    require() { [ -s "$1" ] || { echo "missing or empty: $1" >&2; exit 1; }; }

    mkdir -p "$out/system/bin" "$out/system/lib"
    debugfs -R "dump -p /bin/linker $out/system/bin/linker" ${arm32SystemImg}
    for lib in libc.so libdl.so libm.so; do
      debugfs -R "dump -p /lib/$lib $out/system/lib/$lib" ${arm32SystemImg}
    done

    require "$out/system/bin/linker"
    chmod 555 "$out/system/bin/linker"
    for lib in libc.so libdl.so libm.so; do
      require "$out/system/lib/$lib"
      chmod 444 "$out/system/lib/$lib"
    done
  '';

  arm64 = pkgs.runCommand "android-runtime-root-arm64-api36" {
    nativeBuildInputs = with pkgs; [android-tools e2fsprogs jq unzip util-linux];
  } ''
    require() { [ -s "$1" ] || { echo "missing or empty: $1" >&2; exit 1; }; }

    img=${arm64SystemImg}
    start=$(sfdisk --json "$img" | jq -r '.partitiontable.partitions[] | select(.name == "super") | .start')
    size=$(sfdisk --json "$img" | jq -r '.partitiontable.partitions[] | select(.name == "super") | .size')
    [ -n "$start" ] && [ -n "$size" ] || { echo "no super partition in $img" >&2; exit 1; }
    dd if="$img" of=super.img bs=1M iflag=skip_bytes,count_bytes \
      skip=$((start * 512)) count=$((size * 512))

    lpunpack -p system super.img .
    require system.img
    debugfs -R "dump -p /system/apex/com.android.runtime.apex runtime.apex" system.img
    require runtime.apex
    unzip -p runtime.apex apex_payload.img > apex_payload.img
    require apex_payload.img

    runtime="$out/apex/com.android.runtime"
    mkdir -p "$runtime/bin" "$runtime/lib64/bionic" "$out/system/bin" "$out/system/lib64"
    debugfs -R "dump -p /bin/linker64 $runtime/bin/linker64" apex_payload.img
    for lib in libc.so libdl.so libm.so; do
      debugfs -R "dump -p /lib64/bionic/$lib $runtime/lib64/bionic/$lib" apex_payload.img
    done

    require "$runtime/bin/linker64"
    chmod 555 "$runtime/bin/linker64"
    for lib in libc.so libdl.so libm.so; do
      require "$runtime/lib64/bionic/$lib"
      chmod 444 "$runtime/lib64/bionic/$lib"
    done

    # Legacy /system fallback paths; some binaries resolve the linker there.
    cp "$runtime/bin/linker64" "$out/system/bin/linker64"
    cp "$runtime/lib64/bionic/"*.so "$out/system/lib64/"
  '';
}
