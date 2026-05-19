{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:

rustPlatform.buildRustPackage rec {
  pname = "logq";
  version = "0.1.7";

  src = fetchFromGitHub {
    owner = "ushitora-anqou";
    repo = "logq";
    rev = version;
    hash = "sha256-lqK37EqX5oq3GHDYW7lxQk4NpDLmeb+TqPN4HSjmdMA=";
  };

  cargoHash = "sha256-DrayucDCY2FC80Kk2qiiwpSp4p6GrRI1+O0yutBjaek=";

  checkFlags = [
    "--skip=test_tui_mode_with_command_no_panic"
  ];

  meta = {
    description = "A terminal UI viewer for NDJSON and plain text streams";
    homepage = "https://github.com/ushitora-anqou/logq";
    license = lib.licenses.mit;
    mainProgram = "logq";
  };
}
