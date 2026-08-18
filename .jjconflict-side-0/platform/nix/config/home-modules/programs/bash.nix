{
  programs.bash = {
    enable = true;
    shellOptions = [
      "histappend"
      "checkwinsize"
      "extglob"
      "globstar"
      "checkjobs"
    ];
    historyControl = [
      "ignoredups"
      "erasedups"
    ];
  };
}
