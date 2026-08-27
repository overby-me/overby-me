{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    #mistral-vibe
    goose-cli
    opencode
    claude-code
    rtk

    # LLMs just love to use these tools
    bc
    jq
    python3
  ];
}
