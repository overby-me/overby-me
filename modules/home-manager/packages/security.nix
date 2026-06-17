{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    (pynitrokey.overridePythonAttrs (old: {
      dependencies = (old.dependencies or []) ++ pynitrokey.optional-dependencies.pcsc;
    }))
    nitrokey-app2
    age-plugin-fido2prf
    pcsc-tools
  ];
}
