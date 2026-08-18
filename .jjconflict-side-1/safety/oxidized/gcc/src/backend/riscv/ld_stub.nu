#!/usr/bin/env nu
# Placeholder linker for RISC-V 64 backend.
# Set MY_LD to point to this script to test the custom linker integration.
# TODO: Replace this stub with a real linker implementation.
#
# Runs as a top-level script (no `main`) so it accepts a linker's arbitrary
# dash-prefixed arguments without nushell trying to parse them as its own
# flags. It always fails, signalling the stub is not implemented.
print -e "ERROR: RISC-V 64 custom linker stub called but not yet implemented."
exit 1
