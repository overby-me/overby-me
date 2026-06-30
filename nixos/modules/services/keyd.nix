{
  services.keyd = {
    enable = true;
    keyboards = {
      default = {
        ids = ["*"];
        settings = {
          main = {
            # Map arrow keys to nothing (noop)
            left = "noop";
            right = "noop";
            up = "noop";
            down = "noop";

            capslock = "layer(nav)";
          };

          # Define the navigation layer with hjkl
          nav = {
            h = "left";
            j = "down";
            k = "up";
            l = "right";
          };
        };
      };
    };
  };
}
