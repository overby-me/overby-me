# oxidized-pkg-config, put where a callPackage argument can find it.
#
# Six packages here take it as one. It used to be in scope because they lived
# in the tree that builds it, and the self-overlay put every discovered
# project into `pkgs`; a workspace that ships on its own has to name what it
# uses, so it is an input, and this is what turns the input into a package.
#
# The build is the port's own flake either way: in this tree that input is
# overridden onto `safety/oxidized/pkg-config`, so it is the same source the
# monorepo would have built, and the project-flakes check is what asserts the
# two builds of it agree.
final: prev: {
  oxidized-pkg-config = prev.inputs.oxidized-pkg-config.packages.${final.stdenv.hostPlatform.system}.oxidized-pkg-config;
}
