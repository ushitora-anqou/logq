{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:

rustPlatform.buildRustPackage rec {
  pname = "logq";
  version = "0.1.8";

  src = fetchFromGitHub {
    owner = "ushitora-anqou";
    repo = "logq";
    rev = version;
    hash = "sha256-yrQKZ12IUXpgtv0UbgbtQKD41b9ptJkL89KeXNZXvCE=";
  };

  cargoHash = "sha256-Lc/xCEsbB/I/vl8Odk0C6zqbQX/q5UQO3LpCYH8BT5M=";

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
