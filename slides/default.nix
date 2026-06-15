{
  devShells.slides = pkgs: {
    packages = with pkgs; [
      (python313.withPackages (
        pp:
          with pp; [
            # Python packages
            pip
            notebook
            jupyter-console

            # Kernels
            #deno-jupyter-kernel
            #mojo-jupyter-kernel
            #rust-jupyter-kernel
            #nu-jupyter-kernel
          ]
      ))
      #sidecar
    ];
  };
}
