{pkgs, ...}: {
  programs.starship = {
    enable = true;
    settings = {
      format = "$all\${custom.jj}$line_break$jobs$battery$time$status$os$container$netns$shell$character";
      custom.jj = {
        command = "prompt";
        format = "$output";
        ignore_timeout = true;
        shell = ["${pkgs.starship-jj}/bin/starship-jj" "--ignore-working-copy" "starship"];
        use_stdin = false;
        "when" = true;
      };
      command_timeout = 10000;
      time = {
        disabled = false;
        format = " [$time]($style) ";
      };
      status = {
        disabled = false;
      };
      directory = {
        truncation_length = 8;
        truncation_symbol = ".../";
        truncate_to_repo = false;
      };
    };
  };
}
