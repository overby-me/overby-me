final: prev: {
  ragenix = prev.inputs.ragenix.packages.${prev.stdenv.hostPlatform.system}.default.override {
    plugins = [final.age-plugin-fido2-hmac];
  };
}
