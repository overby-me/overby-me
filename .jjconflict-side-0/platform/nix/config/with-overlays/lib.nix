# Overlay Flake lib as lib.overby-me
# and add overby-me to lib.maintainers
(_: prev: {
  lib = prev.lib.extend (_: prevLib: {
    overby-me = prev.outputs.lib;
    maintainers =
      prevLib.maintainers
      // {
        overby-me = {
          name = "Niclas Overby";
          email = "niclas@overby.me";
          github = "overby-me";
          githubId = "2422942";
        };
      };
  });
})
