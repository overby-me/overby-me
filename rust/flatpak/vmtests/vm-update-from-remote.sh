#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Set up HTTP repo with v1 of the app
setup_http_repo

# Add the HTTP repo as a remote
$FLATPAK --user remote-add --no-gpg-verify test-remote "$REPO_URL" 2>&1

# Remove locally installed app (setup_http_repo only installs runtime locally)
$FLATPAK --user uninstall org.test.Hello 2>&1 || true

# Install app from remote
output=$($FLATPAK --user install test-remote org.test.Hello 2>&1)
echo "install output: $output"
ok "v1 installed from remote"

# Verify v1 runs correctly
run_output=$(run org.test.Hello 2>&1)
echo "v1 run output: $run_output"

if echo "$run_output" | grep -q "Hello world, from a sandbox"; then
  ok "v1 runs correctly"
else
  echo "FAIL: expected 'Hello world, from a sandbox' in output"
  echo "Got: $run_output"
  exit 1
fi

# --- Upgrade to v2 ---

# Stop the HTTP server
cleanup_http

# Modify the app build to produce v2 output
build_dir="$TEST_DATA_DIR/app-build-org.test.Hello"
cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello v2, updated"
SCRIPT
chmod +x "$build_dir/files/bin/hello.sh"

# Re-export the modified app to the same repo
$FLATPAK build-export "$REPO_DIR" "$build_dir" -b stable 2>&1
$FLATPAK build-update-repo "$REPO_DIR" 2>&1
ok "v2 exported to repo"

# Restart HTTP server on the same repo directory
ostree_dir="$REPO_DIR/repo"
port_file="$TEST_DATA_DIR/http-port-v2"
rm -f "$port_file"

python3 -c "
import http.server, sys, os
os.chdir(sys.argv[1])
httpd = http.server.HTTPServer(('127.0.0.1', 0), http.server.SimpleHTTPRequestHandler)
port = httpd.server_address[1]
with open(sys.argv[2], 'w') as f:
    f.write(str(port))
httpd.serve_forever()
" "$ostree_dir" "$port_file" &
HTTP_PID=$!

retries=0
while [ ! -s "$port_file" ] && [ "$retries" -lt 50 ]; do
  sleep 0.1
  retries=$((retries + 1))
done

if [ ! -s "$port_file" ]; then
  echo "FAIL: HTTP server (v2) did not start"
  kill "$HTTP_PID" 2>/dev/null || true
  exit 1
fi

NEW_PORT=$(cat "$port_file")
NEW_URL="http://127.0.0.1:${NEW_PORT}"
echo "v2 HTTP server at $NEW_URL"

# Update the remote to point to the new URL
$FLATPAK --user remote-delete test-remote 2>&1 || true
$FLATPAK --user remote-add --no-gpg-verify test-remote "$NEW_URL" 2>&1
ok "remote updated to v2 URL"

# Run flatpak update
update_output=$($FLATPAK --user update 2>&1)
echo "update output: $update_output"
ok "update completed"

# Verify v2 runs correctly
run_output=$(run org.test.Hello 2>&1)
echo "v2 run output: $run_output"

if echo "$run_output" | grep -q "Hello v2, updated"; then
  ok "v2 runs correctly after update"
else
  echo "FAIL: expected 'Hello v2, updated' in output"
  echo "Got: $run_output"
  exit 1
fi

# Clean up the v2 HTTP server
kill "$HTTP_PID" 2>/dev/null || true
wait "$HTTP_PID" 2>/dev/null || true

echo "PASS: vm-update-from-remote"