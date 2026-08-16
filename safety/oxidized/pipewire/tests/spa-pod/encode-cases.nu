#!/usr/bin/env nu

# Verify rust-pipewire's POD encoder produces byte-identical output to
# the C `spa_pod_builder_*` API for a range of value shapes.
#
# Compiles a small C helper using libspa, runs both encoders for each
# named case, and `cmp`s the binary output. The helper is inlined so the
# whole test is self-contained.

let c_src = ("#" + r#'include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include <spa/pod/builder.h>
#include <spa/utils/defs.h>
#include <spa/utils/type.h>

static int build(const char *name, void *buf, size_t bufsize) {
	struct spa_pod_builder b = SPA_POD_BUILDER_INIT(buf, bufsize);
	struct spa_pod_frame f, f2;

	if      (strcmp(name, "none")             == 0) spa_pod_builder_none(&b);
	else if (strcmp(name, "bool-true")        == 0) spa_pod_builder_bool(&b, true);
	else if (strcmp(name, "bool-false")       == 0) spa_pod_builder_bool(&b, false);
	else if (strcmp(name, "id-7")             == 0) spa_pod_builder_id(&b, 7);
	else if (strcmp(name, "int-42")           == 0) spa_pod_builder_int(&b, 42);
	else if (strcmp(name, "int-neg")          == 0) spa_pod_builder_int(&b, -7);
	else if (strcmp(name, "long-pi")          == 0) spa_pod_builder_long(&b, 0x1122334455667788LL);
	else if (strcmp(name, "float-1_5")        == 0) spa_pod_builder_float(&b, 1.5f);
	else if (strcmp(name, "double-pi")        == 0) spa_pod_builder_double(&b, M_PI);
	else if (strcmp(name, "string-hello")     == 0) spa_pod_builder_string(&b, "hello");
	else if (strcmp(name, "string-empty")     == 0) spa_pod_builder_string(&b, "");
	else if (strcmp(name, "rect-fhd")         == 0) spa_pod_builder_rectangle(&b, 1920, 1080);
	else if (strcmp(name, "frac-30000-1001")  == 0) spa_pod_builder_fraction(&b, 30000, 1001);
	else if (strcmp(name, "fd-3")             == 0) spa_pod_builder_fd(&b, 3);
	else if (strcmp(name, "array-int-1234")   == 0) {
		spa_pod_builder_array(&b, sizeof(int32_t), SPA_TYPE_Int, 4,
				(int32_t[]){1,2,3,4});
	}
	else if (strcmp(name, "struct-mixed")     == 0) {
		spa_pod_builder_push_struct(&b, &f);
		spa_pod_builder_int(&b, 1);
		spa_pod_builder_string(&b, "foo");
		spa_pod_builder_bool(&b, true);
		spa_pod_builder_long(&b, 99);
		spa_pod_builder_pop(&b, &f);
	}
	else if (strcmp(name, "object-format")    == 0) {
		spa_pod_builder_push_object(&b, &f, 0x40003u, 0u);
		spa_pod_builder_prop(&b, 1u, 0u);
		spa_pod_builder_id(&b, 0x10001u);
		spa_pod_builder_prop(&b, 2u, 0u);
		spa_pod_builder_string(&b, "audio.raw");
		spa_pod_builder_pop(&b, &f);
	}
	else if (strcmp(name, "choice-enum-rates") == 0) {
		spa_pod_builder_push_choice(&b, &f2, SPA_CHOICE_Enum, 0);
		spa_pod_builder_int(&b, 48000);
		spa_pod_builder_int(&b, 44100);
		spa_pod_builder_int(&b, 96000);
		spa_pod_builder_pop(&b, &f2);
	}
	else { fprintf(stderr, "unknown case '%s'\n", name); return -1; }
	return b.state.offset;
}

int main(int argc, char **argv) {
	if (argc < 2) { fprintf(stderr, "usage: %s <case>\n", argv[0]); return 2; }
	uint8_t buf[8192];
	int len = build(argv[1], buf, sizeof(buf));
	if (len < 0) return 2;
	if (write(STDOUT_FILENO, buf, (size_t)len) != (ssize_t)len) {
		perror("write"); return 1;
	}
	return 0;
}
'#)

$c_src | save -f --raw ($env.TMPDIR | path join c-pod-encode.c)

let cflags = (^pkg-config --cflags libspa-0.2 | str trim | split row --regex '\s+' | where {|f| $f != "" })
let cc = ($env.CC? | default "cc")
^$cc -O2 -Wall -Werror ...$cflags ($env.TMPDIR | path join c-pod-encode.c) -o ($env.TMPDIR | path join c-pod-encode) -lm

# rust-pipewire's pod-encode helper isn't a symlinked tool name; invoke
# it via the multicall wrapper.
let rust_bin = ($env.RUST_PIPEWIRE_DEV | path join bin rust-pipewire)

let cases = [
  none
  bool-true bool-false
  id-7
  int-42 int-neg
  long-pi
  float-1_5
  double-pi
  string-hello string-empty
  rect-fhd
  frac-30000-1001
  fd-3
  array-int-1234
  struct-mixed
  object-format
  choice-enum-rates
]

let c_bin = ($env.TMPDIR | path join c-pod-encode)
let expected = ($env.TMPDIR | path join expected.bin)
let actual = ($env.TMPDIR | path join actual.bin)

mut failed = 0
for case in $cases {
  let cres = (do { ^$c_bin $case } | complete)
  if $cres.exit_code != 0 {
    print $"FAIL\(($case)\): C harness failed"
    $failed += 1
    continue
  }
  $cres.stdout | save -f --raw $expected
  let rres = (do { ^$rust_bin pod-encode $case } | complete)
  if $rres.exit_code != 0 {
    print $"FAIL\(($case)\): rust harness failed"
    $failed += 1
    continue
  }
  $rres.stdout | save -f --raw $actual
  if (do { ^cmp -s $expected $actual } | complete).exit_code == 0 {
    print $"OK: ($case)"
  } else {
    print $"FAIL: ($case)"
    print "  expected:"
    ^xxd $expected | ^sed 's/^/    /'
    print "  actual:"
    ^xxd $actual | ^sed 's/^/    /'
    $failed += 1
  }
}

if $failed > 0 {
  print $"($failed) cases failed"
  exit 1
}
print $"PASS: spa-pod/encode-cases \((($cases | length) - $failed)/($cases | length)\)"
