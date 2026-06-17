{
  python3,
  runCommand,
  deno,
}:
python3.pkgs.toPythonModule (runCommand "deno-jupyter-kernel"
  {
    buildInputs = [deno];
  } ''
    export DENO_TEST_JUPYTER_PATH=$out/share/jupyter
    ${deno}/bin/deno jupyter --install
  '')
