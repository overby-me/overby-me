{
  python3,
  runCommand,
  evcxr,
}:
python3.pkgs.toPythonModule (runCommand "rust-jupyter-kernel"
  {
    buildInputs = [evcxr];
  } ''
    export JUPYTER_PATH=$out/share/jupyter
    ${evcxr}/bin/evcxr_jupyter --install
  '')
