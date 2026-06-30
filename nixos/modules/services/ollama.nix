{pkgs, ...}: {
  services.ollama = {
    enable = false;
    package = pkgs.ollama-rocm;
    rocmOverrideGfx = "11.0.2";
  };
}
