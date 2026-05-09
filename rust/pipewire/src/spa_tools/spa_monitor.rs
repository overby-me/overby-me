// spa-monitor: load a SPA monitor plugin and dump its events. Real
// implementation deferred; here we just mirror the no-arg usage line.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-monitor");
    if args.len() < 2 {
        println!("usage: {argv0} <plugin.so>");
        return 0;
    }
    let plugin = &args[1];
    // Mirror the C tool: `can't load <path>` to stdout (no extra dlerror
    // line, unlike spa-inspect).
    println!("can't load {plugin}");
    0
}
