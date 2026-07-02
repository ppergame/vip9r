{pkgs}: let
  inherit (pkgs) lib;

  android = pkgs.pkgsCross.aarch64-android-prebuilt;
  android32 = pkgs.pkgsCross.armv7a-android-prebuilt;

  commonConfigureFlags = [
    "--prefix=${placeholder "out"}"
    "--bindir=${placeholder "out"}/bin"

    "--disable-everything"
    "--disable-autodetect"
    "--disable-doc"
    "--disable-debug"
    "--disable-network"
    "--disable-shared"
    "--enable-static"
    "--enable-pic"
    "--enable-pthreads"
    "--enable-runtime-cpudetect"
    "--enable-optimizations"

    "--enable-ffmpeg"
    "--disable-ffplay"
    "--disable-ffprobe"
    "--disable-avdevice"
    "--disable-swresample"
    "--disable-swscale"

    "--enable-decoder=vp9"
    "--enable-demuxer=ivf"
    "--enable-demuxer=matroska"
    "--enable-protocol=file"
    "--enable-protocol=pipe"

    "--enable-muxer=rawvideo"
    "--enable-muxer=yuv4mpegpipe"
    "--enable-encoder=rawvideo"
    "--enable-encoder=wrapped_avframe"

    "--disable-hwaccels"
    "--disable-mediacodec"
    "--disable-jni"
  ];

  mkFfmpegVp9 = {
    pname,
    stdenv,
    nativeBuildInputs,
    targetConfigureFlags,
    preConfigure ? "",
    dontPatchELF ? false,
    noAuditTmpdir ? false,
    platforms,
  }:
    stdenv.mkDerivation {
      inherit pname preConfigure dontPatchELF noAuditTmpdir;
      version = pkgs.ffmpeg.version;

      src = pkgs.ffmpeg.src;

      strictDeps = true;
      inherit nativeBuildInputs;

      dontDisableStatic = true;
      configurePlatforms = [];
      configureFlags = commonConfigureFlags ++ targetConfigureFlags;

      buildFlags = ["ffmpeg"];

      installPhase = ''
        runHook preInstall

        install -Dm755 ffmpeg "$out/bin/ffmpeg"
        install -d "$out/share/${pname}"
        if [ -f ffbuild/config.log ]; then
          install -Dm644 ffbuild/config.log "$out/share/${pname}/config.log"
        fi
        cat > "$out/share/${pname}/usage.txt" <<'EOF'
Example decode commands:

  ffmpeg -threads 1 -i input.webm -f yuv4mpegpipe -pix_fmt yuv420p output.y4m
  ffmpeg -threads 1 -i input.ivf -f rawvideo -pix_fmt yuv420p output.yuv

Use yuv4mpegpipe when the collector wants dimensions/frame boundaries in-band.
Use rawvideo when width/height/pixel format are already known out-of-band.
EOF

        runHook postInstall
      '';

      dontStrip = false;
      enableParallelBuilding = true;

      meta = {
        description = "Minimal FFmpeg CLI with native VP9 decode, IVF/WebM input, and raw YUV output";
        homepage = "https://ffmpeg.org/";
        license = lib.licenses.lgpl21Plus;
        mainProgram = "ffmpeg";
        inherit platforms;
      };
    };
in {
  linux64 = mkFfmpegVp9 {
    pname = "ffmpeg-linux64-vp9";
    stdenv = pkgs.stdenv;
    nativeBuildInputs = [
      pkgs.nasm
      pkgs.pkg-config
    ];
    targetConfigureFlags = [
      "--target-os=linux"
      "--arch=x86_64"
      "--cpu=generic"
      "--pkg-config=${lib.getExe pkgs.pkg-config}"
      "--x86asmexe=${lib.getExe pkgs.nasm}"
    ];
    platforms = ["x86_64-linux"];
  };

  androidArm64 = mkFfmpegVp9 {
    pname = "ffmpeg-android-arm64-vp9";
    stdenv = android.stdenv;
    nativeBuildInputs = [
      android.buildPackages.pkg-config
    ];
    preConfigure = ''
      configureFlagsArray+=("--extra-ldexeflags=-static -no-pie -Wl,-z,max-page-size=16384 -Wl,-z,common-page-size=16384")
    '';
    targetConfigureFlags = [
      "--target-os=android"
      "--arch=aarch64"
      "--cpu=generic"
      "--cross-prefix=${android.stdenv.cc.targetPrefix}"
      "--cc=${android.stdenv.cc.targetPrefix}clang"
      "--cxx=${android.stdenv.cc.targetPrefix}clang++"
      "--host-cc=${pkgs.stdenv.cc}/bin/cc"
      "--pkg-config=${android.buildPackages.pkg-config}/bin/${android.buildPackages.pkg-config.targetPrefix}pkg-config"
      "--enable-cross-compile"
    ];
    dontPatchELF = true;
    noAuditTmpdir = true;
    platforms = ["aarch64-linux"];
  };

  androidArm32 = mkFfmpegVp9 {
    pname = "ffmpeg-android-arm32-vp9";
    stdenv = android32.stdenv;
    nativeBuildInputs = [
      android32.buildPackages.pkg-config
    ];
    preConfigure = ''
      configureFlagsArray+=("--extra-ldexeflags=-static -no-pie")
    '';
    targetConfigureFlags = [
      "--target-os=android"
      "--arch=arm"
      "--cpu=armv7-a"
      "--cross-prefix=${android32.stdenv.cc.targetPrefix}"
      "--cc=${android32.stdenv.cc.targetPrefix}clang"
      "--cxx=${android32.stdenv.cc.targetPrefix}clang++"
      "--host-cc=${pkgs.stdenv.cc}/bin/cc"
      "--pkg-config=${android32.buildPackages.pkg-config}/bin/${android32.buildPackages.pkg-config.targetPrefix}pkg-config"
      "--enable-cross-compile"
    ];
    dontPatchELF = true;
    noAuditTmpdir = true;
    platforms = ["armv7a-linux"];
  };
}
