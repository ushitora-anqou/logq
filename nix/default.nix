{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:

rustPlatform.buildRustPackage rec {
  pname = "logq";
  version = "0.1.3";

  src = fetchFromGitHub {
    owner = "ushitora-anqou";
    repo = "logq";
    rev = version;
    hash = "sha256-+fKi7vfE0CHagSOf9g9TRqIXrSQo35xjIWuQAk0RZoI=";
  };

  cargoHash = "sha256-yft92g499CsKstqm3gXhfhwb6m0h8XrdiJC9v08xymA=";

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
