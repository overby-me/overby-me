#!/usr/bin/env nu
source ./libtest-nix.nu

# Set up HTTP repo with v1 of the app
setup_http_repo

# Add the HTTP repo as a remote
^$env.FLATPAK --user remote-add --no-gpg-verify test-remote $env.REPO_URL

# Remove locally installed app (setup_http_repo only installs runtime locally)
try { ^$env.FLATPAK --user uninstall org.test.Hello }

# Install app from remote
let ir = (do { ^$env.FLATPAK --user install test-remote org.test.Hello } | complete)
let output = $ir.stdout + $ir.stderr
print $"install output: ($output)"
ok "v1 installed from remote"

# Verify v1 runs correctly
let r1 = (do { run org.test.Hello } | complete)
let run_output = $r1.stdout + $r1.stderr
print $"v1 run output: ($run_output)"

if ($run_output | lines | any {|l| $l =~ 'Hello world, from a sandbox' }) {
  ok "v1 runs correctly"
} else {
  print "FAIL: expected 'Hello world, from a sandbox' in output"
  print $"Got: ($run_output)"
  exit 1
}

# --- Upgrade to v2 ---

# Stop the HTTP server
cleanup_http

# Modify the app build to produce v2 output
let build_dir = $env.TEST_DATA_DIR | path join app-build-org.test.Hello
"#!/bin/sh\necho \"Hello v2, updated\"\n" | save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)

# Re-export the modified app to the same repo
^$env.FLATPAK build-export $env.REPO_DIR $build_dir -b stable
^$env.FLATPAK build-update-repo $env.REPO_DIR
ok "v2 exported to repo"

# Restart HTTP server on the same repo directory
let ostree_dir = $env.REPO_DIR | path join repo
let port_file = $env.TEST_DATA_DIR | path join http-port-v2
rm -f $port_file

let server_py = '
import http.server, sys, os
os.chdir(sys.argv[1])
httpd = http.server.HTTPServer(("127.0.0.1", 0), http.server.SimpleHTTPRequestHandler)
port = httpd.server_address[1]
with open(sys.argv[2], "w") as f:
    f.write(str(port))
httpd.serve_forever()
'
let http_job = (job spawn { ^python3 -c $server_py $ostree_dir $port_file })

mut retries = 0
while ((do { ^test -s $port_file } | complete).exit_code != 0) and $retries < 50 {
  sleep 100ms
  $retries += 1
}

if (do { ^test -s $port_file } | complete).exit_code != 0 {
  print "FAIL: HTTP server (v2) did not start"
  try { job kill $http_job }
  exit 1
}

let new_port = (open --raw $port_file | str trim)
let new_url = $"http://127.0.0.1:($new_port)"
print $"v2 HTTP server at ($new_url)"

# Update the remote to point to the new URL
try { ^$env.FLATPAK --user remote-delete test-remote }
^$env.FLATPAK --user remote-add --no-gpg-verify test-remote $new_url
ok "remote updated to v2 URL"

# Run flatpak update
let ur = (do { ^$env.FLATPAK --user update } | complete)
let update_output = $ur.stdout + $ur.stderr
print $"update output: ($update_output)"
ok "update completed"

# Verify v2 runs correctly
let r2 = (do { run org.test.Hello } | complete)
let run_output2 = $r2.stdout + $r2.stderr
print $"v2 run output: ($run_output2)"

if ($run_output2 | lines | any {|l| $l =~ 'Hello v2, updated' }) {
  ok "v2 runs correctly after update"
} else {
  print "FAIL: expected 'Hello v2, updated' in output"
  print $"Got: ($run_output2)"
  exit 1
}

# Clean up the v2 HTTP server
try { job kill $http_job }

print "PASS: vm-update-from-remote"
