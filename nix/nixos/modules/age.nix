{pkgs, ...}: let
  rage-with-plugins = pkgs.symlinkJoin {
    name = "rage-with-plugins";
    paths = [pkgs.rage];
    buildInputs = [pkgs.makeWrapper];
    postBuild = ''
      wrapProgram $out/bin/rage --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.age-plugin-fido2-hmac]}
    '';
  };
in {
  age = {
    ageBin = "${rage-with-plugins}/bin/rage";
    # Try unattended SSH host keys first so boot-time decryption never
    # prompts for the FIDO2 token. The fido2_host_key is kept last as a
    # fallback for hosts/secrets where SSH host keys aren't recipients.
    identityPaths = ["/etc/ssh/ssh_host_ed25519_key" "/etc/ssh/ssh_host_rsa_key" "/etc/age/fido2_host_key"];
  };
  environment = {
    systemPackages = with pkgs; [
      age-plugin-fido2-hmac
    ];
  };
}
