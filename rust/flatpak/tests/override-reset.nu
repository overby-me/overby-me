#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

# Create an override first
^$env.FLATPAK --user override --socket wayland org.test.Hello

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.Hello)

if not ($override_file | path exists) {
  print "FAIL: override file was not created"
  exit 1
}

print "Override file exists after creation"

# Now reset (app_id must come before --reset due to arg parsing order)
^$env.FLATPAK --user override org.test.Hello --reset

if ($override_file | path exists) {
  print "FAIL: override file still exists after --reset"
  exit 1
}

print "PASS: override file removed after --reset"
