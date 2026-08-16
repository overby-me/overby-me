# rust-pkg-config setup hook
#
# This hook ensures that dependencies' .pc files are discoverable
# by adding their lib/pkgconfig and share/pkgconfig directories
# to PKG_CONFIG_PATH when they appear as build inputs.
#
# This is the essential piece that makes rust-pkg-config a drop-in
# replacement for Nixpkgs' pkg-config wrapper in Nix builds.

# Skip setup hook if we're neither a build-time dep, nor doing a native compile.
[[ -z ${strictDeps-} ]] || (( "$hostOffset" < 0 )) || return 0

pkgConfigRs_addPkgConfigPath () {
    addToSearchPath "PKG_CONFIG_PATH" "$1/lib/pkgconfig"
    addToSearchPath "PKG_CONFIG_PATH" "$1/share/pkgconfig"
}

# Use $targetOffset (not $hostOffset) because pkg-config is a build tool
# (nativeBuildInput, hostOffset=-1) that searches for libraries on the
# *host* platform. Libraries like openssl are in buildInputs at offset 0,
# which corresponds to $targetOffset for a nativeBuildInput.
addEnvHooks "$targetOffset" pkgConfigRs_addPkgConfigPath
