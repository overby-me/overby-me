#!/usr/bin/env nu
source ./libtest-nix.nu

# Build app and create a normal bundle.
let build_dir = (make_test_app org.test.Mismatch stable)
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join mm-repo) $build_dir -b stable
^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join mm-repo) ($env.TEST_DATA_DIR | path join mm-good.flatpak) $"app/org.test.Mismatch/($env.ARCH)/stable"
ok "good bundle created"

# Tamper with the bundle: parse the format, swap the tar payload's metadata
# file with a different metadata content so it disagrees with the header's
# metadata block.
let py = r#'
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
'#
$py | ^python3 - ($env.TEST_DATA_DIR | path join mm-good.flatpak) ($env.TEST_DATA_DIR | path join mm-bad.flatpak)
ok "tampered bundle created (header metadata != payload metadata)"

# Install runtime locally so post-install run would otherwise work.
let rt_build_dir = (make_test_runtime org.test.Platform stable)
let rt_dest = $env.FL_DIR | path join runtime org.test.Platform $env.ARCH stable active
mkdir $rt_dest
cp ($rt_build_dir | path join metadata) ($rt_dest | path join metadata)
cp -r ($rt_build_dir | path join files) ($rt_dest | path join files)

let r = (do { ^$env.FLATPAK --user build-import-bundle ($env.TEST_DATA_DIR | path join mm-bad.flatpak) } | complete)
let output = $r.stdout + $r.stderr
let rc = $r.exit_code

print $"import output: ($output)"
print $"import rc: ($rc)"

if $rc == 0 {
    print "FAIL: import should have rejected mismatched metadata"
    exit 1
}

if ($output | lines | any {|l| $l =~ '(?i)metadata mismatch' }) {
    ok "import rejected with metadata mismatch error"
} else {
    print "FAIL: error did not mention 'metadata mismatch'"
    print $"Got: ($output)"
    exit 1
}

# Verify nothing was deployed.
let fr = (do { ^find $env.FL_DIR -path '*org.test.Mismatch*' -name files -type d } | complete)
if ($fr.stdout | str trim | is-not-empty) {
    print "FAIL: org.test.Mismatch should not have files/ deployed"
    exit 1
}
ok "no files/ deployed for the mismatched bundle"

print "PASS: vm-metadata-mismatch"
