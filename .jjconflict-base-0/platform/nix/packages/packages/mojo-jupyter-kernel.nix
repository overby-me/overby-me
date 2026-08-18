{
  runCommand,
  python3,
  mojo,
}: let
  pythonEnv = python3.withPackages (pp:
    with pp; [
      ipykernel
    ]);
in
  # Fix up the paths in the kernel.json file.
  # The original file contains hardcoded paths to the build environment.
  python3.pkgs.toPythonModule (runCommand "mojo-kernel-hook" {} ''
    mkdir -p $out/share/jupyter/kernels/mojo
    cat > $out/share/jupyter/kernels/mojo/kernel.json <<EOF
    {
      "display_name": "Mojo",
      "argv": [
        "${pythonEnv.interpreter}",
        "${mojo}/share/jupyter/kernels/mojo/mojokernel.py",
        "-f",
        "{connection_file}",
        "--modular-home",
        "${mojo}/etc/modular"
      ],
      "language": "mojo",
      "codemirror_mode": "mojo",
      "language_info": {
        "name": "mojo",
        "mimetype": "text/x-mojo",
        "file_extension": ".mojo",
        "codemirror_mode": {
          "name": "mojo"
        }
      },
      "resources": {
        "logo-64x64": "${mojo}/share/jupyter/kernels/mojo/logo-64x64.png",
        "logo-svg": "${mojo}/share/jupyter/kernels/mojo/logo.svg"
      }
    }
    EOF
    ln -s ${mojo}/share/jupyter/kernels/mojo/logo-64x64.png $out/share/jupyter/kernels/mojo/logo-64x64.png
    ln -s ${mojo}/share/jupyter/kernels/mojo/logo.svg $out/share/jupyter/kernels/mojo/logo.svg
  '')
