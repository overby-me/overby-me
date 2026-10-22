//! Port of `hacks/barcode.c`.
//!
//! ```text
//! barcode, draw some barcodes
//! by Dan Bornstein, danfuzz@milk.com
//! Copyright (c) 2003 Dan Bornstein. All rights reserved.
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! See the included man page for more details.
//! ```
//!
//! Real barcodes, not decorative ones: UPC-A, UPC-E, EAN-8 and EAN-13, with
//! their check digits computed and a word from a list underneath each. CONSUME!
//!
//! Upstream's own lesson on the encoding is worth keeping. Each digit is two
//! bars and two spaces in seven units of width, and each has three
//! representations: a base pattern, its inverse, and its mirror image. Which is
//! used says which half of the symbol the digit is on, and in EAN-13 the
//! *choice* between two of them across the left half is itself how the
//! thirteenth digit is encoded, since there is no room left to draw it.
//!
//! Four things to look at, chosen by the mode knob: barcodes scrolling past at
//! assorted magnifications, a grid of them all changing a digit at a time, and
//! a clock that puts the time in a UPC-E. The clock is not a valid barcode:
//! it has extra dividers after the second and fourth digits so the reading is
//! grouped like a clock, which is upstream's own note.
//!
//! Nothing here uses [`crate::runtime::font`]: the hack carries its own 5x8
//! font, as it carries its own one-bit bitmaps.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{XColor, hsv_to_rgb};
use crate::runtime::{
    About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent, random,
    random_below,
};

const BARCODE_WIDTH: i32 = 164;
const BARCODE_HEIGHT: i32 = 69;
const MAX_MAG: i32 = 7;

/// `RAND_FLOAT_01`: a float in (0..1), taken from the same bits upstream takes.
fn rand_float_01() -> f64 {
    f64::from((random() >> 8) & 0xffff) / 65536.0
}

/// A one-bit bitmap, packed exactly as upstream packs it: a byte holds eight
/// pixels with the leftmost in the low bit.
#[derive(Clone)]
struct Bitmap {
    width: i32,
    height: i32,
    width_bytes: i32,
    buf: Vec<u8>,
}

impl Bitmap {
    fn new(width: i32, height: i32) -> Bitmap {
        let width_bytes = (width + 7) / 8;
        Bitmap {
            width,
            height,
            width_bytes,
            buf: vec![0; (height * width_bytes) as usize],
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0);
    }

    /// Out-of-range reads give zero, which is what makes the copies below safe
    /// to run off the edge of a glyph.
    fn get(&self, x: i32, y: i32) -> u8 {
        let xbyte = x >> 3;
        let xbit = x & 0x7;
        if !(0..self.width_bytes).contains(&xbyte) || !(0..self.height).contains(&y) {
            return 0;
        }
        (self.buf[(self.width_bytes * y + xbyte) as usize] >> xbit) & 1
    }

    /// Out-of-range writes are ignored.
    fn set(&mut self, x: i32, y: i32, value: u8) {
        if !(0..self.width).contains(&x) || !(0..self.height).contains(&y) {
            return;
        }
        let xbyte = x >> 3;
        let xbit = x & 0x7;
        let i = (self.width_bytes * y + xbyte) as usize;
        if value != 0 {
            self.buf[i] |= 1 << xbit;
        } else {
            self.buf[i] &= !(1 << xbit);
        }
    }

    fn vlin(&mut self, x: i32, mut y1: i32, y2: i32) {
        while y1 <= y2 {
            self.set(x, y1, 1);
            y1 += 1;
        }
    }

    /// A rectangle out of the font, which is the only source these copy from.
    fn copy_from_font(&mut self, dx: i32, dy: i32, sx: i32, sy: i32, width: i32, height: i32) {
        for y in 0..height {
            for x in 0..width {
                self.set(x + dx, y + dy, font_get(x + sx, y + sy));
            }
        }
    }

    /// Blow `src` up by `mag` into this one.
    fn scale_from(&mut self, src: &Bitmap, mag: i32) {
        for y in 0..BARCODE_HEIGHT {
            for x in 0..BARCODE_WIDTH {
                let v = src.get(x, y);
                for x2 in 0..mag {
                    for y2 in 0..mag {
                        self.set(x * mag + x2, y * mag + y2, v);
                    }
                }
            }
        }
    }

    /// Draw the given 5x8 character at the given coordinates.
    fn draw_char_5x8(&mut self, x: i32, y: i32, c: char) {
        self.copy_from_font(x, y, 0, (c as i32) * 8, 5, 8);
    }

    /// Draw a string of 5x8 characters at the given coordinates.
    fn draw_string_5x8(&mut self, x: i32, y: i32, s: &str) {
        let origx = x;
        let mut x = x;
        let mut y = y;
        for c in s.chars() {
            if c == '\n' {
                x = origx;
                y += 8;
            } else {
                let c = if (c as u32) < 32 { ' ' } else { c };
                self.draw_char_5x8(x, y, c);
                x += 5;
            }
        }
    }
}

/// One bit of the compiled-in font, which upstream treats as a bitmap eight
/// wide and a thousand and twenty-four tall.
fn font_get(x: i32, y: i32) -> u8 {
    if !(0..8).contains(&x) || !(0..1024).contains(&y) {
        return 0;
    }
    (FONT5X8[y as usize] >> x) & 1
}

/// Which of the three representations of a digit to draw.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UpcSet {
    LeftA,
    LeftB,
    Right,
}

/* the Left A patterns */
const UPC_LEFT_A: [u32; 10] = [0x0d, 0x19, 0x13, 0x3d, 0x23, 0x31, 0x2f, 0x3b, 0x37, 0x0b];
/* the Left B patterns */
const UPC_LEFT_B: [u32; 10] = [0x27, 0x33, 0x1b, 0x21, 0x1d, 0x39, 0x05, 0x11, 0x09, 0x17];
/* the Right patterns */
const UPC_RIGHT: [u32; 10] = [0x72, 0x66, 0x6c, 0x42, 0x5c, 0x4e, 0x50, 0x44, 0x48, 0x74];
/// The EAN-13 first-digit patterns: which of the two left-hand
/// representations each of the six left digits takes is the thirteenth digit.
const EAN13_FIRST_DIGIT: [u32; 10] = [0x00, 0x0b, 0x0d, 0x0e, 0x13, 0x19, 0x1c, 0x15, 0x16, 0x1a];
/// The UPC-E last-digit patterns for first digit 0 (complement for digit 1);
/// also used for 5-digit supplemental check patterns.
const UPC_E_LAST_DIGIT: [u32; 10] = [0x38, 0x34, 0x32, 0x31, 0x2c, 0x26, 0x23, 0x2a, 0x29, 0x25];

/// Turn a character into its digit value; anything else is zero.
fn char_to_digit(c: u8) -> u32 {
    if c.is_ascii_digit() {
        u32::from(c - b'0')
    } else {
        0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Scroll,
    Grid,
    Clock12,
    Clock24,
}

struct Barcode {
    /// Where the left of the barcode is.
    x: i32,
    y: i32,
    /// Magnification factor.
    mag: i32,
    bitmap: Bitmap,
    code: Vec<u8>,
    pixel: Pixel,
}

struct BarcodeHack {
    delay: u32,
    window_width: i32,
    window_height: i32,
    fg_pixel: Pixel,
    bg_pixel: Pixel,
    grid_pixel: Pixel,
    button_down_p: bool,
    grid_alloced_p: bool,
    strings: Vec<Option<Vec<u8>>>,

    barcodes: Vec<Barcode>,
    barcode_count: usize,
    barcode_max: usize,

    /// The scratch bitmap everything is drawn into before being scaled.
    the_bitmap: Bitmap,

    mode: Mode,
    step: i32,
    grid_w: i32,
    grid_h: i32,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (w, h) = (d.width(), d.height());

    let mode = match d.res.string("mode").to_ascii_lowercase().as_str() {
        "grid" => Mode::Grid,
        "clock" | "clock12" => Mode::Clock12,
        "clock24" => Mode::Clock24,
        // Upstream exits on an unknown mode; scrolling is its own default.
        _ => Mode::Scroll,
    };

    let mut step = d.res.int("step");
    if step < 1 {
        step = 1;
    }
    if w > 2560 || h > 2560 {
        step *= 3; /* Retina displays */
    }

    let mut delay = d.res.int("delay").max(0) as u32;
    if mode == Mode::Clock12 || mode == Mode::Clock24 {
        delay = 10000; /* only update every 1/10th second */
    }

    let barcode_max = 20;
    Box::new(BarcodeHack {
        delay,
        window_width: w,
        window_height: h,
        fg_pixel: d.res.pixel("foreground"),
        bg_pixel: d.res.pixel("background"),
        grid_pixel: d.res.pixel("foreground"),
        button_down_p: false,
        grid_alloced_p: false,
        strings: (0..200).map(|_| None).collect(),
        barcodes: (0..barcode_max)
            .map(|_| Barcode {
                x: 0,
                y: 0,
                mag: 1,
                bitmap: Bitmap::new(BARCODE_WIDTH * MAX_MAG, BARCODE_HEIGHT * MAX_MAG),
                code: Vec::new(),
                pixel: 0,
            })
            .collect(),
        barcode_count: 0,
        barcode_max,
        the_bitmap: Bitmap::new(BARCODE_WIDTH * MAX_MAG, BARCODE_HEIGHT * MAX_MAG),
        mode,
        step,
        grid_w: 0,
        grid_h: 0,
    })
}

impl BarcodeHack {
    fn clockp(&self) -> bool {
        self.mode == Mode::Clock12 || self.mode == Mode::Clock24
    }

    /// Draw the given digit character; a '0' stands in for anything that is
    /// not one, except in the clock, which draws letters too.
    fn draw_digit_char(&self, b: &mut Bitmap, x: i32, y: i32, c: u8) {
        let c = if !self.clockp() && !c.is_ascii_digit() {
            b'0'
        } else {
            c
        };
        b.draw_char_5x8(x, y, c as char);
    }

    /// Draw a upc/ean digit at the given coordinates.
    fn draw_upc_ean_digit(b: &mut Bitmap, x: i32, y1: i32, y2: i32, n: u8, set: UpcSet) {
        let n = char_to_digit(n) as usize;
        let bits = match set {
            UpcSet::LeftA => UPC_LEFT_A[n],
            UpcSet::LeftB => UPC_LEFT_B[n],
            UpcSet::Right => UPC_RIGHT[n],
        };
        for (x, i) in (x..).zip((0..=6).rev()) {
            if bits & (1 << i) != 0 {
                b.vlin(x, y1, y2);
            }
        }
    }

    /// How wide a supplemental barcode is, in bar units.
    fn supplement_width(digits: &[u8]) -> i32 {
        match digits.len() {
            2 => 28, /* 8 + 4 + 2*7 + 1*2 */
            5 => 55, /* 8 + 4 + 5*7 + 4*2 */
            _ => 0,
        }
    }

    /// The little extra barcode that sits to the right of the main one.
    fn draw_supplemental_bars(
        &self,
        b: &mut Bitmap,
        digits: &[u8],
        x: i32,
        y: i32,
        y2: i32,
        text_above: bool,
    ) {
        let len = digits.len();
        let (mut y, mut y2) = (y, y2);
        let text_y;
        if text_above {
            text_y = y;
            y += 8;
        } else {
            y2 -= 8;
            text_y = y2 + 2;
        }
        let x = x + 8; /* skip the space between the main and supplemental */

        let (text_x, parity) = match len {
            2 => (
                x + 5,
                (char_to_digit(digits[0]) * 10 + char_to_digit(digits[1])) & 0x3,
            ),
            5 => {
                let p = ((char_to_digit(digits[0])
                    + char_to_digit(digits[2])
                    + char_to_digit(digits[4]))
                    * 3
                    + (char_to_digit(digits[1]) + char_to_digit(digits[3])) * 9)
                    % 10;
                (x + 10, UPC_E_LAST_DIGIT[p as usize])
            }
            // Upstream exits here; a supplement is only ever built with two or
            // five digits.
            _ => return,
        };

        /* header */
        b.vlin(x, y, y2);
        b.vlin(x + 2, y, y2);
        b.vlin(x + 3, y, y2);

        for (i, &d) in digits.iter().enumerate() {
            let lset = if parity & (1 << (len - 1 - i)) != 0 {
                UpcSet::LeftB
            } else {
                UpcSet::LeftA
            };
            let base_x = x + 2 + i as i32 * 9;

            /* separator / end of header */
            if i == 0 {
                b.vlin(base_x, y, y2);
            }
            b.vlin(base_x + 1, y, y2);

            Self::draw_upc_ean_digit(b, base_x + 2, y, y2, d, lset);
            self.draw_digit_char(b, text_x + i as i32 * 6, text_y, d);
        }
    }

    fn draw_upc_a_bars(b: &mut Bitmap, digits: &[u8], x: i32, y: i32, bar_y2: i32, guard_y2: i32) {
        /* header */
        b.vlin(x, y, guard_y2);
        b.vlin(x + 2, y, guard_y2);
        /* center marker */
        b.vlin(x + 46, y, guard_y2);
        b.vlin(x + 48, y, guard_y2);
        /* trailer */
        b.vlin(x + 92, y, guard_y2);
        b.vlin(x + 94, y, guard_y2);

        for i in 0..6 {
            Self::draw_upc_ean_digit(
                b,
                x + 3 + i * 7,
                y,
                if i == 0 { guard_y2 } else { bar_y2 },
                digits[i as usize],
                UpcSet::LeftA,
            );
            Self::draw_upc_ean_digit(
                b,
                x + 50 + i * 7,
                y,
                if i == 5 { guard_y2 } else { bar_y2 },
                digits[i as usize + 6],
                UpcSet::Right,
            );
        }
    }

    fn make_upc_a_full(&self, dest: &mut Bitmap, digits: &[u8], y: i32) -> i32 {
        let base_width = 108;
        let height = 60 + y;

        dest.clear();
        Self::draw_upc_a_bars(dest, digits, 6, y, height - 10, height - 4);
        self.draw_digit_char(dest, 0, height - 14, digits[0]);
        for i in 0..5 {
            self.draw_digit_char(dest, 18 + i * 7, height - 7, digits[i as usize + 1]);
            self.draw_digit_char(dest, 57 + i * 7, height - 7, digits[i as usize + 6]);
        }
        self.draw_digit_char(dest, 103, height - 14, digits[11]);
        base_width
    }

    fn make_upc_a(&self, dest: &mut Bitmap, digits: &mut [u8], y: i32) -> i32 {
        let mut mul = 3;
        let mut sum = 0;
        for &d in digits.iter().take(11) {
            sum += char_to_digit(d) * mul;
            mul ^= 2;
        }
        if digits[11] == b'?' {
            digits[11] = ((10 - (sum % 10)) % 10) as u8 + b'0';
        }
        self.make_upc_a_full(dest, digits, y)
    }

    fn draw_upc_e_bars(
        &self,
        b: &mut Bitmap,
        digits: &[u8],
        x: i32,
        y: i32,
        bar_y2: i32,
        guard_y2: i32,
    ) {
        let mut parity = UPC_E_LAST_DIGIT[char_to_digit(digits[7]) as usize];
        let clockp = self.clockp();
        if digits[0] == b'1' {
            parity = !parity;
        }

        /* header */
        b.vlin(x, y, guard_y2);
        b.vlin(x + 2, y, guard_y2);
        /* trailer */
        let t = if clockp { 8 } else { 0 };
        b.vlin(x + 46 + t, y, guard_y2);
        b.vlin(x + 48 + t, y, guard_y2);
        b.vlin(x + 50 + t, y, guard_y2);

        // Clock kludge: an extra set of dividers after digits 2 and 4. This
        // makes it *not* a valid barcode, but it looks pretty as a clock.
        if clockp {
            b.vlin(x + 18, y, guard_y2);
            b.vlin(x + 18 + 2, y, guard_y2);
            b.vlin(x + 36, y, guard_y2);
            b.vlin(x + 36 + 2, y, guard_y2);
        }

        for i in 0..6 {
            let lset = if parity & (1 << (5 - i)) != 0 {
                UpcSet::LeftB
            } else {
                UpcSet::LeftA
            };
            let off = if clockp {
                if i < 2 {
                    0
                } else if i < 4 {
                    4 /* extra spacing for clock bars */
                } else {
                    8
                }
            } else {
                0
            };
            Self::draw_upc_ean_digit(
                b,
                x + 3 + i * 7 + off,
                y,
                bar_y2,
                digits[i as usize + 1],
                lset,
            );
        }
    }

    fn make_upc_e_full(&self, dest: &mut Bitmap, digits: &[u8], y: i32) -> i32 {
        let base_width = 64;
        let height = 60 + y;

        dest.clear();
        self.draw_upc_e_bars(dest, digits, 6, y, height - 10, height - 4);
        self.draw_digit_char(dest, 0, height - 14, digits[0]);
        for i in 0..6 {
            self.draw_digit_char(dest, 11 + i * 7, height - 7, digits[i as usize + 1]);
        }
        self.draw_digit_char(dest, 59, height - 14, digits[7]);
        base_width
    }

    fn make_upc_e(&self, dest: &mut Bitmap, digits: &[u8], y: i32) -> i32 {
        let mut compressed = [b'0'; 9];
        for (i, &d) in digits.iter().take(7).enumerate() {
            compressed[i + 1] = d;
        }
        let Some(expanded) = expand_to_upc_a_digits(&compressed) else {
            return 0;
        };
        compressed[7] = expanded[11];
        self.make_upc_e_full(dest, &compressed, y)
    }

    fn draw_ean13_bars(b: &mut Bitmap, digits: &[u8], x: i32, y: i32, bar_y2: i32, guard_y2: i32) {
        let left_pattern = EAN13_FIRST_DIGIT[char_to_digit(digits[0]) as usize];

        /* header */
        b.vlin(x, y, guard_y2);
        b.vlin(x + 2, y, guard_y2);
        /* center marker */
        b.vlin(x + 46, y, guard_y2);
        b.vlin(x + 48, y, guard_y2);
        /* trailer */
        b.vlin(x + 92, y, guard_y2);
        b.vlin(x + 94, y, guard_y2);

        for i in 0..6 {
            let lset = if left_pattern & (1 << (5 - i)) != 0 {
                UpcSet::LeftB
            } else {
                UpcSet::LeftA
            };
            Self::draw_upc_ean_digit(b, x + 3 + i * 7, y, bar_y2, digits[i as usize + 1], lset);
            Self::draw_upc_ean_digit(
                b,
                x + 50 + i * 7,
                y,
                bar_y2,
                digits[i as usize + 7],
                UpcSet::Right,
            );
        }
    }

    fn make_ean13_full(&self, dest: &mut Bitmap, digits: &[u8], y: i32) -> i32 {
        let base_width = 102;
        let height = 60 + y;

        dest.clear();
        Self::draw_ean13_bars(dest, digits, 6, y, height - 10, height - 4);
        self.draw_digit_char(dest, 0, height - 7, digits[0]);
        for i in 0..6 {
            self.draw_digit_char(dest, 11 + i * 7, height - 7, digits[i as usize + 1]);
            self.draw_digit_char(dest, 57 + i * 7, height - 7, digits[i as usize + 7]);
        }
        base_width
    }

    fn make_ean13(&self, dest: &mut Bitmap, digits: &mut [u8], y: i32) -> i32 {
        let mut mul = 1;
        let mut sum = 0;
        for &d in digits.iter().take(12) {
            sum += char_to_digit(d) * mul;
            mul ^= 2;
        }
        if digits[12] == b'?' {
            digits[12] = ((10 - (sum % 10)) % 10) as u8 + b'0';
        }
        self.make_ean13_full(dest, digits, y)
    }

    fn draw_ean8_bars(b: &mut Bitmap, digits: &[u8], x: i32, y: i32, bar_y2: i32, guard_y2: i32) {
        /* header */
        b.vlin(x, y, guard_y2);
        b.vlin(x + 2, y, guard_y2);
        /* center marker */
        b.vlin(x + 32, y, guard_y2);
        b.vlin(x + 34, y, guard_y2);
        /* trailer */
        b.vlin(x + 64, y, guard_y2);
        b.vlin(x + 66, y, guard_y2);

        for i in 0..4 {
            Self::draw_upc_ean_digit(
                b,
                x + 3 + i * 7,
                y,
                bar_y2,
                digits[i as usize],
                UpcSet::LeftA,
            );
            Self::draw_upc_ean_digit(
                b,
                x + 36 + i * 7,
                y,
                bar_y2,
                digits[i as usize + 4],
                UpcSet::Right,
            );
        }
    }

    fn make_ean8_full(&self, dest: &mut Bitmap, digits: &[u8], y: i32) -> i32 {
        let base_width = 68;
        let height = 60 + y;

        dest.clear();
        Self::draw_ean8_bars(dest, digits, 0, y, height - 10, height - 4);
        for i in 0..4 {
            self.draw_digit_char(dest, 5 + i * 7, height - 7, digits[i as usize]);
            self.draw_digit_char(dest, 37 + i * 7, height - 7, digits[i as usize + 4]);
        }
        base_width
    }

    fn make_ean8(&self, dest: &mut Bitmap, digits: &mut [u8], y: i32) -> i32 {
        let mut mul = 3;
        let mut sum = 0;
        for &d in digits.iter().take(7) {
            sum += char_to_digit(d) * mul;
            mul ^= 2;
        }
        if digits[7] == b'?' {
            digits[7] = ((10 - (sum % 10)) % 10) as u8 + b'0';
        }
        self.make_ean8_full(dest, digits, y)
    }

    /// Dispatch to the right form factor. The string is digits, optionally a
    /// comma and a two or five digit supplement, optionally a colon and the
    /// banner to write across the top.
    fn process_upc_ean(&self, str: &[u8], dest: &mut Bitmap) {
        let mut digits: Vec<u8> = Vec::new();
        let mut sup_digits: Vec<u8> = Vec::new();
        let mut banner: Option<&[u8]> = None;
        let mut supplement = false;
        let vstart = 9;

        let mut i = 0;
        while digits.len() < 15 && sup_digits.len() < 7 && i < str.len() {
            let c = str[i];
            if c.is_ascii_digit() || c == b'?' {
                if supplement {
                    sup_digits.push(c);
                } else {
                    digits.push(c);
                }
            } else if c == b',' {
                supplement = true;
            } else if c == b':' {
                banner = Some(&str[i + 1..]);
                break;
            }
            i += 1;
        }

        let sup_width = if sup_digits.is_empty() {
            0
        } else {
            // Upstream exits on any other length; nothing here builds one.
            Self::supplement_width(&sup_digits)
        };

        let width = match digits.len() {
            7 => self.make_upc_e(dest, &digits, vstart),
            8 => self.make_ean8(dest, &mut digits, vstart),
            12 => self.make_upc_a(dest, &mut digits, vstart),
            13 => self.make_ean13(dest, &mut digits, vstart),
            // Upstream exits on a bad length.
            _ => return,
        };

        if sup_width != 0 {
            let h = dest.height;
            self.draw_supplemental_bars(dest, &sup_digits, width, vstart + 1, h - 4, true);
        }

        let banner = banner.unwrap_or(b"barcode");
        let text = String::from_utf8_lossy(banner).to_string();
        dest.draw_string_5x8(
            (width + sup_width - text.chars().count() as i32 * 5) / 2,
            0,
            &text,
        );
    }

    /// Make a new barcode string: some digits, a check digit to be worked out,
    /// maybe a supplement, and a word.
    fn make_barcode_string(&self) -> Vec<u8> {
        let dig = match (rand_float_01() * 4.0) as i32 {
            0 => 6,
            1 => 7,
            2 => 11,
            _ => 12,
        };
        let mut s = Vec::new();
        for _ in 0..dig {
            s.push((rand_float_01() * 10.0) as u8 + b'0');
        }
        s.push(b'?');

        let dig = match (rand_float_01() * 3.0) as i32 {
            0 => 0,
            1 => 2,
            _ => 5,
        };
        if dig != 0 {
            s.push(b',');
            for _ in 0..dig {
                s.push((rand_float_01() * 10.0) as u8 + b'0');
            }
        }
        s.push(b':');
        s.extend_from_slice(WORDS[(rand_float_01() * WORDS.len() as f64) as usize].as_bytes());
        s
    }

    fn scroll_model(&mut self) {
        let mut i = 0;
        while i < self.barcode_count {
            self.barcodes[i].x -= self.step;
            if self.barcodes[i].x + BARCODE_WIDTH * self.barcodes[i].mag < 0 {
                /* fell off the edge */
                if i != self.barcode_count - 1 {
                    self.barcodes[i..self.barcode_count].rotate_left(1);
                }
                self.barcode_count -= 1;
                continue;
            }
            i += 1;
        }

        while self.barcode_count < self.barcode_max {
            let n = self.barcode_count;
            let mut x = if n == 0 {
                0
            } else {
                self.barcodes[n - 1].x + self.barcodes[n - 1].mag * BARCODE_WIDTH
            };
            x += (rand_float_01() * 100.0) as i32;
            let mut mag = (rand_float_01() * f64::from(MAX_MAG)) as i32;
            if self.window_width < 100 || self.window_height < 100 {
                mag /= 2;
                if mag <= 0 {
                    mag = 1;
                }
            }
            let mut y =
                (rand_float_01() * f64::from(self.window_height - BARCODE_HEIGHT * mag)) as i32;
            if y < 0 {
                y = 0;
            }

            // The barcode is drawn into the scratch bitmap and scaled out of
            // it, so both have to be moved out of `self` while it is borrowed.
            let code = self.make_barcode_string();
            let mut scratch = std::mem::replace(&mut self.the_bitmap, Bitmap::new(1, 1));
            scratch.clear();
            self.process_upc_ean(&code, &mut scratch);
            let mut bm = std::mem::replace(&mut self.barcodes[n].bitmap, Bitmap::new(1, 1));
            bm.clear();
            bm.scale_from(&scratch, mag.max(1));
            self.barcodes[n].bitmap = bm;
            self.the_bitmap = scratch;

            let (r, g, b) = hsv_to_rgb(random_below(360), 1.0, 1.0);
            let bc = &mut self.barcodes[n];
            bc.x = x;
            bc.y = y;
            bc.mag = mag;
            bc.code = code;
            bc.pixel = XColor::from_rgb16(r, g, b).pixel;
            self.barcode_count += 1;
        }
    }

    fn update_grid(&mut self, d: &mut Dpy) {
        if self.grid_w == 0 || self.grid_h == 0 || random_below(400) == 0 {
            d.clear_window();
            self.grid_w = 1 + random_below(3);
            self.grid_h = 1 + random_below(4);
        }

        if !self.grid_alloced_p || random_below(100) == 0 {
            let (r, g, b) = hsv_to_rgb(random_below(360), 1.0, 1.0);
            self.grid_pixel = XColor::from_rgb16(r, g, b).pixel;
            self.grid_alloced_p = true;
        }

        self.barcode_count = (self.grid_w * self.grid_h) as usize;
        for b in self.barcodes.iter_mut() {
            b.x = 999999;
            b.y = 999999;
        }

        let mut i = 0;
        for y in 0..self.grid_h {
            for x in 0..self.grid_w {
                let digits = 12;
                let cell_w = self.window_width / self.grid_w;
                let cell_h = self.window_height / self.grid_h;
                let mag_x = cell_w / BARCODE_WIDTH;
                let mag_y = cell_h / BARCODE_HEIGHT;
                let bw = 108; /* BARCODE_WIDTH */
                let bh = BARCODE_HEIGHT;
                let mag = mag_x.min(mag_y);

                if self.strings[i].is_none() {
                    let mut s: Vec<u8> =
                        (0..digits).map(|_| random_below(10) as u8 + b'0').collect();
                    s.push(b'?');
                    s.push(b':');
                    self.strings[i] = Some(s);
                }
                /* change one digit in this barcode */
                if let Some(s) = &mut self.strings[i] {
                    let j = random_below(digits) as usize;
                    s[j] = random_below(10) as u8 + b'0';
                }
                let code = self.strings[i].clone().unwrap_or_default();

                let mut bm = std::mem::replace(&mut self.barcodes[i].bitmap, Bitmap::new(1, 1));
                bm.clear();
                self.process_upc_ean(&code, &mut bm);
                self.barcodes[i].bitmap = bm;

                let b = &mut self.barcodes[i];
                b.mag = mag;
                b.x = x * cell_w + (cell_w - mag * bw) / 2;
                b.y = y * cell_h + (cell_h - mag * bh) / 2;
                b.pixel = self.grid_pixel;
                b.code = code;
                i += 1;
            }
        }
    }

    /// This one draws a clock. By jwz.
    fn update_clock(&mut self, d: &mut Dpy) {
        let bw = 76; /* BARCODE_WIDTH */
        let bh = BARCODE_HEIGHT;
        let secs = d.wall_clock();
        let hour = (secs / 3600.0) as i32 % 24;
        let min = (secs / 60.0) as i32 % 60;
        let sec = secs as i32 % 60;

        let mag_x = self.window_width / bw;
        let mag_y = self.window_height / bh;
        self.barcode_count = 1;
        let mag = mag_x.min(mag_y).clamp(1, MAX_MAG);

        let code = if !self.button_down_p {
            let h = if self.mode == Mode::Clock24 {
                hour
            } else if hour > 12 {
                hour - 12
            } else if hour == 0 {
                12
            } else {
                hour
            };
            format!("0{h:02}{min:02}{sec:02}?:")
        } else {
            // Upstream shows the date while the button is held. There is no
            // calendar here, so this is the same clock the other way up.
            format!("0{sec:02}{min:02}{hour:02}?:")
        };
        let mut code = code.into_bytes();

        let vstart = 9;
        let hh = bh + vstart;
        if let Some(expanded) = expand_to_upc_a_digits(&code) {
            code[7] = expanded[11];
        }

        let mut scratch = std::mem::replace(&mut self.the_bitmap, Bitmap::new(1, 1));
        scratch.clear();
        self.draw_upc_e_bars(&mut scratch, &code, 6, 9, 59, 65);
        for i in 0..6 {
            let off = if i < 2 {
                0
            } else if i < 4 {
                4
            } else {
                8
            };
            self.draw_digit_char(
                &mut scratch,
                11 + i * 7 + off,
                hh - 16,
                code[i as usize + 1],
            );
        }
        if !self.button_down_p {
            let am = if hour < 12 { b'A' } else { b'P' };
            self.draw_digit_char(&mut scratch, 0, hh - 23, am);
            self.draw_digit_char(&mut scratch, 68, hh - 23, b'M');
        }

        let mut bm = std::mem::replace(&mut self.barcodes[0].bitmap, Bitmap::new(1, 1));
        bm.clear();
        bm.scale_from(&scratch, mag);
        self.barcodes[0].bitmap = bm;
        self.the_bitmap = scratch;

        let b = &mut self.barcodes[0];
        b.mag = mag;
        b.x = (self.window_width - mag * bw) / 2;
        b.y = (self.window_height - mag * (bh + 9)) / 2;
        b.pixel = self.fg_pixel;
        b.code = code;
    }

    /// Render the current model.
    ///
    /// Upstream hands the packed bitmap to `XPutImage` as a depth-one image,
    /// which paints the clear bits in the background as well as the set ones in
    /// the foreground. That is what stops a scrolling barcode from smearing, so
    /// this paints both, and clips to the window rather than drawing the parts
    /// that are off it.
    fn render_frame(&mut self, d: &mut Dpy) {
        for i in 0..self.barcode_count {
            let b = &self.barcodes[i];
            if b.x > self.window_width {
                break;
            }
            let (w, h) = (BARCODE_WIDTH * b.mag, BARCODE_HEIGHT * b.mag);
            let x0 = b.x.max(0);
            let y0 = b.y.max(0);
            let x1 = (b.x + w).min(self.window_width);
            let y1 = (b.y + h).min(self.window_height);
            let (fg, bg) = (b.pixel, self.bg_pixel);
            for y in y0..y1 {
                for x in x0..x1 {
                    let v = b.bitmap.get(x - b.x, y - b.y);
                    d.win().put_pixel(x, y, if v != 0 { fg } else { bg });
                }
            }
        }
    }
}

/// Expand eight UPC-E digits into a UPC-A number, or `None` if the form factor
/// is wrong. Also works out the check digit when it is given as '?'.
fn expand_to_upc_a_digits(compressed: &[u8]) -> Option<[u8; 12]> {
    if compressed.len() < 8 || (compressed[0] != b'0' && compressed[0] != b'1') {
        return None;
    }
    let mut e = [b'0'; 12];
    e[0] = compressed[0];
    e[6] = b'0';
    e[7] = b'0';
    e[11] = compressed[7];

    match compressed[6] {
        b'0' | b'1' | b'2' => {
            e[1] = compressed[1];
            e[2] = compressed[2];
            e[3] = compressed[6];
            e[4] = b'0';
            e[5] = b'0';
            e[8] = compressed[3];
            e[9] = compressed[4];
            e[10] = compressed[5];
        }
        b'3' => {
            e[1] = compressed[1];
            e[2] = compressed[2];
            e[3] = compressed[3];
            e[4] = b'0';
            e[5] = b'0';
            e[8] = b'0';
            e[9] = compressed[4];
            e[10] = compressed[5];
        }
        b'4' => {
            e[1] = compressed[1];
            e[2] = compressed[2];
            e[3] = compressed[3];
            e[4] = compressed[4];
            e[5] = b'0';
            e[8] = b'0';
            e[9] = b'0';
            e[10] = compressed[5];
        }
        _ => {
            e[1] = compressed[1];
            e[2] = compressed[2];
            e[3] = compressed[3];
            e[4] = compressed[4];
            e[5] = compressed[5];
            e[8] = b'0';
            e[9] = b'0';
            e[10] = compressed[6];
        }
    }

    if e[11] == b'?' {
        let mut mul = 3;
        let mut sum = 0;
        for &d in e.iter().take(11) {
            sum += char_to_digit(d) * mul;
            mul ^= 2;
        }
        e[11] = ((10 - (sum % 10)) % 10) as u8 + b'0';
    }
    Some(e)
}

impl Screenhack for BarcodeHack {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        match self.mode {
            Mode::Scroll => self.scroll_model(),
            Mode::Grid => self.update_grid(d),
            Mode::Clock12 | Mode::Clock24 => self.update_clock(d),
        }
        self.render_frame(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.window_width = width;
        self.window_height = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if !self.clockp() {
            return false;
        }
        match event {
            XEvent::ButtonPress { .. } => {
                self.button_down_p = true;
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.button_down_p = false;
                true
            }
            _ => false,
        }
    }
}

/// A bunch of words, one of which goes under each barcode.
static WORDS: [&str; 313] = [
    "abdomen",
    "abeyance",
    "abhorrence",
    "abrasion",
    "abstraction",
    "acid",
    "addiction",
    "affluenza",
    "alertness",
    "Algeria",
    "all your base",
    "antifa",
    "anxiety",
    "aorta",
    "argyle socks",
    "attrition",
    "axis of evil",
    "bamboo",
    "banana slug",
    "bangle",
    "bankruptcy",
    "baptism",
    "beer",
    "bellicosity",
    "bells",
    "belly",
    "bird flu",
    "Bitcorn",
    "bliss",
    "bogosity",
    "boobies",
    "boobs",
    "booty",
    "bread",
    "bribe",
    "brogrammers",
    "bubba",
    "burrito",
    "California",
    "cancer",
    "capybara",
    "cardinality",
    "caribou",
    "carnage",
    "children",
    "chocolate",
    "chupacabra",
    "CLONE",
    "cock",
    "congress",
    "constriction",
    "contrition",
    "cop",
    "corpse",
    "coronavirus",
    "covid-19",
    "cowboy",
    "crabapple",
    "craziness",
    "cthulhu",
    "Death",
    "decepticon",
    "deception",
    "Decker",
    "decoder",
    "decoy",
    "defenestration",
    "democracy",
    "dependency",
    "despair",
    "desperation",
    "disease",
    "DNA Lounge",
    "doberman",
    "DOOM",
    "doom loop",
    "dot com",
    "dreams",
    "drugs",
    "Dunning-Krugerrands",
    "easy",
    "ebony",
    "election",
    "eloquence",
    "emergency",
    "emolument",
    "eureka",
    "excommunication",
    "fat",
    "fatherland",
    "Faust",
    "fear",
    "fever",
    "filth",
    "flatulence",
    "fluff",
    "fnord",
    "followers",
    "frak",
    "freedom",
    "fruit",
    "futility",
    "gerbils",
    "GOD",
    "goggles",
    "goobers",
    "gorilla",
    "guillotine",
    "H5N1",
    "halibut",
    "handmaid",
    "happiness",
    "hate",
    "helplessness",
    "hemorrhoid",
    "hermaphrodite",
    "heroin",
    "heroine",
    "hope",
    "horse paste",
    "hysteria",
    "icepick",
    "identity",
    "ignorance",
    "illuminati",
    "importance",
    "individuality",
    "influence",
    "influencers",
    "influenza",
    "inkling",
    "insurrection",
    "intoxicant",
    "ire",
    "irritant",
    "jade",
    "jaundice",
    "Joyce",
    "kidney stone",
    "kitchenette",
    "kiwi",
    "lathe",
    "lattice",
    "lawyer",
    "lemming",
    "likes",
    "liquidation",
    "lobbyist",
    "love",
    "lozenge",
    "magazine",
    "magnesium",
    "malfunction",
    "marmot",
    "marshmallow",
    "measles",
    "merit",
    "merkin",
    "mescaline",
    "methane",
    "milk",
    "mischief",
    "mistrust",
    "money",
    "monkey",
    "monkeybutter",
    "nationalism",
    "nature",
    "neuron",
    "NFTs",
    "noise",
    "nomenclature",
    "NULL",
    "null island",
    "nutria",
    "OBEY",
    "ouroboros",
    "ocelot",
    "offspring",
    "overseer",
    "pain",
    "pajamas",
    "passenger",
    "passion",
    "Passover",
    "peace",
    "penance",
    "persimmon",
    "petticoat",
    "pharmacist",
    "PhD",
    "pitchfork",
    "plague",
    "Poindexter",
    "politician",
    "pony",
    "presidency",
    "prison",
    "prophecy",
    "Prozac",
    "punishment",
    "punk rock",
    "punk",
    "pussy",
    "quagmire",
    "quarantine",
    "quartz",
    "rabies",
    "radish",
    "rage",
    "readout",
    "reality",
    "rectum",
    "reject",
    "rejection",
    "respect",
    "revolution",
    "roadrunner",
    "rootkit",
    "rule",
    "SARS",
    "SARS-CoV-2",
    "savor",
    "scab",
    "scalar",
    "Scandinavia",
    "schadenfreude",
    "security",
    "sediment",
    "self worth",
    "shadow profile",
    "sickness",
    "silicone",
    "slack",
    "slander",
    "slavery",
    "sledgehammer",
    "smegma",
    "smelly socks",
    "sorrow",
    "space program",
    "stamen",
    "standardization",
    "stench",
    "subculture",
    "subversion",
    "suffering",
    "surrender",
    "surveillance",
    "synthesis",
    "television",
    "tenant",
    "tendril",
    "teratoma",
    "terror",
    "terrorism",
    "terrorist",
    "the impossible",
    "the panopticon",
    "the unknown",
    "toast",
    "topography",
    "tribble",
    "truism",
    "truthiness",
    "turgid",
    "twits",
    "undef",
    "underbrush",
    "underling",
    "unguent",
    "unusual",
    "uplink",
    "urge",
    "vaccines",
    "valor",
    "variance",
    "vaudeville",
    "vector",
    "vegetarian",
    "venom",
    "verifiability",
    "very fine people",
    "viagra",
    "vibrator",
    "victim",
    "vignette",
    "villainy",
    "W.A.S.T.E.",
    "wagon",
    "waiver",
    "warehouse",
    "waste",
    "waveform",
    "whiffle ball",
    "whorl",
    "windmill",
    "words",
    "worm",
    "worship",
    "Xanax",
    "Xerxes",
    "Xhosa",
    "xylophone",
    "yellow",
    "yesterday",
    "your nose",
    "Y2038",
    "Zanzibar",
    "zeal",
    "zebra",
    "zest",
    "zinc",
];

/// The 5x8 font this carries rather than asking the server for one: eight
/// rows for each of 128 characters, one byte a row, leftmost pixel in the
/// low bit.
static FONT5X8: [u8; 1024] = [
    0x1e, 0x01, 0x06, 0x01, 0x1e, 0x00, 0x1e, 0x01, 0x06, 0x01, 0x1e, 0x00, 0x1e, 0x01, 0x1e, 0x01,
    0x1e, 0x00, 0x01, 0x00, 0x1f, 0x08, 0x04, 0x08, 0x1f, 0x00, 0x11, 0x1f, 0x11, 0x00, 0x1f, 0x01,
    0x01, 0x00, 0x1f, 0x04, 0x0a, 0x11, 0x00, 0x01, 0x00, 0x0e, 0x11, 0x11, 0x00, 0x0e, 0x11, 0x11,
    0x0e, 0x00, 0x1f, 0x08, 0x04, 0x08, 0x1f, 0x00, 0x44, 0x41, 0x4e, 0x20, 0x42, 0x4f, 0x52, 0x4e,
    0x53, 0x54, 0x45, 0x49, 0x4e, 0x21, 0x21, 0x00, 0x66, 0x6e, 0x6f, 0x72, 0x64, 0x00, 0x00, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x02, 0x02, 0x02, 0x00, 0x02, 0x00,
    0x05, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x05, 0x0f, 0x05, 0x0f, 0x05, 0x05, 0x00,
    0x02, 0x0f, 0x01, 0x0f, 0x08, 0x0f, 0x04, 0x00, 0x0b, 0x0b, 0x08, 0x06, 0x01, 0x0d, 0x0d, 0x00,
    0x03, 0x05, 0x02, 0x05, 0x0d, 0x05, 0x0b, 0x00, 0x04, 0x04, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x04, 0x02, 0x02, 0x02, 0x02, 0x02, 0x04, 0x00, 0x02, 0x04, 0x04, 0x04, 0x04, 0x04, 0x02, 0x00,
    0x00, 0x09, 0x06, 0x0f, 0x06, 0x09, 0x00, 0x00, 0x00, 0x02, 0x02, 0x07, 0x02, 0x02, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x06, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x08, 0x08, 0x04, 0x06, 0x02, 0x01, 0x01, 0x00,
    0x0f, 0x09, 0x09, 0x09, 0x09, 0x09, 0x0f, 0x00, 0x06, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0f, 0x00,
    0x0f, 0x09, 0x08, 0x0f, 0x01, 0x09, 0x0f, 0x00, 0x0f, 0x08, 0x08, 0x0f, 0x08, 0x08, 0x0f, 0x00,
    0x09, 0x09, 0x09, 0x0f, 0x08, 0x08, 0x08, 0x00, 0x0f, 0x09, 0x01, 0x0f, 0x08, 0x09, 0x0f, 0x00,
    0x03, 0x01, 0x01, 0x0f, 0x09, 0x09, 0x0f, 0x00, 0x0f, 0x09, 0x09, 0x0c, 0x04, 0x04, 0x04, 0x00,
    0x0f, 0x09, 0x09, 0x0f, 0x09, 0x09, 0x0f, 0x00, 0x0f, 0x09, 0x09, 0x0f, 0x08, 0x08, 0x08, 0x00,
    0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x04, 0x04, 0x06, 0x00,
    0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x0f, 0x00, 0x00, 0x00,
    0x01, 0x02, 0x04, 0x08, 0x04, 0x02, 0x01, 0x00, 0x0f, 0x09, 0x08, 0x0e, 0x02, 0x00, 0x02, 0x00,
    0x0f, 0x09, 0x0d, 0x0d, 0x0d, 0x01, 0x0f, 0x00, 0x0f, 0x09, 0x09, 0x0f, 0x09, 0x09, 0x09, 0x00,
    0x07, 0x09, 0x09, 0x07, 0x09, 0x09, 0x07, 0x00, 0x0f, 0x01, 0x01, 0x01, 0x01, 0x01, 0x0f, 0x00,
    0x07, 0x09, 0x09, 0x09, 0x09, 0x09, 0x07, 0x00, 0x0f, 0x01, 0x01, 0x0f, 0x01, 0x01, 0x0f, 0x00,
    0x0f, 0x01, 0x01, 0x0f, 0x01, 0x01, 0x01, 0x00, 0x0f, 0x01, 0x01, 0x0d, 0x09, 0x09, 0x0f, 0x00,
    0x09, 0x09, 0x09, 0x0f, 0x09, 0x09, 0x09, 0x00, 0x07, 0x02, 0x02, 0x02, 0x02, 0x02, 0x07, 0x00,
    0x0e, 0x04, 0x04, 0x04, 0x04, 0x05, 0x07, 0x00, 0x09, 0x09, 0x09, 0x07, 0x09, 0x09, 0x09, 0x00,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x0f, 0x00, 0x09, 0x0f, 0x0f, 0x0f, 0x09, 0x09, 0x09, 0x00,
    0x09, 0x0b, 0x0d, 0x09, 0x09, 0x09, 0x09, 0x00, 0x0f, 0x09, 0x09, 0x09, 0x09, 0x09, 0x0f, 0x00,
    0x0f, 0x09, 0x09, 0x0f, 0x01, 0x01, 0x01, 0x00, 0x0f, 0x09, 0x09, 0x09, 0x0b, 0x05, 0x0b, 0x00,
    0x07, 0x09, 0x09, 0x07, 0x09, 0x09, 0x09, 0x00, 0x0f, 0x01, 0x01, 0x0f, 0x08, 0x08, 0x0f, 0x00,
    0x0f, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x00, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x0f, 0x00,
    0x09, 0x09, 0x09, 0x09, 0x09, 0x05, 0x02, 0x00, 0x09, 0x09, 0x09, 0x09, 0x0f, 0x0f, 0x09, 0x00,
    0x09, 0x09, 0x05, 0x06, 0x0a, 0x09, 0x09, 0x00, 0x09, 0x09, 0x09, 0x0f, 0x08, 0x08, 0x0f, 0x00,
    0x0f, 0x08, 0x08, 0x06, 0x01, 0x01, 0x0f, 0x00, 0x0e, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0e, 0x00,
    0x01, 0x01, 0x02, 0x06, 0x04, 0x08, 0x08, 0x00, 0x07, 0x04, 0x04, 0x04, 0x04, 0x04, 0x07, 0x00,
    0x02, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x00,
    0x02, 0x02, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x08, 0x0f, 0x09, 0x0f, 0x00,
    0x01, 0x01, 0x0f, 0x09, 0x09, 0x09, 0x0f, 0x00, 0x00, 0x00, 0x0f, 0x01, 0x01, 0x01, 0x0f, 0x00,
    0x08, 0x08, 0x0f, 0x09, 0x09, 0x09, 0x0f, 0x00, 0x00, 0x00, 0x0f, 0x09, 0x0f, 0x01, 0x0f, 0x00,
    0x0e, 0x02, 0x0f, 0x02, 0x02, 0x02, 0x02, 0x00, 0x00, 0x00, 0x0f, 0x09, 0x09, 0x0f, 0x08, 0x0c,
    0x01, 0x01, 0x0f, 0x09, 0x09, 0x09, 0x09, 0x00, 0x02, 0x00, 0x03, 0x02, 0x02, 0x02, 0x07, 0x00,
    0x04, 0x00, 0x04, 0x04, 0x04, 0x04, 0x05, 0x07, 0x01, 0x01, 0x09, 0x05, 0x03, 0x05, 0x09, 0x00,
    0x03, 0x02, 0x02, 0x02, 0x02, 0x02, 0x07, 0x00, 0x00, 0x00, 0x09, 0x0f, 0x0f, 0x09, 0x09, 0x00,
    0x00, 0x00, 0x0f, 0x09, 0x09, 0x09, 0x09, 0x00, 0x00, 0x00, 0x0f, 0x09, 0x09, 0x09, 0x0f, 0x00,
    0x00, 0x00, 0x0f, 0x09, 0x09, 0x0f, 0x01, 0x01, 0x00, 0x00, 0x0f, 0x09, 0x09, 0x0f, 0x08, 0x08,
    0x00, 0x00, 0x0f, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x01, 0x0f, 0x08, 0x0f, 0x00,
    0x00, 0x02, 0x0f, 0x02, 0x02, 0x02, 0x0e, 0x00, 0x00, 0x00, 0x09, 0x09, 0x09, 0x09, 0x0f, 0x00,
    0x00, 0x00, 0x09, 0x09, 0x09, 0x05, 0x02, 0x00, 0x00, 0x00, 0x09, 0x09, 0x0f, 0x0f, 0x09, 0x00,
    0x00, 0x00, 0x09, 0x09, 0x06, 0x09, 0x09, 0x00, 0x00, 0x00, 0x09, 0x09, 0x09, 0x0f, 0x08, 0x0c,
    0x00, 0x00, 0x0f, 0x08, 0x06, 0x01, 0x0f, 0x00, 0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08, 0x00,
    0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x00, 0x01, 0x02, 0x02, 0x04, 0x02, 0x02, 0x01, 0x00,
    0x00, 0x0a, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x00,
];

const DEFAULTS: &[&str] = &[
    ".background:	black",
    ".foreground:	green",
    "*fpsSolid: 	true",
    "*step:	 	1",
    "*delay:		10000",
    "*mode:		scroll",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "scroll",
        label: "Scrolling barcodes",
    },
    SelectItem {
        value: "grid",
        label: "Barcode grid",
    },
    SelectItem {
        value: "clock12",
        label: "Barcode clock (AM/PM)",
    },
    SelectItem {
        value: "clock24",
        label: "Barcode clock (24 hour)",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::select("mode", "Mode", MODES, "scroll"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "barcode",
    label: "Barcode",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dan Bornstein and Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=gmtAySJdsfg"),
        blurb: "Scrolling UPC-A, UPC-E, EAN-8 and EAN-13 barcodes. CONSUME!",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    /// The check digit is the whole point of a barcode: a transcription slip
    /// in the digit tables would still draw bars, and they would be wrong.
    /// These are real numbers with published check digits.
    #[test]
    fn check_digits_come_out_right() {
        let hack = BarcodeHack {
            delay: 0,
            window_width: 100,
            window_height: 100,
            fg_pixel: 0,
            bg_pixel: 0,
            grid_pixel: 0,
            button_down_p: false,
            grid_alloced_p: false,
            strings: Vec::new(),
            barcodes: Vec::new(),
            barcode_count: 0,
            barcode_max: 0,
            the_bitmap: Bitmap::new(BARCODE_WIDTH, BARCODE_HEIGHT),
            mode: Mode::Scroll,
            step: 1,
            grid_w: 0,
            grid_h: 0,
        };
        let mut dest = Bitmap::new(BARCODE_WIDTH, BARCODE_HEIGHT);

        // UPC-A: 03600029145 + check digit 2.
        let mut d = b"03600029145?".to_vec();
        hack.make_upc_a(&mut dest, &mut d, 9);
        assert_eq!(d[11], b'2');

        // EAN-13: 590123412345 + check digit 7.
        let mut d = b"590123412345?".to_vec();
        hack.make_ean13(&mut dest, &mut d, 9);
        assert_eq!(d[12], b'7');

        // EAN-8: 9638507 + check digit 4.
        let mut d = b"9638507?".to_vec();
        hack.make_ean8(&mut dest, &mut d, 9);
        assert_eq!(d[7], b'4');

        // UPC-E 04252614 expands to UPC-A 042100005264.
        let e = expand_to_upc_a_digits(b"04252614").expect("a leading zero expands");
        assert_eq!(&e, b"042100005264");
    }

    /// A digit's three representations are related: the right-hand pattern is
    /// the inverse of the left-A one, and left-B is left-A mirrored and
    /// inverted. Upstream's own table says so, so the tables can check
    /// themselves.
    #[test]
    fn the_digit_patterns_are_the_three_forms_of_each_other() {
        for n in 0..10 {
            let a = UPC_LEFT_A[n];
            assert_eq!(UPC_RIGHT[n], !a & 0x7f, "right is left-A inverted");
            let mirrored: u32 = (0..7).map(|i| ((a >> i) & 1) << (6 - i)).sum();
            assert_eq!(UPC_LEFT_B[n], !mirrored & 0x7f, "left-B is left-A mirrored");
        }
    }
}
