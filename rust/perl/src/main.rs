fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-v" => {
                println!("perl (rust-perl) {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {
                eprintln!("rust-perl: not yet implemented");
                std::process::exit(1);
            }
        }
    }

    eprintln!("usage: perl [switches] [programfile] [arguments]");
    std::process::exit(1);
}
