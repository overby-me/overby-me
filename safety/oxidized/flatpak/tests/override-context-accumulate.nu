#!/usr/bin/env nu

^$env.FLATPAK --user override --socket x11 org.test.Accum
^$env.FLATPAK --user override --socket wayland org.test.Accum
^$env.FLATPAK --user override --socket pulseaudio org.test.Accum

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Accum)

# The sockets field should contain all three, semicolon separated
let g = (do { ^grep '^sockets=' $override_file } | complete)
let content = (if $g.exit_code == 0 { $g.stdout } else { open --raw $override_file })
$content | ^grep -q "x11"
$content | ^grep -q "wayland"
$content | ^grep -q "pulseaudio"

print "PASS: override-context-accumulate"
