{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    #mistral-vibe
    opencode
    claude-code
    rtk

    # LLMs just love to use these tools
    bc
    jq
    python3
  ];
}
