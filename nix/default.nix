{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:

rustPlatform.buildRustPackage rec {
  pname = "logq";
  version = "0.1.5";

  src = fetchFromGitHub {
    owner = "ushitora-anqou";
    repo = "logq";
    rev = version;
    hash = "sha256-QXSvyT/WuFKD8DISTd5ujnnj/NwLMuNZNWXdeAxPrgk=";
  };

  cargoHash = "sha256-8nDCrN2sJ4kmWWGCCoT7JZF8aUtXILJFoUblKiTQ6GA=";

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
