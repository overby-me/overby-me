{
  programs.readline = {
    enable = true;
    extraConfig = ''
      "\e[A":history-search-backward
      "\e[B":history-search-forward
      set completion-ignore-case On
      set completion-prefix-display-length 2
    '';
  };
}
