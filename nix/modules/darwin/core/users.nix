_: {
  # nix-darwin needs to know about the primary user for user-scoped settings
  # (homebrew, system.defaults that write to the user domain, etc.).
  system.primaryUser = "overby.me";

  users.users."overby.me" = {
    home = "/Users/overby.me";
  };
}
