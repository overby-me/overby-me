{
  services.espanso = {
    enable = true;
    configs = {
      default = {
        show_notifications = false;
      };
    };
    matches = {
      base = {
        matches = [
          {
            trigger = ":100";
            replace = "💯";
          }
          {
            trigger = ":nix";
            replace = "❄️";
          }
          {
            trigger = ":rust";
            replace = "🦀";
          }
          {
            trigger = ":mojo";
            replace = "🔥";
          }
          {
            trigger = ":cpp";
            replace = "💣";
          }
          {
            trigger = ":ok";
            replace = "✅";
          }
          {
            trigger = ":todo";
            replace = "🚧";
          }
          {
            trigger = ":no";
            replace = "🚫";
          }
          {
            trigger = ":eu";
            replace = "🇪🇺";
          }
          {
            trigger = ":dk";
            replace = "🇩🇰";
          }
          {
            trigger = ":us";
            replace = "🇺🇸";
          }
          {
            trigger = ":at";
            replace = "🌀";
          }
        ];
      };
    };
  };
}
