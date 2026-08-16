use goblin::elf::Elf;
use std::process;

pub fn print_interpreter(elf: &Elf) {
    match &elf.interpreter {
        Some(interp) => println!("{interp}"),
        None => {
            eprintln!("patchelf: no PT_INTERP segment found");
            process::exit(1);
        }
    }
}

pub fn print_rpath(elf: &Elf) {
    if !elf.runpaths.is_empty() {
        println!("{}", elf.runpaths.join(":"));
        return;
    }
    if !elf.rpaths.is_empty() {
        println!("{}", elf.rpaths.join(":"));
        return;
    }
    println!();
}

pub fn print_soname(elf: &Elf) {
    match &elf.soname {
        Some(name) => println!("{name}"),
        None => {
            eprintln!("patchelf: no DT_SONAME found");
            process::exit(1);
        }
    }
}

pub fn print_needed(elf: &Elf) {
    for lib in &elf.libraries {
        println!("{lib}");
    }
}
