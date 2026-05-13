{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:

rustPlatform.buildRustPackage rec {
  pname = "logq";
  version = "0.1.4";

  src = fetchFromGitHub {
    owner = "ushitora-anqou";
    repo = "logq";
    rev = version;
    hash = "sha256-jyAnuKf3+LsRYo1map4Dfk3fKFJinssF+WJJd58mlL8=";
  };

  cargoHash = "sha256-f5HMCh++aj9VSIn92UJukxdPrcGioaqkV3C7pcgp7zI=";

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
