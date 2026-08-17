{
  description = "The published projects, as inputs for the differential checks";

  inputs = {
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";

    oxidized-awk = {
      url = "path:../../../../safety/oxidized/awk";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bash = {
      url = "path:../../../../safety/oxidized/bash";
      inputs.workspace.follows = "workspace";
    };
    oxidized-binutils = {
      url = "path:../../../../safety/oxidized/binutils";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bison = {
      url = "path:../../../../safety/oxidized/bison";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bubblewrap = {
      url = "path:../../../../safety/oxidized/bubblewrap";
      inputs.workspace.follows = "workspace";
    };
    oxidized-bzip2 = {
      url = "path:../../../../safety/oxidized/bzip2";
      inputs.workspace.follows = "workspace";
    };
    oxidized-diffutils = {
      url = "path:../../../../safety/oxidized/diffutils";
      inputs.workspace.follows = "workspace";
    };
    oxidized-file = {
      url = "path:../../../../safety/oxidized/file";
      inputs.workspace.follows = "workspace";
    };
    oxidized-gcc = {
      url = "path:../../../../safety/oxidized/gcc";
      inputs.workspace.follows = "workspace";
    };
    oxidized-gzip = {
      url = "path:../../../../safety/oxidized/gzip";
      inputs.workspace.follows = "workspace";
    };
    oxidized-help2man = {
      url = "path:../../../../safety/oxidized/help2man";
      inputs.workspace.follows = "workspace";
    };
    oxidized-llvm = {
      url = "path:../../../../safety/oxidized/llvm";
      inputs.workspace.follows = "workspace";
    };
    oxidized-make = {
      url = "path:../../../../safety/oxidized/make";
      inputs.workspace.follows = "workspace";
    };
    oxidized-ninja = {
      url = "path:../../../../safety/oxidized/ninja";
      inputs.workspace.follows = "workspace";
    };
    oxidized-patch = {
      url = "path:../../../../safety/oxidized/patch";
      inputs.workspace.follows = "workspace";
    };
    oxidized-perl = {
      url = "path:../../../../safety/oxidized/perl";
      inputs.workspace.follows = "workspace";
    };
    oxidized-pipewire = {
      url = "path:../../../../safety/oxidized/pipewire";
      inputs.workspace.follows = "workspace";
    };
    oxidized-patchelf = {
      url = "path:../../../../safety/oxidized/patchelf";
      inputs.workspace.follows = "workspace";
    };
    oxidized-pcre2 = {
      url = "path:../../../../safety/oxidized/pcre2";
      inputs.workspace.follows = "workspace";
    };
    oxidized-sed = {
      url = "path:../../../../safety/oxidized/sed";
      inputs.workspace.follows = "workspace";
    };
    oxidized-texinfo = {
      url = "path:../../../../safety/oxidized/texinfo";
      inputs.workspace.follows = "workspace";
    };
    wclip = {
      url = "path:../../../../dev/wclip";
      inputs.workspace.follows = "workspace";
    };
  };

  outputs = inputs: {
    published = removeAttrs inputs ["self" "workspace"];
  };
}
