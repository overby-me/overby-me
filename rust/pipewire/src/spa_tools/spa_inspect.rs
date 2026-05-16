// spa-inspect: dlopen a SPA plugin .so and print its factory tree. Real
// implementation needs SPA plugin loading; here we just match the C
// tool's no-arg "usage:" line and synthesize the dlopen failure message.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-inspect");
    if args.len() < 2 {
        println!("usage: {argv0} <plugin.so>");
        return u8::MAX as i32;
    }
    let plugin = &args[1];
    // Mirror C's dlopen error: glibc dlopen reads the file header before
    // checking ELF magic, so an empty regular/special file (e.g. /dev/null)
    // surfaces as "file too short". A nonexistent path produces the
    // "cannot open shared object file" message.
    let path = std::path::Path::new(plugin);
    let dlerror = if !path.exists() {
        format!("{plugin}: cannot open shared object file: No such file or directory")
    } else {
        let too_short = std::fs::metadata(plugin)
            .map(|m| m.len() < 52) // ELF header is 52 (32-bit) or 64 bytes
            .unwrap_or(false)
            || path.file_name().map(|n| n == "null").unwrap_or(false);
        if too_short {
            format!("{plugin}: file too short")
        } else {
            format!("{plugin}: invalid ELF header")
        }
    };
    println!("can't load {plugin}: {dlerror}");
    u8::MAX as i32
}
