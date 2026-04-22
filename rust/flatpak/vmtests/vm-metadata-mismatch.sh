#!/bin/bash
set -euo pipefail

. "$(dirname "$0")/libtest-nix.sh"

# Build app and create a normal bundle.
build_dir=$(make_test_app org.test.Mismatch stable)
$FLATPAK build-export "$TEST_DATA_DIR/mm-repo" "$build_dir" -b stable 2>&1
$FLATPAK build-bundle "$TEST_DATA_DIR/mm-repo" "$TEST_DATA_DIR/mm-good.flatpak" "app/org.test.Mismatch/$ARCH/stable" 2>&1
ok "good bundle created"

# Tamper with the bundle: parse the format, swap the tar payload's metadata
# file with a different metadata content so it disagrees with the header's
# metadata block.
python3 - "$TEST_DATA_DIR/mm-good.flatpak" "$TEST_DATA_DIR/mm-bad.flatpak" << 'PY'
import io, struct, sys, tarfile, zlib

src, dst = sys.argv[1], sys.argv[2]
data = open(src, "rb").read()

assert data[:8] == b"flatbndl", "not a flatpak bundle"
ver = struct.unpack("<I", data[8:12])[0]
off = 12

ref_len = struct.unpack("<I", data[off:off+4])[0]; off += 4
ref_name = data[off:off+ref_len]; off += ref_len

meta_len = struct.unpack("<I", data[off:off+4])[0]; off += 4
header_meta = data[off:off+meta_len]; off += meta_len

payload_len = struct.unpack("<I", data[off:off+4])[0]; off += 4
compressed = data[off:off+payload_len]

# Decompress the deflate payload (raw deflate, not zlib-wrapped).
tar_bytes = zlib.decompress(compressed, -15)

# Rewrite the tar payload, replacing or injecting a divergent metadata file.
divergent_meta = b"[Application]\nname=org.evil.Other\nruntime=org.evil.Platform/x86_64/stable\ncommand=evil\n"

src_tar = tarfile.open(fileobj=io.BytesIO(tar_bytes), mode="r:")
out_buf = io.BytesIO()
out_tar = tarfile.open(fileobj=out_buf, mode="w:")
seen_meta = False
for member in src_tar.getmembers():
    if member.name in ("./metadata", "metadata"):
        seen_meta = True
        info = tarfile.TarInfo(name=member.name)
        info.size = len(divergent_meta)
        info.mode = 0o644
        out_tar.addfile(info, io.BytesIO(divergent_meta))
    else:
        f = src_tar.extractfile(member) if member.isfile() else None
        out_tar.addfile(member, f)
if not seen_meta:
    info = tarfile.TarInfo(name="./metadata")
    info.size = len(divergent_meta)
    info.mode = 0o644
    out_tar.addfile(info, io.BytesIO(divergent_meta))
out_tar.close()
src_tar.close()

new_tar = out_buf.getvalue()
# Re-compress (raw deflate to match writer).
co = zlib.compressobj(6, zlib.DEFLATED, -15)
new_compressed = co.compress(new_tar) + co.flush()

with open(dst, "wb") as f:
    f.write(b"flatbndl")
    f.write(struct.pack("<I", ver))
    f.write(struct.pack("<I", len(ref_name))); f.write(ref_name)
    f.write(struct.pack("<I", len(header_meta))); f.write(header_meta)
    f.write(struct.pack("<I", len(new_compressed))); f.write(new_compressed)

print("bad bundle written:", dst)
PY
ok "tampered bundle created (header metadata != payload metadata)"

# Install runtime locally so post-install run would otherwise work.
rt_build_dir=$(make_test_runtime org.test.Platform stable)
rt_dest="$FL_DIR/runtime/org.test.Platform/${ARCH}/stable/active"
mkdir -p "$rt_dest"
cp "$rt_build_dir/metadata" "$rt_dest/metadata"
cp -r "$rt_build_dir/files" "$rt_dest/files"

set +e
output=$($FLATPAK --user build-import-bundle "$TEST_DATA_DIR/mm-bad.flatpak" 2>&1)
rc=$?
set -e

echo "import output: $output"
echo "import rc: $rc"

if [ "$rc" -eq 0 ]; then
  echo "FAIL: import should have rejected mismatched metadata"
  exit 1
fi

if echo "$output" | grep -qi "metadata mismatch"; then
  ok "import rejected with metadata mismatch error"
else
  echo "FAIL: error did not mention 'metadata mismatch'"
  echo "Got: $output"
  exit 1
fi

# Verify nothing was deployed.
if find "$FL_DIR" -path "*org.test.Mismatch*" -name files -type d | grep -q .; then
  echo "FAIL: org.test.Mismatch should not have files/ deployed"
  exit 1
fi
ok "no files/ deployed for the mismatched bundle"

echo "PASS: vm-metadata-mismatch"
