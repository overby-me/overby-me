#!/bin/bash
set -euo pipefail

$FLATPAK --user remote-add alpha-remote https://alpha.example.com/repo
$FLATPAK --user remote-add beta-remote https://beta.example.com/repo
$FLATPAK --user remote-add gamma-remote https://gamma.example.com/repo
$FLATPAK --user remotes > "$WORK/order_out"
grep -q "alpha-remote" "$WORK/order_out"
grep -q "beta-remote" "$WORK/order_out"
grep -q "gamma-remote" "$WORK/order_out"

echo "PASS: remote-add-order-preserved"