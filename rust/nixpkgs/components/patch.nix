# GNU patch → rust-patch
#
# GNU patch applies diff/patch files to source trees. It is used
# extensively in mkDerivation's patchPhase to apply nixpkgs patches.
# rust-patch supports unified, context, and normal diff formats with
# fuzz matching, reverse patching, and all common flags.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "patch";
  original = pkgs.gnupatch;
  replacement = pkgs.rust-patch;
  status = status.available;
  source = source.repo;
  phase = 4;
  description = "Apply diff files to source trees";
  notes = "Using rust-patch from rust/patch — unified/context/normal diff with fuzz matching";
}
