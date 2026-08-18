//! Flies the drone in the default room and prints what happened, so a change
//! to the navigator can be eyeballed without a browser.
//!
//! ```text
//! cargo run --example trace -- [seconds]
//! ```

use randie_sim::{DEFAULT_MAP, Simulator};

fn main() {
    let seconds: u32 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(120);

    let Ok(mut sim) = Simulator::with_map(DEFAULT_MAP) else {
        eprintln!("the default room has a character in it that is not a block");
        std::process::exit(1);
    };

    let ticks = seconds * 1000 / sim.delta_time;

    println!("second  where it is        where it thinks it is   heading  task");

    for tick in 0..ticks {
        sim.step();

        // Once a second is enough to follow along.
        if tick % (1000 / sim.delta_time) != 0 {
            continue;
        }

        let nav = &sim.drone.firmware.nav;
        println!(
            "{:6}  ({:7.1},{:7.1})  ({:5},{:5})          {:6.1}°  {:?}",
            sim.time / 1000,
            sim.drone.pos.x,
            sim.drone.pos.y,
            nav.posx,
            nav.posy,
            sim.drone.angle.to_degrees(),
            nav.task,
        );
    }

    println!("\nThe map the drone built:\n");
    print!("{}", sim.drone.firmware.map.render());
}
