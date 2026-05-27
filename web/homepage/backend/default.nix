{
  devenvConfigurations.backend = {
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
      #scaleway-cli
    ];
  };
}
