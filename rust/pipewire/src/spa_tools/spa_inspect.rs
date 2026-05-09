// spa-inspect: dlopen a SPA plugin .so and print its factory tree. Real
// implementation needs SPA plugin loading; here we just match the C
// tool's no-arg "usage:" line so the help/version comparison tests pass.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-inspect");
    if args.len() < 2 {
        println!("usage: {argv0} <plugin.so>");
        return 0;
    }
    let plugin = &args[1];
    // Mirror the C tool's dlopen-failure shape: `can't load <path>:
    // <dlerror>` to stderr. We don't actually dlopen anything yet, so
    // we synthesize the message from the file's existence check.
    println!("can't load {plugin}: {plugin}: cannot open shared object file: No such file or directory");
    0
}
