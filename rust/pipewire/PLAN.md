# rust-pipewire: Plan to Pass Upstream PipeWire Tests

## Goal

A from-scratch Rust reimplementation of [PipeWire](https://pipewire.org/) — the
multimedia (audio + video) graph daemon and its surrounding tools — verified
against the upstream PipeWire test suite (the `test/` and `spa/tests/`
directories of the pipewire source tree).

The model is the same as `rust-binutils`, `rust-perl`, and `rust-systemd`: a
single multicall binary (`rust-pipewire`) that dispatches on `argv[0]` and
exposes the full set of PipeWire tools/daemons via symlinks. Tests are run as
Nix checks comparing rust-pipewire output against the reference C
`pipewire`/`spa` toolchain.

## Current Status

**0/0 tests passing** — project is being scaffolded.

The reference upstream is `pkgs.pipewire` (currently 1.6.3). The Nix
derivation extracts `pkgs.pipewire.src` and runs tests against rust-pipewire
binaries.

---

## Why this is hard

PipeWire is much larger than the typical CLI tool in this repo:

| Component | Lines of C | Role |
|-----------|-----------|------|
| `spa/` | ~80k | Simple Plugin API: types, POD, plugins (audio/video/bluez/v4l2/alsa) |
| `src/pipewire/` | ~30k | Core library: protocol, mainloop, context, proxy, stream |
| `src/modules/` | ~70k | Loadable modules: protocol-native, client-node, metadata, etc. |
| `src/daemon/` | ~1k | The `pipewire` daemon executable |
| `src/tools/` | ~15k | `pw-cli`, `pw-cat`, `pw-mon`, `pw-link`, `pw-dump`, ... |
| `pipewire-pulse/` | ~20k | PulseAudio compat server |
| `pipewire-jack/` | ~15k | JACK API shim |
| `pipewire-alsa/` | ~5k | ALSA plugin |

Total ~250k LOC of C across the whole tree. A literal one-to-one rewrite is a
multi-year effort. This plan focuses on the parts that the upstream test
suite actually exercises, in dependency order.

## Test inventory

Upstream tests live in two places:

### `test/` (PipeWire core + pwtest framework)

Each `test-*.c` file is a `pwtest`-based binary that exercises one part of
the public C API. Most tests link against `libpipewire-0.3.so` / `libspa-0.2.so`
and exercise data structures and IPC behavior.

| Test binary | Source files | What it tests |
|-------------|--------------|---------------|
| `test-pwtest` | `test-pwtest.c` | The pwtest harness itself (sanity check) |
| `test-pw-utils` | `test-properties.c`, `test-array.c`, `test-map.c`, `test-mempool.c`, `test-utils.c` | `pw_properties`, `pw_array`, `pw_map`, `pw_mempool`, helper macros |
| `test-lib` | `test-lib.c` | Library init/deinit, version constants |
| `test-client` | `test-client.c` | `pw_client` proxy on a running core |
| `test-loop` | `test-loop.c` | Event loop with timers/signals/sources |
| `test-context` | `test-context.c` | `pw_context` lifecycle |
| `test-stream` | `test-stream.c` | High-level `pw_stream` audio capture/playback |
| `test-config` | `test-config.c` | Config parsing |
| `test-logger` | `test-logger.c` | Logging framework |
| `test-spa` | `test-spa-buffer.c`, `test-spa-control.c`, `test-spa-format.c`, `test-spa-json.c`, `test-spa-utils.c`, `test-spa-log.c`, `test-spa-node.c`, `test-spa-pod.c` | All SPA building blocks |
| `test-functional` | `test-functional.c` | End-to-end with `pactl`/`openal-info` (skipped without those tools) |

### `spa/tests/` (SPA microbenchmarks)

These are `benchmark-*.c` programs and a single template — they don't have
a fixed pass/fail criterion, so they're informational only.

### How we run upstream tests

Two complementary harnesses:

1. **`spa-json-dump.exp` style (custom comparison)** — a `tests/<tool>/<name>.sh`
   shell script feeds an input file to both the reference binary and our
   rust-pipewire binary, then `diff`s the output. This is the
   `rust-binutils` model. Best for tools whose entire output is text
   (`spa-json-dump`, `pw-dump`, `pw-cli list-objects`, ...).

2. **`pwtest` C-binary style (ABI-compatible cdylib)** — to run the
   upstream `test-pw-utils` etc. without modification, we need to produce
   a `libpipewire-0.3.so` and `libspa-0.2.so` in Rust that satisfy the
   exact same C ABI. The test binaries are then linked against our
   `.so` files via `LD_LIBRARY_PATH`. This is high-effort but high-fidelity
   and matches `rust-binutils-dejagnu`. **Deferred to Phase 6+**; until
   then we mirror each upstream test as a native Rust test in our own
   crate.

3. **Daemon interop** — start `rust-pipewire` daemon, connect with the C
   `pw-cli` binary, verify protocol-level interactions. This is the most
   realistic end-to-end check and is added once a working
   protocol-native server exists.

---

## Architecture

### Tools to provide (by `argv[0]` dispatch)

```text
Daemons
  pipewire              — main media graph daemon
  pipewire-pulse        — PulseAudio-compat server
  pipewire-aes67        — AES67 audio-over-IP daemon (stub)
  pipewire-avb          — AVB daemon (stub)
  pipewire-vulkan       — Vulkan compute daemon (stub)

Client tools
  pw-cli                — interactive REPL + scripted commands
  pw-mon                — live registry/global watcher
  pw-dump               — JSON dump of registry
  pw-link               — link two ports
  pw-metadata           — read/write metadata
  pw-loopback           — create a loopback node
  pw-config             — print effective config
  pw-cat / pw-play / pw-record  — audio capture/playback (WAV first)
  pw-dot                — dump graph as graphviz
  pw-top                — performance/cycle viewer
  pw-profiler           — log raw profiler events
  pw-reserve            — D-Bus device reservation
  pw-container          — namespace/container helper
  pw-mididump           — dump MIDI events
  pw-midiplay / pw-midirecord — MIDI playback/capture
  pw-midi2play / pw-midi2record / pw-sysex — MIDI 2.0 helpers
  pw-dsdplay / pw-encplay — DSD / encoded audio playback
  pw-v4l2               — V4L2 LD_PRELOAD helper

SPA tools
  spa-json-dump         — JSON pretty-printer (the easiest tool)
  spa-inspect           — inspect a SPA plugin
  spa-monitor           — run a SPA monitor and dump events
  spa-acp-tool          — ALSA card profile tool
  spa-resample          — SPA resampler test driver
```

### Crate / module layout

```text
rust/pipewire/
  Cargo.toml
  Cargo.lock
  default.nix              # rust-pipewire / rust-pipewire-dev packages + checks
  testsuite.nix            # custom comparison harness
  upstream-testsuite.nix   # (later) ABI-cdylib harness running pwtest binaries
  PLAN.md                  # this file
  README.md
  src/
    main.rs                # multicall dispatch on argv[0]/argv[1]
    spa/
      mod.rs
      utils/
        dict.rs            # spa_dict
        list.rs            # spa_list (intrusive doubly-linked)
        hook.rs            # spa_hook_list
        json.rs            # SPA-JSON parser/builder
        json_pod.rs        # spa-json ↔ POD bridge
        ringbuffer.rs
        result.rs
        ratelimit.rs
        string.rs          # safe atoi/atof + spa_streq
      pod/
        mod.rs             # spa_pod core types
        builder.rs         # spa_pod_builder
        parser.rs          # spa_pod_parser
        iter.rs            # spa_pod_iter
        compare.rs
        debug.rs           # spa_debug_pod_*
      buffer/              # spa_buffer, spa_data, spa_meta
      control/             # spa_control
      param/
        format.rs          # MediaType/MediaSubType + audio/raw/video/raw param IDs
        audio_raw.rs
        video_raw.rs
      node/                # spa_node + io types
      types.rs             # SPA_TYPE_* enum table
      log.rs               # spa_log + journal/stderr backends
    pipewire/
      mod.rs
      properties.rs        # pw_properties (ordered key-value)
      array.rs             # pw_array (typed dynamic array)
      map.rs               # pw_map (id ↔ ptr table)
      mempool.rs           # pw_mempool (reusable shared-memory blocks)
      loop.rs              # pw_loop = epoll + spa_source
      protocol.rs          # native protocol framing (length, msg type, fds)
      protocol_native.rs   # all opcodes + serialization (Core, Registry, Client, Node, ...)
      context.rs
      core.rs
      registry.rs
      proxy.rs
      client.rs
      stream.rs
      filter.rs
      conf.rs              # config file parsing (spa-json with includes)
    server/
      mod.rs
      daemon.rs            # pipewire main()
      module.rs            # PIPEWIRE_MODULE_PATH loader + builtin modules
      modules/
        protocol_native.rs # listener + per-client decoder
        client_node.rs
        metadata.rs
        default_nodes.rs
        adapter.rs
    pulse/                 # pipewire-pulse server
      mod.rs
    tools/
      pw_cli.rs
      pw_mon.rs
      pw_dump.rs
      pw_link.rs
      pw_metadata.rs
      pw_loopback.rs
      pw_config.rs
      pw_cat.rs
      pw_dot.rs
      pw_top.rs
      pw_profiler.rs
      pw_mididump.rs
      pw_midiplay.rs
      pw_midirecord.rs
      pw_reserve.rs
      pw_container.rs
    spa_tools/
      spa_json_dump.rs
      spa_inspect.rs
      spa_monitor.rs
      spa_acp_tool.rs
      spa_resample.rs
  tests/                   # custom comparison shell scripts
    spa-json-dump/
      basic.sh
      indent.sh
      simplified.sh
    pw-cli/
      version.sh
      help.sh
    pw-config/
      list-paths.sh
    ...
```

### POD: the core data model

Every IPC message and every parameter in PipeWire is encoded as a
**POD** (Plain Old Data). A POD is a typed, length-prefixed byte
sequence that can carry primitives, strings, arrays, structs, choice
values, and recursively-typed objects. Implementing this correctly is
the single biggest enabler — once POD round-trips bit-exactly, the
native protocol, parameter negotiation, and the JSON↔POD bridge all
fall into place.

The encoding (from `spa/include/spa/pod/pod.h`):

```c
struct spa_pod {
  uint32_t size;   // payload size in bytes (excludes the 8-byte header)
  uint32_t type;   // SPA_TYPE_*
  // payload follows, padded to 8-byte boundary
};
```

Concrete payloads include `Bool`, `Id`, `Int`, `Long`, `Float`, `Double`,
`String`, `Bytes`, `Rectangle`, `Fraction`, `Bitmap`, `Array`, `Choice`,
`Struct`, `Object` (with `Prop` entries), `Sequence` (with `Control`
entries), `Pointer`, `Fd`. ABI sizes are fixed and asserted by
`test-spa-pod.c::pod_abi_sizes` — we'll match them.

### Native protocol

The wire format (from `src/pipewire/protocol-native.c`):

```text
struct pw_proto_native_msg {
  uint32_t id;      // proxy/object id
  uint32_t opcode;  // method index for the destination interface
  uint32_t size;    // message body size (sizeof POD struct)
  uint32_t n_fds;   // number of file descriptors that accompany the message
  // followed by `size` bytes containing one `Struct` POD with the args
};
```

Messages are framed with a length-prefixed header and travel over a Unix
seqpacket socket. File descriptors (e.g. `memfd`s for shared buffers) are
sent as ancillary data via `SCM_RIGHTS`.

The first method on every connection is `Core.Hello` (opcode 1) which
sends our supported version. The server answers with `Core.Info` (opcode
0) on the core proxy. From there:

- `Core.GetRegistry` returns a registry proxy
- `Registry.Bind` binds a global by id and returns a proxy
- `Client.UpdateProperties`, `Node.Subscribe`, etc.

We implement enough of the client side to drive `pw-cli list-objects` and
`pw-dump` against a real daemon. This proves our POD + protocol work
without requiring a working server first.

---

## Phased plan

Each phase ends with a measurable test increment and at least one commit.

### Phase 0 — Scaffolding *(this commit)*

Goal: project compiles, multicall binary works, first comparison test
passes.

- [ ] `Cargo.toml` (edition 2024, multicall binary)
- [ ] `src/main.rs` dispatches on `argv[0]` / `argv[1]`
- [ ] `default.nix`: `rust-pipewire` (release) + `rust-pipewire-dev` (debug)
      with `postInstall` symlinks for every tool name
- [ ] `testsuite.nix`: shared comparison harness (extract `pkgs.pipewire.src`,
      define `$REF` and `$RUST` for paired binaries, normalize store paths)
- [ ] First passing test: `pw-cli --version` and `spa-json-dump --help`
      produce identical output (after binary-name normalization)
- [ ] Wire `./rust/pipewire` into `flake.nix`

**Test count target**: 2/2 custom passing.

### Phase 1 — SPA utilities

Goal: dict / list / array / hash / json work; `spa-json-dump` produces
identical output to the C version on a corpus of 20+ JSON inputs.

- [ ] `spa::utils::dict` — `SpaDict` (read-only k/v map)
- [ ] `spa::utils::list` — intrusive doubly-linked list helpers
- [ ] `spa::utils::ringbuffer` — power-of-two ring buffer
- [ ] `spa::utils::string` — `spa_streq`, `spa_strstartswith`, `spa_atoi32`
      (with strict trailing-char detection, like `spa_atou32`), `spa_atof`
- [ ] `spa::utils::json` — full SPA-JSON tokenizer + writer:
  - relaxed JSON (unquoted keys, `=` instead of `:`, comments)
  - strict JSON
  - SPA-JSON arrays, sections without enclosing braces
- [ ] `pipewire::array` — `PwArray`
- [ ] `pipewire::map` — `PwMap`
- [ ] `pipewire::properties` — `PwProperties` ordered k/v with parse/format
- [ ] Native unit tests mirroring `test-spa-utils.c`, `test-spa-json.c`,
      `test-properties.c`, `test-array.c`, `test-map.c`
- [ ] `spa-json-dump` tool: parses input → re-serializes JSON

**Test count target**: ~25 custom tests.

### Phase 2 — POD encoding/decoding

Goal: every POD type round-trips bit-exactly; `test-spa-pod.c` (rewritten
in Rust) and a `pod-roundtrip` corpus pass.

- [ ] All POD primitive readers/writers with correct alignment
- [ ] `PodBuilder` (push primitives, open/close array/struct/object/choice/sequence)
- [ ] `PodParser` (peek, get, optional fields)
- [ ] `PodIter` (iterate items in struct/object/sequence)
- [ ] `pod_compare` and `pod_simplify`
- [ ] `pod_filter` (intersect choice values — used in format negotiation)
- [ ] `spa_debug_pod_*` (printable pretty-printer)
- [ ] `spa-json-pod` bridge (used by `pw-cli set-param`)
- [ ] Test corpus: feed binary blobs taken from a recorded C session into
      both `pw-dump` (C) and our reader, compare JSON output

**Test count target**: ~50 custom tests.

### Phase 3 — Type system

Goal: `SPA_TYPE_*` IDs map to names and back; param IDs and key IDs match
upstream.

- [ ] `spa::types` — exhaustive `SpaType` enum with id/name table
- [ ] `spa::param::format` — MediaType / MediaSubType / Format param IDs
- [ ] `spa::param::audio_raw` — AudioFormat enum (S16_LE, F32_LE, ...)
- [ ] `spa::param::video_raw` — VideoFormat enum
- [ ] `pw_keys` — every documented `PW_KEY_*` constant string
- [ ] Verify against `pkgs.pipewire`'s `spa-inspect support/null-audio-sink`
      output

### Phase 4 — Mainloop & sources

Goal: a `PwLoop` driven by `epoll_wait` can dispatch IO/timer/signal/idle
sources matching the SPA loop interface.

- [ ] `pw_loop` with backing `epoll`
- [ ] `add_io`, `add_timer`, `add_signal`, `add_idle` source kinds
- [ ] `add_event` (cross-thread eventfd) for `pw_thread_loop`
- [ ] `pw_thread_loop` (loop on its own thread + lock + cond)
- [ ] Native unit tests mirroring `test-loop.c`

### Phase 5 — Native protocol client

Goal: connect to a running C `pipewire` daemon, send `Core.Hello`,
receive `Core.Info`, walk the registry, bind a node, dump it. Same
output as upstream `pw-dump`.

- [ ] Unix `SOCK_SEQPACKET` connection at `$XDG_RUNTIME_DIR/pipewire-0`
- [ ] `pw_protocol_native` framing (header + body + fds via `SCM_RIGHTS`)
- [ ] Auto-generated opcode dispatch tables for every interface in
      `protocol-native.c` (Core, Registry, Client, ClientNode, Device,
      Factory, Link, Metadata, Module, Node, Port, Profiler, Session)
- [ ] `pw_proxy` lifecycle (id allocation, listener events)
- [ ] `pw_context` + `pw_core` connect/disconnect
- [ ] `pw_registry` + global add/remove/bind
- [ ] First daemon-interop check: `pw-dump` against a `pipewire` daemon
      run inside the Nix sandbox produces identical JSON

**This is the hard milestone.** Until it passes there's no real
multimedia work happening; once it passes, every CLI tool can be built
on top.

### Phase 6 — Tools that don't need a daemon

These work by reading config files / parsing user input only.

- [ ] `pw-config` — parse and dump effective config (uses `spa-json`)
- [ ] `pw-dot` — read a recorded `pw-dump` JSON and emit graphviz
- [ ] `pw-mididump` — parse a SMF or MIDI 2.0 capture and pretty-print

### Phase 7 — Tools that need a daemon

- [ ] `pw-cli` — full REPL with `list-objects`, `info`, `enum-params`,
      `set-param`, `connect`, `monitor`
- [ ] `pw-mon` — registry watcher
- [ ] `pw-dump` — full registry → JSON
- [ ] `pw-link` — link/unlink ports
- [ ] `pw-metadata` — get/set metadata k/v on the metadata global
- [ ] `pw-loopback` — create a loopback node pair
- [ ] `pw-top` — read the profiler global
- [ ] `pw-profiler` — capture profiler stream

Comparison test pattern: run our tool and the C tool against the **same**
running daemon (the upstream C `pipewire`), check both produce
equivalent output. This verifies our client side without needing our own
daemon.

### Phase 8 — Stream + filter API

- [ ] `pw_stream` (audio capture/playback) — needed by `pw-cat`
- [ ] `pw_filter` (per-port processing) — needed by `pw-loopback`
- [ ] `pw-cat` / `pw-play` / `pw-record` for `.wav` files (libsndfile-equivalent or `hound`)

### Phase 9 — Server side

Goal: `rust-pipewire` daemon hosts our own clients; both C and Rust
clients can connect, see the same registry, link nodes.

- [ ] `pw_impl_core` / `pw_impl_client` / `pw_impl_node` / `pw_impl_link`
- [ ] `module-protocol-native` (listening server)
- [ ] `module-client-node` (so foreign nodes can be exported)
- [ ] `module-metadata`
- [ ] `module-default-nodes`
- [ ] `module-rt` (RTKit / `SCHED_FIFO`)
- [ ] `module-portal`
- [ ] Daemon-only tests: start `rust-pipewire`, run upstream `pw-cli ls`
      against it, expect a non-empty registry

### Phase 10 — Audio backends (SPA plugins)

- [ ] `support/null-audio-sink` (always works; needed for tests)
- [ ] `audioconvert` (resample + format conversion + mix)
- [ ] `audiomixer`
- [ ] `alsa` source/sink (gated; opt-in feature)
- [ ] `bluez5` (deferred indefinitely — out of scope for the first 1.0)

### Phase 11 — Compatibility layers

- [ ] `pipewire-pulse` daemon (PulseAudio TCP/Unix native protocol)
      — runs `pactl info` against it
- [ ] `pipewire-jack` (drop-in `libjack.so`) — deferred; requires C ABI

### Phase 12 — Upstream pwtest harness

Goal: link the unmodified upstream `test-pw-utils` / `test-spa` / etc.
binaries against our `librust-pipewire-0.3.so` cdylib and run them.

- [ ] `cdylib` crate-type variants exporting the C ABI surface that
      pwtest uses (a small subset of `pw_*` and `spa_*` symbols)
- [ ] `upstream-testsuite.nix` extracts `pipewire.src`, builds the test
      binaries with our `.so` substituted via `LD_PRELOAD`, parses the
      pwtest TAP-ish output, applies threshold gates per binary

This is the analog of `rust-binutils-dejagnu`. Until it works, we treat
each upstream test as a manually-mirrored Rust unit test.

---

## Out of scope (initial 1.0)

- GStreamer plugin (`gst/`)
- Bluetooth backend (`spa/plugins/bluez5/`)
- AVB / AES67 daemons (stubbed to just print "unimplemented")
- Vulkan compute (`pipewire-vulkan`)
- V4L2 LD_PRELOAD shim (`pipewire-v4l2`)
- DSD / encoded audio (`pw-dsdplay`, `pw-encplay`)

These ship as no-op stubs so the multicall binary still satisfies every
symlinked name; they will print a clear "not implemented in
rust-pipewire" message and exit 0 (so package install scripts that probe
`--version` don't break).

---

## Workflow

1. Pick the next failing test from `nix flake check`.
2. Reproduce with
   `nix build .#checks.x86_64-linux.rust-pipewire-test-<tool>-<name>`.
3. Read `nix log <drv>` to see the diff. The test script lives at
   `tests/<tool>/<name>.sh`.
4. Implement the missing feature, rebuild.
5. Once green, commit with message
   `feat(rust/pipewire): <short description> + <N> tests`
   (matching the existing rust-binutils style).
6. Push the `pipewire-rust` bookmark.

## Milestones

| Milestone | Tests passing | What works |
|-----------|---------------|------------|
| M0 | 2/2 | Scaffolding, multicall, `--version`/`--help` parity |
| M1 | ~25 | SPA utilities + `spa-json-dump` byte-identical |
| M2 | ~75 | POD round-trip + types |
| M3 | ~85 | Mainloop + first protocol round-trip |
| M4 | ~120 | `pw-cli` / `pw-dump` / `pw-mon` against C daemon |
| M5 | ~180 | Stream API + `pw-cat` for WAV |
| M6 | ~220 | Daemon hosts C clients |
| M7 | informational `+ pwtest` | Upstream pwtest binaries running via cdylib |

---

## References (read these once before touching the code)

- `pkgs.pipewire.src/spa/include/spa/pod/pod.h` — POD encoding
- `pkgs.pipewire.src/src/pipewire/protocol-native.c` — wire format
- `pkgs.pipewire.src/src/pipewire/core.c` — `Core.Hello`/`Core.Info`/`Core.GetRegistry`
- `pkgs.pipewire.src/src/modules/module-protocol-native.c` — server side
- `pkgs.pipewire.src/test/pwtest.h` + `test/test-spa-pod.c` — what we have to match
- The PipeWire wiki page on the [native protocol](https://docs.pipewire.org/page_native_protocol.html)
