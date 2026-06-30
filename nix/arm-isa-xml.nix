{pkgs}: let
  fetchArmIsa = {
    name,
    url,
    hash,
  }:
    pkgs.fetchurl {
      inherit name url hash;
    };

  a64 = fetchArmIsa {
    name = "ISA_A64_xml_A_profile-2026-03_96.tar.gz";
    url = "https://developer.arm.com/-/cdn-downloads/permalink/Exploration-Tools-A64-ISA/ISA_A64/ISA_A64_xml_A_profile-2026-03_96.tar.gz";
    hash = "sha256-nROKpN1tuLBL1q06N4IFU3v2cXKl+ZGAcpUX0URfGQY=";
  };

  aarch32 = fetchArmIsa {
    name = "ISA_AArch32_xml_A_profile-2026-03_96.tar.gz";
    url = "https://developer.arm.com/-/cdn-downloads/permalink/Exploration-Tools-AArch32-ISA/ISA_AArch32/ISA_AArch32_xml_A_profile-2026-03_96.tar.gz";
    hash = "sha256-mN1OFuZ0R9MTQ4i3k8OOUGOO0y4kuQAPSJELoF0PtD0=";
  };

  copyXmlSet = {
    archive,
    sourceDir,
    targetDir,
  }: ''
    tar -xzf ${archive}
    install -d "$out/${targetDir}"
    cp ${sourceDir}/*.xml "$out/${targetDir}/"
    cp ${sourceDir}/*.dtd "$out/${targetDir}/"
  '';
in
  pkgs.runCommand "arm-a-profile-isa-xml-2026-03_96" {
    nativeBuildInputs = [pkgs.gnutar pkgs.gzip];
  } ''
    mkdir -p "$out"
    ${copyXmlSet {
      archive = a64;
      sourceDir = "ISA_A64_xml_A_profile_2026-03_96-2026-03_rel";
      targetDir = "a64";
    }}
    ${copyXmlSet {
      archive = aarch32;
      sourceDir = "ISA_AArch32_xml_A_profile_2026-03_96-2026-03_rel";
      targetDir = "aarch32-t32";
    }}
  ''
