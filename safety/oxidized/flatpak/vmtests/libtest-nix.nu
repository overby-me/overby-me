# Minimal test harness for rust-flatpak VM tests (nushell).
# Fixtures source this file from the same directory: `source ./libtest-nix.nu`

$env.LC_ALL = "C"
$env.FLATPAK = ($env.FLATPAK? | default "flatpak")
$env.ARCH = (^uname -m | str trim)
$env.TEST_DATA_DIR = ($env.WORK | path join testdata)
$env.FL_DIR = ($env.HOME | path join .local share flatpak)

mkdir $env.TEST_DATA_DIR $env.FL_DIR

$env.TESTS_PASSED = "0"
$env.TESTS_FAILED = "0"

def --env ok [msg: string] {
    print $"ok - ($msg)"
    $env.TESTS_PASSED = (($env.TESTS_PASSED | into int) + 1 | into string)
}

def skip [msg: string] {
    print $"skip - ($msg)"
}

# File assertions use external `test` so symlinks are followed exactly like
# bash's [ -f ] / [ -d ] did.
def test-flag [flag: string, path: string] {
    (do { ^test $flag $path } | complete).exit_code == 0
}

def assert_has_file [f: string] {
    if not (test-flag "-f" $f) { print $"FAIL: expected file: ($f)"; exit 1 }
}

def assert_has_dir [d: string] {
    if not (test-flag "-d" $d) { print $"FAIL: expected dir: ($d)"; exit 1 }
}

def assert_not_has_file [f: string] {
    if (test-flag "-f" $f) { print $"FAIL: unexpected file: ($f)"; exit 1 }
}

def assert_not_has_dir [d: string] {
    if (test-flag "-d" $d) { print $"FAIL: unexpected dir: ($d)"; exit 1 }
}

def assert_file_has_content [file: string, pattern: string] {
    if (do { ^grep -qE $pattern $file } | complete).exit_code != 0 {
        print $"FAIL: '($file)' missing content matching '($pattern)'"
        print (open --raw $file)
        exit 1
    }
}

def assert_not_file_has_content [file: string, pattern: string] {
    if (do { ^grep -qE $pattern $file } | complete).exit_code == 0 {
        print $"FAIL: '($file)' unexpectedly contains '($pattern)'"
        print (open --raw $file)
        exit 1
    }
}

def assert_file_empty [f: string] {
    if (test-flag "-s" $f) {
        print $"FAIL: expected empty: ($f)"
        print (open --raw $f)
        exit 1
    }
}

def assert_streq [a: string, b: string] {
    if $a != $b { print $"FAIL: expected '($a)' == '($b)'"; exit 1 }
}

def assert_not_streq [a: string, b: string] {
    if $a == $b { print $"FAIL: expected '($a)' != '($b)'"; exit 1 }
}

def assert_match [output: string, pattern: string, msg?: string] {
    if not ($output | lines | any {|l| $l =~ $pattern }) {
        print $"FAIL: ($msg | default 'output did not match'): expected pattern '($pattern)' in:"
        print $output
        exit 1
    }
}

def assert_not_match [output: string, pattern: string, msg?: string] {
    if ($output | lines | any {|l| $l =~ $pattern }) {
        print $"FAIL: ($msg | default 'output unexpectedly matched'): pattern '($pattern)' in:"
        print $output
        exit 1
    }
}

def make_test_app [app_id: string = "org.test.Hello", branch: string = "stable"] {
    let build_dir = $env.TEST_DATA_DIR | path join $"app-build-($app_id)"
    rm -rf $build_dir
    ^$env.FLATPAK build-init $build_dir $app_id org.test.Sdk org.test.Platform $branch
    mkdir ($build_dir | path join files bin)
    "#!/bin/sh\necho \"Hello world, from a sandbox\"\n"
    | save -f ($build_dir | path join files bin hello.sh)
    ^chmod +x ($build_dir | path join files bin hello.sh)

    # Desktop file
    mkdir ($build_dir | path join files share applications)
    $"[Desktop Entry]\nName=Hello\nExec=hello.sh\nType=Application\nIcon=($app_id)\n"
    | save -f ($build_dir | path join files share applications $"($app_id).desktop")

    # Icon
    mkdir ($build_dir | path join files share icons hicolor 64x64 apps)
    "PNG\n" | save -f ($build_dir | path join files share icons hicolor 64x64 apps $"($app_id).png")

    (^$env.FLATPAK build-finish $build_dir --command hello.sh
        --share network --share ipc
        --socket x11 --socket wayland --socket pulseaudio
        --device dri --filesystem home)

    $build_dir
}

def make_test_runtime [rt_id: string = "org.test.Platform", branch: string = "stable"] {
    let build_dir = $env.TEST_DATA_DIR | path join $"runtime-build-($rt_id)"
    rm -rf $build_dir
    mkdir ($build_dir | path join files bin) ($build_dir | path join files lib) ($build_dir | path join files etc)

    # Copy real binaries into the runtime so commands work in sandbox
    for cmd in [sh bash cat echo ls env mkdir rm cp mv ln test readlink wc grep sed awk head tail sort tr cut tee id uname hostname date sleep true false pwd dirname basename] {
        let path = which $cmd | get 0?.path | default ""
        if ($path != "") and (test-flag "-f" $path) {
            try { ^cp $path ($build_dir | path join files bin) }
        }
    }

    # Copy required shared libraries
    for lib_dir in ["/lib" "/lib64" "/usr/lib" "/usr/lib64"] {
        if ($lib_dir | path exists) {
            for pat in ["ld-linux*.so*" "libc.so*" "libdl.so*" "libpthread.so*" "libm.so*"] {
                for f in (glob ($lib_dir | path join $pat)) {
                    try { ^cp -a $f ($build_dir | path join files lib) }
                }
            }
        }
    }

    # On NixOS (and other distros), the dynamic linker is often at
    # /lib64/ld-linux-x86-64.so.2. Binaries have this path hardcoded in their
    # ELF header. Inside the bwrap sandbox, /lib64 -> usr/lib64, so we need
    # files/lib64/ to contain the linker too.
    mkdir ($build_dir | path join files lib64)
    for f in (glob ($build_dir | path join files lib "ld-linux*.so*")) {
        try { ^cp -a $f ($build_dir | path join files lib64) }
    }
    # Also try copying from the host /lib64 directly (handles cases where
    # /lib didn't have it)
    for f in (glob "/lib64/ld-linux*.so*") {
        try { ^cp -a $f ($build_dir | path join files lib64) }
    }
    # On NixOS, the real interpreter is in /nix/store; find it from any
    # copied binary
    let interp = (
        try {
            (do {
                ^readelf -l ($build_dir | path join files bin sh)
                | ^grep -oP '/nix/store/\S+/lib/ld-linux[^\]]*'
            } | complete).stdout
        } catch { "" }
    ) | str trim
    if ($interp != "") and (test-flag "-f" $interp) {
        try { ^cp -a $interp ($build_dir | path join files lib64) }
        # Also create the standard name as a symlink
        let interp_name = $interp | path basename
        let std_name = $build_dir | path join files lib64 ld-linux-x86-64.so.2
        if (not ($std_name | path exists)) and (test-flag "-f" ($build_dir | path join files lib64 $interp_name)) {
            try { ^ln -sf $interp_name $std_name }
        }
    }

    $"[Runtime]\nname=($rt_id)\nruntime=($rt_id)/($env.ARCH)/($branch)\nsdk=org.test.Sdk/($env.ARCH)/($branch)\n"
    | save -f ($build_dir | path join metadata)

    $build_dir
}

# Install runtime directly (file copy) so apps installed from remote can run.
def setup_local_runtime [branch: string = "stable"] {
    let rt_src = $env.TEST_DATA_DIR | path join runtime-build-org.test.Platform
    let rt_dest = $env.FL_DIR | path join runtime org.test.Platform $env.ARCH $branch active
    mkdir $rt_dest
    cp ($rt_src | path join metadata) ($rt_dest | path join metadata)
    cp -r ($rt_src | path join files) ($rt_dest | path join files)
}

def setup_repo [branch: string = "stable"] {
    make_test_app org.test.Hello $branch
    make_test_runtime org.test.Platform $branch

    # Install app from build dir
    try { ^$env.FLATPAK --user install ($env.TEST_DATA_DIR | path join app-build-org.test.Hello) }

    # Install runtime manually (direct file copy to installation)
    setup_local_runtime $branch
}

def install_repo [branch: string = "master"] {
    try { ^$env.FLATPAK --user install ($env.TEST_DATA_DIR | path join app-build-org.test.Hello) }
}

def run [...args: string] {
    ^$env.FLATPAK --user run ...$args
}

def run_sh [app_id: string, ...cmd: string] {
    ^$env.FLATPAK --user run --command=sh $app_id -c ($cmd | str join " ")
}

# ---------------------------------------------------------------------------
# HTTP repo helpers (Cat 13: remote + network tests)
# ---------------------------------------------------------------------------

# Build a test app, export to an OSTree repo, update summary, and serve over
# HTTP. Sets $env.REPO_DIR, $env.REPO_PORT, $env.REPO_URL, and $env.HTTP_JOB.
def --env setup_http_repo [branch: string = "stable"] {
    let repo_dir = $env.TEST_DATA_DIR | path join http-repo
    rm -rf $repo_dir

    # Build test app
    let build_dir = make_test_app org.test.Hello $branch

    # Build test runtime
    let rt_build_dir = make_test_runtime org.test.Platform $branch

    # Export app to OSTree repo
    ^$env.FLATPAK build-export $repo_dir $build_dir -b $branch

    # Export runtime to OSTree repo
    ^$env.FLATPAK build-export $repo_dir $rt_build_dir -b $branch

    # Generate GVariant summary
    ^$env.FLATPAK build-update-repo $repo_dir

    # Serve repo/ (the OSTree repo dir) over HTTP
    let ostree_dir = $repo_dir | path join repo
    let port_file = $env.TEST_DATA_DIR | path join http-port
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
    $env.HTTP_JOB = (job spawn { ^python3 -c $server_py $ostree_dir $port_file } | into string)

    # Wait for the port file to appear
    mut retries = 0
    while (not (test-flag "-s" $port_file)) and $retries < 50 {
        sleep 100ms
        $retries += 1
    }

    if not (test-flag "-s" $port_file) {
        print "FAIL: HTTP server did not start"
        cleanup_http
        exit 1
    }

    $env.REPO_DIR = $repo_dir
    $env.REPO_PORT = (open --raw $port_file | str trim)
    $env.REPO_URL = $"http://127.0.0.1:($env.REPO_PORT)"

    # Also install runtime locally so run commands work after remote install
    setup_local_runtime $branch
}

# Clean up HTTP server. There is no EXIT trap in nushell, but background jobs
# die with the nu process, so leaking one on a failure path is harmless here.
# Fixtures that finish normally should still call this to stop the server.
def cleanup_http [] {
    if ($env.HTTP_JOB? | default "") != "" {
        try { job kill ($env.HTTP_JOB | into int) }
    }
}
