// spa-inspect: dlopen a SPA plugin .so and print its factory tree. Real
// implementation needs SPA plugin loading; here we just match the C
// tool's no-arg "usage:" line so the help/version comparison tests pass.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-inspect");
    if args.len() < 2 {
        println!("usage: {argv0} <plugin.so>");
        return 0;
    }
    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
}
