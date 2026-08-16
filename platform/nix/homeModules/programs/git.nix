_: {
  programs.git = {
    enable = true;
    lfs = {
      enable = true;
    };
    settings = {
      user = {
        name = "Niclas Overby";
        email = "niclas@overby.me";
      };
      core = {
        autocrlf = "input";
        editor = "vi";
      };
      init = {
        defaultBranch = "main";
      };
      push = {
        default = "simple";
        autoSetupRemote = true;
      };
      pull = {
        rebase = true;
      };
      merge = {
        tool = "vi";
      };
      mergetool = {
        vi = {
          cmd = "vi $MERGED";
        };
      };
      diff = {
        tool = "vi";
      };
      difftool = {
        vi = {
          cmd = "vi --diff $LOCAL $REMOTE";
        };
      };
      color = {
        ui = "auto";
      };
      credential = {
        helper = "store";
      };
    };
    attributes = [
      "*.java merge=mergiraf"
      "*.kt merge=mergiraf"
      "*.rs merge=mergiraf"
      "*.go merge=mergiraf"
      "*.js merge=mergiraf"
      "*.jsx merge=mergiraf"
      "*.mjs merge=mergiraf"
      "*.json merge=mergiraf"
      "*.yml merge=mergiraf"
      "*.yaml merge=mergiraf"
      "*.toml merge=mergiraf"
      "*.html merge=mergiraf"
      "*.htm merge=mergiraf"
      "*.xhtml merge=mergiraf"
      "*.xml merge=mergiraf"
      "*.c merge=mergiraf"
      "*.h merge=mergiraf"
      "*.cc merge=mergiraf"
      "*.cpp merge=mergiraf"
      "*.hpp merge=mergiraf"
      "*.cs merge=mergiraf"
      "*.dart merge=mergiraf"
      "*.dts merge=mergiraf"
      "*.scala merge=mergiraf"
      "*.sbt merge=mergiraf"
      "*.ts merge=mergiraf"
      "*.tsx merge=mergiraf"
      "*.py merge=mergiraf"
      "*.php merge=mergiraf"
      "*.phtml merge=mergiraf"
      "*.sol merge=mergiraf"
      "*.lua merge=mergiraf"
      "*.rb merge=mergiraf"
    ];
  };
}
