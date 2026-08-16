# gzip → oxidized-gzip
#
# GNU gzip provides compression/decompression for the .gz format.
# oxidized-gzip wraps the flate2 crate with GNU gzip flag compatibility.
# Provides gzip, gunzip, and zcat via argv[0] detection.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "gzip";
  original = pkgs.gzip;
  replacement = pkgs.oxidized-gzip;
  status = status.available;
  source = source.repo;
  phase = 3;
  description = "Compression utility for .gz format";
  notes = "Using oxidized-gzip from safety/oxidized/gzip — wraps flate2 crate, provides gzip/gunzip/zcat";
}
