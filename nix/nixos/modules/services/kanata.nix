{
  services.kanata = {
    enable = true;
    keyboards.default = {
      devices = [];
      extraDefCfg = "process-unmapped-keys yes";
      config = ''
        (defsrc
          caps
          left down up   rght
          h    j    k    l
        )

        (defalias
          nav (tap-hold 200 200 caps (layer-while-held nav))
        )

        (deflayer base
          @nav
          XX   XX   XX   XX
          h    j    k    l
        )

        (deflayer nav
          _
          XX   XX   XX   XX
          left down up   rght
        )
      '';
    };
  };
}
