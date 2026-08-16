# Run a single test from the upstream bzip2 sample files against rust-bzip2.
#
# Compares rust-bzip2 output against reference sample data shipped with
# the upstream bzip2 source tarball.
#
# Run with: nix build .#checks.x86_64-linux.rust-bzip2-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-bzip2-test-compress-1
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-bzip2-test-${name}" {
  nativeBuildInputs = [pkgs.rust-bzip2-dev pkgs.bzip2 pkgs.coreutils];
  bzip2Src = pkgs.bzip2.src;
} ''
  # Extract the upstream bzip2 source to get sample files
  tar xf $bzip2Src
  BZ2_SRC=$(echo bzip2-*)
  cd "$BZ2_SRC"

  RUST_BZIP2="${pkgs.rust-bzip2-dev}/bin/bzip2"
  REF_BZIP2="${pkgs.bzip2}/bin/bzip2"

  echo "Running bzip2 test: ${name}"

  case "${name}" in
    compress-1)
      $RUST_BZIP2 -1 < sample1.ref > out.rb2 && cmp out.rb2 sample1.bz2
      ;;
    compress-2)
      $RUST_BZIP2 -2 < sample2.ref > out.rb2 && cmp out.rb2 sample2.bz2
      ;;
    compress-3)
      $RUST_BZIP2 -3 < sample3.ref > out.rb2 && cmp out.rb2 sample3.bz2
      ;;
    decompress-1)
      $RUST_BZIP2 -d < sample1.bz2 > out.tst && cmp out.tst sample1.ref
      ;;
    decompress-2)
      $RUST_BZIP2 -d < sample2.bz2 > out.tst && cmp out.tst sample2.ref
      ;;
    decompress-3)
      $RUST_BZIP2 -ds < sample3.bz2 > out.tst && cmp out.tst sample3.ref
      ;;
    roundtrip-[1-9])
      TEST_NAME="${name}"
      LEVEL="''${TEST_NAME##roundtrip-}"
      $RUST_BZIP2 "-$LEVEL" < sample1.ref > out.bz2
      $RUST_BZIP2 -d < out.bz2 > out.ref
      cmp out.ref sample1.ref
      ;;
    roundtrip-text)
      # Generate some text data
      for i in $(seq 1 200); do
        echo "The quick brown fox jumps over the lazy dog. Line number $i."
      done > input.txt
      $RUST_BZIP2 -9 < input.txt > compressed.bz2
      $REF_BZIP2 -d < compressed.bz2 > decompressed.txt
      cmp input.txt decompressed.txt
      ;;
    roundtrip-binary)
      # Generate binary data
      dd if=/dev/urandom of=input.bin bs=1024 count=64 2>/dev/null
      $RUST_BZIP2 -9 < input.bin > compressed.bz2
      $REF_BZIP2 -d < compressed.bz2 > decompressed.bin
      cmp input.bin decompressed.bin
      ;;
    integrity)
      $RUST_BZIP2 -t < sample1.bz2
      $RUST_BZIP2 -t < sample2.bz2
      $RUST_BZIP2 -t < sample3.bz2
      ;;
    stdin-stdout)
      # Pipe data through stdin compression and stdout decompression, verify roundtrip
      cp sample1.ref input.dat
      $RUST_BZIP2 -1 < input.dat | $RUST_BZIP2 -d > output.dat
      cmp input.dat output.dat
      ;;
    symlinks)
      # Test that bunzip2 and bzcat symlinks work correctly
      RUST_BUNZIP2="${pkgs.rust-bzip2-dev}/bin/bunzip2"
      RUST_BZCAT="${pkgs.rust-bzip2-dev}/bin/bzcat"

      # bunzip2 should decompress from stdin to stdout
      $RUST_BUNZIP2 < sample1.bz2 > out1.tst
      cmp out1.tst sample1.ref

      # bzcat should decompress from stdin to stdout
      $RUST_BZCAT < sample2.bz2 > out2.tst
      cmp out2.tst sample2.ref

      # bunzip2 on a file should remove the .bz2 and produce the original
      cp sample1.bz2 test_bunzip.bz2
      $RUST_BUNZIP2 test_bunzip.bz2
      cmp test_bunzip sample1.ref
      test ! -f test_bunzip.bz2  # original should be removed

      # bzcat on a file should write to stdout and keep the original
      cp sample2.bz2 test_bzcat.bz2
      $RUST_BZCAT test_bzcat.bz2 > out_bzcat.tst
      cmp out_bzcat.tst sample2.ref
      test -f test_bzcat.bz2  # original should still exist
      ;;
    keep)
      # Test -k flag preserves input files
      # Compress with -k: input file should remain
      cp sample1.ref test_keep.ref
      $RUST_BZIP2 -1 -k test_keep.ref
      test -f test_keep.ref      # input preserved
      test -f test_keep.ref.bz2  # output created
      cmp test_keep.ref sample1.ref

      # Decompress with -k: input file should remain
      cp sample1.bz2 test_keep.bz2
      $RUST_BZIP2 -dk test_keep.bz2
      test -f test_keep.bz2  # input preserved
      test -f test_keep       # output created
      cmp test_keep sample1.ref
      ;;
    force-overwrite)
      # Test -f flag overwrites existing output files
      # Create existing output file
      cp sample1.ref test_force.ref
      echo "stale" > test_force.ref.bz2

      # Without -f, should fail (output exists)
      if $RUST_BZIP2 -1 test_force.ref 2>/dev/null; then
        echo "FAIL: should have refused to overwrite without -f"
        exit 1
      fi

      # With -f, should succeed and overwrite
      cp sample1.ref test_force.ref
      $RUST_BZIP2 -1f test_force.ref
      $RUST_BZIP2 -d < test_force.ref.bz2 > test_force_out.ref
      cmp test_force_out.ref sample1.ref
      ;;
    empty)
      # Compress and decompress empty input
      touch empty_input
      $RUST_BZIP2 -1 < empty_input > empty.bz2
      $RUST_BZIP2 -d < empty.bz2 > empty_output
      cmp empty_input empty_output
      ;;
    large)
      # Handle files larger than one bzip2 block (900kB default at -9)
      dd if=/dev/urandom of=large_input bs=1024 count=2048 2>/dev/null
      $RUST_BZIP2 -9 < large_input > large.bz2
      $RUST_BZIP2 -d < large.bz2 > large_output
      cmp large_input large_output
      ;;
    bad-input)
      # Graceful error on corrupt/non-bz2 input
      echo "this is not bzip2 data" > bad.bz2
      if $RUST_BZIP2 -d < bad.bz2 > /dev/null 2>&1; then
        echo "FAIL: should have failed on bad input"
        exit 1
      fi

      # Test with truncated bz2 file
      dd if=sample1.bz2 of=truncated.bz2 bs=10 count=1 2>/dev/null
      if $RUST_BZIP2 -d < truncated.bz2 > /dev/null 2>&1; then
        echo "FAIL: should have failed on truncated input"
        exit 1
      fi

      # Integrity test should also fail on bad data
      if $RUST_BZIP2 -t < bad.bz2 2>/dev/null; then
        echo "FAIL: integrity test should have failed on bad input"
        exit 1
      fi
      ;;
    *)
      echo "Unknown test: ${name}"
      exit 1
      ;;
  esac

  echo "PASS: ${name}"
  touch $out
''
