let
  publicKeys = (import ./publicKeys.nix).all;
in {
  "resolved.age" = {
    inherit publicKeys;
  };
  "u2f-keys.age" = {
    inherit publicKeys;
  };
  "id_ed25519.age" = {
    inherit publicKeys;
  };
  "id_rsa.age" = {
    inherit publicKeys;
  };
  "wifi-concero.age" = {
    inherit publicKeys;
  };
  "spindle-token.age" = {
    inherit publicKeys;
  };
  "ironclaw-env.age" = {
    inherit publicKeys;
  };
  "searxng-env.age" = {
    inherit publicKeys;
  };
  "stalwart-admin-password.age" = {
    inherit publicKeys;
  };
}
