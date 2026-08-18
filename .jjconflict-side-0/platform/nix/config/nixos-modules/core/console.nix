{pkgs, ...}: {
  console = {
    keyMap = "us-acentos";
    font = "ter-132n";
    packages = [pkgs.terminus_font];
    earlySetup = true;
  };
}
