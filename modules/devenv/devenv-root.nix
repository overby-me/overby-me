{
  inputs,
  lib,
  ...
}: let
  envJson = lib.readFile inputs.env.outPath;
  env =
    if envJson != ""
    then lib.fromJSON envJson
    else {PWD = "/home/overby.me/Work/overby.me";};
in {
  devenv.root = env.PWD;
}
