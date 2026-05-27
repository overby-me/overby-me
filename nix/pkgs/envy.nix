{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage {
  pname = "envy";
  version = "0.6.0";

  src = fetchFromGitHub {
    owner = "mre";
    repo = "envy";
    rev = "45a68eb1e944c3383ac443df23a0ff6827140910";
    hash = "sha256-2NIfAKx80sX7zqrCIMe2L6+LvQPtTWxQQmWMMpD0zAY=";
  };

  buildFeatures = ["bash-support"];

  cargoHash = "sha256-U8IBel3jDgIaHROfeNn8gAF3nQTxSJGb4gEqiwX39Dk=";

  doCheck = false;

  meta = {
    description = "Manage environment variables without cluttering your .zshrc";
    homepage = "https://github.com/mre/envy";
    license = with lib.licenses; [asl20 mit];
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "envy";
  };
}
