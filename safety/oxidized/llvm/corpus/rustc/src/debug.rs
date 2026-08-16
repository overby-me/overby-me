//@ flags: -Cpanic=abort -Cdebuginfo=2
//! Debug info, which every other seed switches off. This is the only place
//! the corpus holds real specialized metadata nodes: a compile unit and its
//! file, a subprogram per function with its subroutine type, local variables
//! introduced by `#dbg_declare` records, composite types for the struct and
//! the enum, an enumerator list, and the locations hung off instructions.
//!
//! It is deliberately small. What it is here to pin is the grammar of those
//! nodes and the order the printer writes them in, not the completeness of
//! what rustc records.
#![no_std]

pub struct Point {
    pub x: i32,
    pub y: i64,
}

pub enum Shape {
    Empty,
    Dot(Point),
}

pub fn distance(p: &Point) -> i64 {
    let dx = i64::from(p.x);
    let dy = p.y;
    dx * dx + dy * dy
}

pub fn area(shape: &Shape) -> i64 {
    match shape {
        Shape::Empty => 0,
        Shape::Dot(point) => distance(point),
    }
}

pub fn accumulate(values: &[i32]) -> i64 {
    let mut total: i64 = 0;
    let mut index = 0;
    while index < values.len() {
        total += i64::from(values[index]);
        index += 1;
    }
    total
}
