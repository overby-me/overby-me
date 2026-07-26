//@ flags: -Cpanic=abort
//! Calls: direct, indirect through a function pointer, through a trait
//! object's vtable, to a variadic C function, and to the memcpy and overflow
//! intrinsics. Also the global constants and the string data that come with
//! them.
#![no_std]

unsafe extern "C" {
    fn printf(format: *const u8, ...) -> i32;
    fn plain(x: i32) -> i32;
}

pub trait Shape {
    fn area(&self) -> u64;
    fn name(&self) -> &'static str;
}

pub struct Square(pub u32);

impl Shape for Square {
    fn area(&self) -> u64 {
        u64::from(self.0) * u64::from(self.0)
    }

    fn name(&self) -> &'static str {
        "square"
    }
}

#[unsafe(no_mangle)]
pub fn direct(x: i32) -> i32 {
    unsafe { plain(x) }
}

#[unsafe(no_mangle)]
pub fn indirect(f: fn(i32) -> i32, x: i32) -> i32 {
    f(x)
}

#[unsafe(no_mangle)]
pub fn dynamic(shape: &dyn Shape) -> u64 {
    shape.area()
}

#[unsafe(no_mangle)]
pub fn dynamic_name(shape: &dyn Shape) -> usize {
    shape.name().len()
}

#[unsafe(no_mangle)]
pub fn variadic(x: i32) -> i32 {
    unsafe { printf(c"%d\n".as_ptr() as *const u8, x) }
}

#[unsafe(no_mangle)]
pub fn overflowing(a: u32, b: u32) -> (u32, bool) {
    a.overflowing_add(b)
}

#[unsafe(no_mangle)]
pub fn saturating(a: i16, b: i16) -> i16 {
    a.saturating_sub(b)
}

#[unsafe(no_mangle)]
pub fn counting(x: u64) -> (u32, u32, u32) {
    (x.count_ones(), x.leading_zeros(), x.trailing_zeros())
}

#[unsafe(no_mangle)]
pub fn byte_swap(x: u32) -> u32 {
    x.swap_bytes()
}

#[unsafe(no_mangle)]
pub fn boxed_square(side: u32) -> Square {
    Square(side)
}
