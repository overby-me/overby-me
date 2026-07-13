{
  devenvConfigurations.wiki-backend = {
    pkgs,
    inputs,
    ...
  }: {
    imports = with inputs.self.devenvModules; [
      devenv-root
    ];

    languages = {
      rust = {
        enable = true;
      };
    };

    packages = with pkgs; [
      openssl
      # `scw`, used by `just deploy` to ship the backend to Scaleway Functions.
      scaleway-cli
    ];
  };
}
