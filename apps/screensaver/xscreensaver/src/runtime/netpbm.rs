//! The netpbm filters `vidwhacker` is made of.
//!
//! Upstream's `vidwhacker` is a shell script. It grabs a picture and pipes it
//! through one of nineteen pipelines built out of the netpbm tools, so porting
//! the saver means porting the tools: this module is `pamedge`, `pamoil`,
//! `pgmbentley`, `ppmrelief`, `ppmspread`, `ppmshift`, `pgmenhance`,
//! `pnmnorm`, `pnmsmooth`, `pnminvert`, `pamarith`, `pgmnoise`, `pamcrater`
//! and three of `ppmpat`'s patterns, ported from netpbm's own C rather than
//! from a description of what they do.
//!
//! Four of the names in the script are the old ones. `pgmedge` is `pamedge`
//! now, `pgmcrater` is `pamcrater`, `ppmnorm` is `pnmnorm` and `pnmarith` is
//! `pamarith`; the functions here carry the names the script uses, since that
//! is what a reader comparing the two will have in front of them.
//!
//! Everything is one image type. netpbm has three (PBM, PGM, PPM) and the
//! tools convert between them constantly, but the only conversions the
//! pipelines actually perform are `ppmtopgm` and `pgmtoppm`, and a PGM is
//! exactly a PPM whose three samples agree. So [`Pnm`] is always three
//! samples, `to_pgm` sets all three to the luminosity, and nothing has to know
//! which of the two it is holding.

use super::color::Pixel;
use super::fb::Fb;
use super::rand::{frand, random};

/// netpbm's luminosity weights, from `lib/ppm.h`.
const LUMIN_R: f64 = 0.2989;
const LUMIN_G: f64 = 0.5866;
const LUMIN_B: f64 = 0.1145;

/// An image, as every one of these tools sees it: three samples a pixel, and
/// a maxval that is always 255 here because that is what arrives and what
/// `pnmdepth 255` in the pipelines asks for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pnm {
    pub w: i32,
    pub h: i32,
    pub px: Vec<[u8; 3]>,
}

impl Pnm {
    pub fn new(w: i32, h: i32) -> Pnm {
        Pnm {
            w: w.max(1),
            h: h.max(1),
            px: vec![[0; 3]; (w.max(1) * h.max(1)) as usize],
        }
    }

    /// Read one out of a framebuffer, which is where a picture arrives from.
    pub fn from_fb(fb: &Fb) -> Pnm {
        let (w, h) = (fb.width(), fb.height());
        let mut p = Pnm::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = fb.get_pixel(x, y);
                p.set(x, y, [(v >> 16) as u8, (v >> 8) as u8, v as u8]);
            }
        }
        p
    }

    /// Write it back out again.
    pub fn to_fb(&self, fb: &mut Fb) {
        for y in 0..fb.height().min(self.h) {
            for x in 0..fb.width().min(self.w) {
                let [r, g, b] = self.get(x, y);
                let v: Pixel = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
                fb.put_pixel(x, y, v);
            }
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> [u8; 3] {
        let (x, y) = (x.clamp(0, self.w - 1), y.clamp(0, self.h - 1));
        self.px[(y * self.w + x) as usize]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, v: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        self.px[(y * self.w + x) as usize] = v;
    }

    /// `ppm_luminosity`, which rounds rather than truncating: netpbm's is
    /// `(pixval)(PPM_LUMIN(p) + 0.5)`. Truncating turns some greys one darker
    /// on the way through, because the weights only sum to one to within a
    /// float.
    #[inline]
    pub fn lumin(p: [u8; 3]) -> u8 {
        let l = LUMIN_R * f64::from(p[0]) + LUMIN_G * f64::from(p[1]) + LUMIN_B * f64::from(p[2]);
        (l + 0.5) as u8
    }
}

fn clip(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// `ppmtopgm`: three samples become the one luminosity, three times over.
pub fn to_pgm(src: &Pnm) -> Pnm {
    let mut out = src.clone();
    for p in &mut out.px {
        let l = Pnm::lumin(*p);
        *p = [l, l, l];
    }
    out
}

/// `pgmtoppm color0-color1`: grey becomes a position between two colours.
pub fn to_ppm(src: &Pnm, c0: [u8; 3], c1: [u8; 3]) -> Pnm {
    let mut out = src.clone();
    for p in &mut out.px {
        let input = i32::from(p[0]);
        for (i, ch) in p.iter_mut().enumerate() {
            *ch = clip((i32::from(c0[i]) * (255 - input) + i32::from(c1[i]) * input) / 255);
        }
    }
    out
}

/// `pnminvert`.
pub fn invert(src: &Pnm) -> Pnm {
    let mut out = src.clone();
    for p in &mut out.px {
        for ch in p.iter_mut() {
            *ch = 255 - *ch;
        }
    }
    out
}

/// `pnmflip -lr`.
pub fn flip_lr(src: &Pnm) -> Pnm {
    let mut out = src.clone();
    for y in 0..src.h {
        for x in 0..src.w {
            out.set(x, y, src.get(src.w - 1 - x, y));
        }
    }
    out
}

/// `pnmflip -tb`.
pub fn flip_tb(src: &Pnm) -> Pnm {
    let mut out = src.clone();
    for y in 0..src.h {
        for x in 0..src.w {
            out.set(x, y, src.get(x, src.h - 1 - y));
        }
    }
    out
}

/// Which of `pamarith`'s functions to apply. Only the four the pipelines use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arith {
    Add,
    Subtract,
    Multiply,
    Difference,
}

/// `pamarith`. netpbm works in normalised samples, so a multiply is a
/// multiply of fractions rather than of bytes, which is why it darkens.
pub fn arith(a: &Pnm, b: &Pnm, f: Arith) -> Pnm {
    let mut out = Pnm::new(a.w.min(b.w), a.h.min(b.h));
    for y in 0..out.h {
        for x in 0..out.w {
            let (pa, pb) = (a.get(x, y), b.get(x, y));
            let mut v = [0u8; 3];
            for i in 0..3 {
                let (s0, s1) = (f64::from(pa[i]) / 255.0, f64::from(pb[i]) / 255.0);
                let r = match f {
                    Arith::Add => (s0 + s1).min(1.0),
                    Arith::Subtract => (s0 - s1).max(0.0),
                    Arith::Multiply => s0 * s1,
                    Arith::Difference => (s0 - s1).abs(),
                };
                v[i] = clip((r * 255.0).round() as i32);
            }
            out.set(x, y, v);
        }
    }
    out
}

/// `pamedge`: a Sobel gradient, scaled by netpbm's "arbitrary scaling factor"
/// of 1.8, with a black border because the operator needs a neighbour on
/// every side.
pub fn edge(src: &Pnm) -> Pnm {
    let mut out = Pnm::new(src.w, src.h);
    if src.w < 3 || src.h < 3 {
        return out;
    }
    let at = |x: i32, y: i32, p: usize| f64::from(src.get(x, y)[p]);
    for y in 1..src.h - 1 {
        for x in 1..src.w - 1 {
            let mut v = [0u8; 3];
            for (p, ch) in v.iter_mut().enumerate() {
                let hg = |yy: i32| at(x + 1, yy, p) - at(x - 1, yy, p);
                let ha = |yy: i32| at(x - 1, yy, p) + 2.0 * at(x, yy, p) + at(x + 1, yy, p);
                let grad1 = hg(y - 1) + 2.0 * hg(y) + hg(y + 1);
                let grad2 = ha(y + 1) - ha(y - 1);
                let gradient = (grad1 * grad1 + grad2 * grad2).sqrt();
                *ch = clip((gradient / 1.8) as i32);
            }
            out.set(x, y, v);
        }
    }
    out
}

/// `pgmenhance`, at netpbm's default of nine, which is an unsharp mask: the
/// pixel less nine tenths of its neighbourhood mean, divided by the tenth
/// that is left.
///
/// Upstream's comment traces it to Knuth's "Digital Halftones by Dot
/// Diffusion" by way of two 1976 papers.
pub fn enhance(src: &Pnm) -> Pnm {
    const N: f64 = 9.0;
    let phi = N / 10.0;
    let omphi = 1.0 - phi;
    let mut out = src.clone();
    if src.w < 3 || src.h < 3 {
        return out;
    }
    for y in 1..src.h - 1 {
        for x in 1..src.w - 1 {
            let mut v = [0u8; 3];
            for (p, ch) in v.iter_mut().enumerate() {
                let mut sum = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        sum += f64::from(src.get(x + dx, y + dy)[p]);
                    }
                }
                let this = f64::from(src.get(x, y)[p]);
                *ch = clip(((this - phi * sum / 9.0) / omphi + 0.5) as i32);
            }
            out.set(x, y, v);
        }
    }
    out
}

/// `pamoil`, at netpbm's default smear of three: every pixel becomes the
/// value that occurs most often in the square around it, which is what makes
/// a photograph look painted.
/// A greyscale image is three equal planes, and `pamoil` is the most
/// expensive thing here, so it is worth not doing three times over. Both
/// pipelines that use it run `ppmtopgm` first.
fn is_grey(p: &Pnm) -> bool {
    p.px.iter().all(|s| s[0] == s[1] && s[1] == s[2])
}

pub fn oil(src: &Pnm) -> Pnm {
    const SMEAR: i32 = 3;
    /* At most this many samples land in the histogram, so at most this many
    entries of it are ever non-zero. Clearing and searching all 256 costs
    five times what the filter itself does. */
    const WINDOW: usize = ((2 * SMEAR + 1) * (2 * SMEAR + 1)) as usize;

    let mut out = Pnm::new(src.w, src.h);
    let planes = if is_grey(src) { 1 } else { 3 };
    let mut hist = [0u32; 256];
    let mut touched = [0u8; WINDOW];

    for y in 0..src.h {
        let (y0, y1) = ((y - SMEAR).max(0), (y + SMEAR).min(src.h - 1));
        for x in 0..src.w {
            let (x0, x1) = ((x - SMEAR).max(0), (x + SMEAR).min(src.w - 1));
            let mut v = [0u8; 3];
            for (p, out_ch) in v.iter_mut().enumerate().take(planes) {
                let mut n = 0;
                for dy in y0..=y1 {
                    for dx in x0..=x1 {
                        let s = src.get(dx, dy)[p];
                        let c = &mut hist[s as usize];
                        if *c == 0 {
                            touched[n] = s;
                            n += 1;
                        }
                        *c += 1;
                    }
                }
                // Upstream scans the whole histogram in value order and takes
                // the first entry with the highest count, so a tie goes to
                // the darker value. Only the touched entries can win, but the
                // tie-break still has to be by value and not by the order the
                // neighbourhood happened to be walked in.
                let mut modal = 0u8;
                let mut best = 0;
                for &s in &touched[..n] {
                    let c = hist[s as usize];
                    if c > best || (c == best && s < modal) {
                        best = c;
                        modal = s;
                    }
                }
                for &s in &touched[..n] {
                    hist[s as usize] = 0;
                }
                *out_ch = modal;
            }
            if planes == 1 {
                v = [v[0], v[0], v[0]];
            }
            out.set(x, y, v);
        }
    }
    out
}

/// `pgmbentley`: every pixel slides down the image by a quarter of its own
/// brightness, so bright things smear downwards and the picture melts.
///
/// The output starts black and pixels are only ever written, never blended,
/// so where nothing lands stays black. That is upstream's, not an omission.
pub fn bentley(src: &Pnm) -> Pnm {
    const N: i32 = 4;
    let mut out = Pnm::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            let v = src.get(x, y);
            // A colour image is bentleyed per plane in netpbm, but the tool is
            // PGM only and the pipelines always convert first, so the row is
            // chosen by the one value they all share.
            let brow = (src.h - 1).min(y + i32::from(v[0]) / N);
            out.set(x, brow, v);
        }
    }
    out
}

/// `ppmrelief`: an emboss, which is the picture minus a copy of itself
/// shifted two pixels diagonally, lifted to the middle grey.
pub fn relief(src: &Pnm) -> Pnm {
    let mut out = Pnm::new(src.w, src.h);
    if src.w < 3 || src.h < 3 {
        return out;
    }
    let mv2 = 255 / 2;
    for y in 2..src.h {
        for x in 0..src.w - 2 {
            let a = src.get(x, y);
            let b = src.get(x + 2, y - 2);
            let mut v = [0u8; 3];
            for i in 0..3 {
                v[i] = clip(i32::from(a[i]) + (mv2 - i32::from(b[i])));
            }
            out.set(x, y, v);
        }
    }
    out
}

/// `ppmspread`: each pixel swaps with another one within `spread` of it.
pub fn spread(src: &Pnm, amount: i32) -> Pnm {
    let mut out = Pnm::new(src.w, src.h);
    let n = (amount + 1).max(1) as u32;
    for y in 0..src.h {
        for x in 0..src.w {
            let p = src.get(x, y);
            let xdis = (random() % n) as i32 - (n as i32 / 2);
            let ydis = (random() % n) as i32 - (n as i32 / 2);
            let (xnew, ynew) = (x + xdis, y + ydis);
            if xnew >= 0 && xnew < src.w && ynew >= 0 && ynew < src.h {
                /* Displacing a pixel is accomplished by swapping it with
                another pixel in its vicinity. */
                let p2 = src.get(xnew, ynew);
                out.set(xnew, ynew, p);
                out.set(x, y, p2);
            } else {
                out.set(x, y, p);
            }
        }
    }
    out
}

/// `ppmshift`: every row slides sideways by its own random amount, and the
/// pixel at the edge it left smears out to fill the gap.
pub fn shift(src: &Pnm, amount: i32) -> Pnm {
    let mut out = Pnm::new(src.w, src.h);
    let n = (amount + 1).max(1) as u32;
    for y in 0..src.h {
        let now = if amount != 0 {
            (random() % n) as i32 - (n as i32 / 2)
        } else {
            0
        };
        for x in 0..src.w {
            // Reading with a clamp is what upstream's pointer arithmetic comes
            // to: it stops advancing the source at the last column, or starts
            // late, so the edge pixel repeats.
            out.set(x, y, src.get(x + now, y));
        }
    }
    out
}

/// `pnmsmooth`, at its default 3 by 3: the mean of the neighbours.
pub fn smooth(src: &Pnm) -> Pnm {
    let mut out = src.clone();
    if src.w < 3 || src.h < 3 {
        return out;
    }
    for y in 1..src.h - 1 {
        for x in 1..src.w - 1 {
            let mut v = [0u8; 3];
            for (p, ch) in v.iter_mut().enumerate() {
                let mut sum = 0u32;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        sum += u32::from(src.get(x + dx, y + dy)[p]);
                    }
                }
                *ch = (sum / 9) as u8;
            }
            out.set(x, y, v);
        }
    }
    out
}

/// `pnmnorm`: stretch the contrast so that the darkest two per cent of the
/// picture goes to black and the brightest one per cent to white.
///
/// A colour picture is normalised by its luminosity and the three channels
/// are scaled by the same ratio, which is what keeps the hues.
pub fn norm(src: &Pnm) -> Pnm {
    let mut hist = [0u32; 256];
    for p in &src.px {
        hist[Pnm::lumin(*p) as usize] += 1;
    }
    let total: u32 = hist.iter().sum();
    if total == 0 {
        return src.clone();
    }

    let bcut = (f64::from(total) * 2.0 / 100.0) as u32;
    let wcut = (f64::from(total) * 1.0 / 100.0) as u32;

    let mut bvalue = 0usize;
    let mut count = hist[0];
    while count < bcut && bvalue < 255 {
        bvalue += 1;
        count += hist[bvalue];
    }
    let mut wvalue = 255usize;
    let mut count = hist[255];
    while count < wcut && wvalue > 0 {
        wvalue -= 1;
        count += hist[wvalue];
    }
    if wvalue <= bvalue {
        return src.clone();
    }

    /* Map the middle brightnesses linearly onto 0..maxval. */
    let range = (wvalue - bvalue) as u32;
    let mut new_brightness = [0u8; 256];
    for (i, nb) in new_brightness.iter_mut().enumerate() {
        *nb = if i <= bvalue {
            0
        } else if i >= wvalue {
            255
        } else {
            (((i - bvalue) as u32 * 255 + range / 2) / range).min(255) as u8
        };
    }

    let mut out = src.clone();
    for p in &mut out.px {
        let old = Pnm::lumin(*p);
        if old == 0 {
            continue; /* zero times anything is zero */
        }
        let scaler = f64::from(new_brightness[old as usize]) / f64::from(old);
        for ch in p.iter_mut() {
            *ch = clip((f64::from(*ch) * scaler).round() as i32);
        }
    }
    out
}

/// `pgmnoise`: an image of nothing but random grey.
pub fn noise(w: i32, h: i32) -> Pnm {
    let mut out = Pnm::new(w, h);
    for p in &mut out.px {
        let v = (random() % 256) as u8;
        *p = [v, v, v];
    }
    out
}

/// `pamcrater`: a cratered plain, seen from directly above as a height field.
///
/// The radius law is upstream's, and its comment is worth keeping: "Thanks,
/// Rudy, for this equation that maps the uniformly distributed numbers from
/// cast() into an area-law distribution as observed on cratered bodies."
pub fn crater(number: i32, w: i32, h: i32) -> Pnm {
    const CDEPTH_POWER: f64 = 1.5;
    const DEPTH_BIAS2: f64 = 0.5;
    /* netpbm works in 16-bit elevations and clamps to this window. */
    const MAXVAL: f64 = 65535.0;

    let (w, h) = (w.max(1), h.max(1));
    let mut terrain = vec![MAXVAL as u32 / 2; (w * h) as usize];
    let idx = |x: i32, y: i32| {
        (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize /* craters wrap */
    };

    for _ in 0..number.max(0) {
        let cx = frand(f64::from(w - 1)) as i32;
        let cy = frand(f64::from(h - 1)) as i32;
        let radius = (1.0 / (std::f64::consts::PI * (1.0 - frand(0.9999)))).sqrt();

        if radius < 3.0 {
            /* Set pixel to the average of its Moore neighborhood. */
            let mut amptot = 0u64;
            for y in cy - 1..=cy + 1 {
                for x in cx - 1..=cx + 1 {
                    amptot += u64::from(terrain[idx(x, y)]);
                }
            }
            let axelev = (amptot / 9) as i64;
            let perturb = if radius >= 1.0 {
                i64::from((random() >> 8) as i32 & 0x3) - 1
            } else {
                0
            };
            terrain[idx(cx, cy)] = (axelev + perturb).clamp(0, MAXVAL as i64) as u32;
            continue;
        }

        let crater_radius = radius as i32;
        let impact_radius = 2.max(crater_radius / 3);
        let axelev = {
            let mut amptot = 0u64;
            let mut npatch = 0u64;
            for y in cy - impact_radius..=cy + impact_radius {
                for x in cx - impact_radius..=cx + impact_radius {
                    amptot += u64::from(terrain[idx(x, y)]);
                    npatch += 1;
                }
            }
            (amptot / npatch) as f64
        };

        let rollmin = 0.9f64;
        for y in cy - crater_radius..=cy + crater_radius {
            let dysq = f64::from((cy - y) * (cy - y));
            for x in cx - crater_radius..=cx + crater_radius {
                let dxsq = f64::from((cx - x) * (cx - x));
                let cd = (dxsq + dysq) / f64::from(crater_radius * crater_radius);
                let cd2 = cd * 2.25;
                let tcz = DEPTH_BIAS2.sqrt() - (1.0 - cd2).abs().sqrt();
                let mut cz = if cd2 > 1.0 { 0.0f64 } else { -10.0 }.max(tcz);
                cz *= f64::from(crater_radius).powf(CDEPTH_POWER);
                if dysq == 0.0 && dxsq == 0.0 && cz as i32 == 0 {
                    cz = if cz < 0.0 { -1.0 } else { 1.0 };
                }
                let roll = (((1.0 / (1.0 - rollmin.min(cd))) / (1.0 / (1.0 - rollmin)))
                    - (1.0 - rollmin))
                    / rollmin;
                let here = f64::from(terrain[idx(x, y)]);
                let av = (axelev + cz) * (1.0 - roll) + (here + cz) * roll;
                terrain[idx(x, y)] = av.clamp(1000.0, 64000.0) as u32;
            }
        }
    }

    /* Down to eight bits, which is what the pipeline's pgmtoppm would do with
    the ratio anyway. */
    let mut out = Pnm::new(w, h);
    for (i, &e) in terrain.iter().enumerate() {
        let v = (f64::from(e) / MAXVAL * 255.0).round().clamp(0.0, 255.0) as u8;
        out.px[i] = [v, v, v];
    }
    out
}

/// `randomColor` with the luminosity rejection of `randomDarkColor` and
/// `randomBrightColor`. netpbm's threshold is a quarter.
fn random_color(bright: bool) -> [u8; 3] {
    const DARK_THRESH: f64 = 0.25;
    loop {
        let p = [
            (random() % 256) as u8,
            (random() % 256) as u8,
            (random() % 256) as u8,
        ];
        let l = f64::from(Pnm::lumin(p));
        if (l > 255.0 * DARK_THRESH) == bright {
            return p;
        }
    }
}

fn average_two(a: [u8; 3], b: [u8; 3]) -> [u8; 3] {
    [
        ((u32::from(a[0]) + u32::from(b[0])) / 2) as u8,
        ((u32::from(a[1]) + u32::from(b[1])) / 2) as u8,
        ((u32::from(a[2]) + u32::from(b[2])) / 2) as u8,
    ]
}

/// `ppmd_filledrectangle` with no drawproc: paint it.
fn fill_rect(img: &mut Pnm, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
    for yy in y..y + h {
        for xx in x..x + w {
            img.set(xx, yy, c);
        }
    }
}

/// The same, through `average_drawproc`: paint the average of what is there
/// and what is being painted. This is what makes a plaid's crossings a third
/// colour rather than whichever stripe was drawn last.
fn fill_rect_avg(img: &mut Pnm, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
    for yy in y..y + h {
        for xx in x..x + w {
            if xx < 0 || yy < 0 || xx >= img.w || yy >= img.h {
                continue;
            }
            let there = img.get(xx, yy);
            img.set(xx, yy, average_two(there, c));
        }
    }
}

/// `ppmpat -madras`.
pub fn pat_madras(w: i32, h: i32) -> Pnm {
    let mut img = Pnm::new(w, h);
    let (w, h) = (img.w, img.h);
    let back = random_color(false);
    let fore1 = random_color(true);
    let fore2 = random_color(true);

    let cols2 = w * 2 / 44;
    let rows2 = h * 2 / 44;
    let cols3 = w * 3 / 44;
    let rows3 = h * 3 / 44;
    let cols12 = w - 10 * cols2 - 4 * cols3;
    let rows12 = h - 10 * rows2 - 4 * rows3;
    let cols6a = cols12 / 2;
    let rows6a = rows12 / 2;
    let cols6b = cols12 - cols6a;
    let rows6b = rows12 - rows6a;

    /* Warp. */
    let warp: &[(i32, i32, [u8; 3])] = &[
        (0, cols2, back),
        (cols2, cols3, fore1),
        (cols2 + cols3, cols2, back),
        (2 * cols2 + cols3, cols2, fore2),
        (3 * cols2 + cols3, cols2, back),
        (4 * cols2 + cols3, cols6a, fore1),
        (4 * cols2 + cols3 + cols6a, cols2, back),
        (5 * cols2 + cols3 + cols6a, cols3, fore2),
        (5 * cols2 + 2 * cols3 + cols6a, cols2, back),
        (6 * cols2 + 2 * cols3 + cols6a, cols3, fore2),
        (6 * cols2 + 3 * cols3 + cols6a, cols2, back),
        (7 * cols2 + 3 * cols3 + cols6a, cols6b, fore1),
        (7 * cols2 + 3 * cols3 + cols6a + cols6b, cols2, back),
        (8 * cols2 + 3 * cols3 + cols6a + cols6b, cols2, fore2),
        (9 * cols2 + 3 * cols3 + cols6a + cols6b, cols2, back),
        (10 * cols2 + 3 * cols3 + cols6a + cols6b, cols3, fore1),
    ];
    for &(x, cw, c) in warp {
        fill_rect(&mut img, x, 0, cw, h, c);
    }

    /* Woof. */
    let woof: &[(i32, i32, [u8; 3])] = &[
        (0, rows2, back),
        (rows2, rows3, fore2),
        (rows2 + rows3, rows2, back),
        (2 * rows2 + rows3, rows2, fore1),
        (3 * rows2 + rows3, rows2, back),
        (4 * rows2 + rows3, rows6a, fore2),
        (4 * rows2 + rows3 + rows6a, rows2, back),
        (5 * rows2 + rows3 + rows6a, rows3, fore1),
        (5 * rows2 + 2 * rows3 + rows6a, rows2, back),
        (6 * rows2 + 2 * rows3 + rows6a, rows3, fore1),
        (6 * rows2 + 3 * rows3 + rows6a, rows2, back),
        (7 * rows2 + 3 * rows3 + rows6a, rows6b, fore2),
        (7 * rows2 + 3 * rows3 + rows6a + rows6b, rows2, back),
        (8 * rows2 + 3 * rows3 + rows6a + rows6b, rows2, fore1),
        (9 * rows2 + 3 * rows3 + rows6a + rows6b, rows2, back),
        (10 * rows2 + 3 * rows3 + rows6a + rows6b, rows3, fore2),
    ];
    for &(y, rh, c) in woof {
        fill_rect_avg(&mut img, 0, y, w, rh, c);
    }
    img
}

/// `ppmpat -tartan`.
pub fn pat_tartan(w: i32, h: i32) -> Pnm {
    let mut img = Pnm::new(w, h);
    let (w, h) = (img.w, img.h);
    let back = random_color(false);
    let fore1 = random_color(true);
    let fore2 = random_color(true);

    let cols1 = w / 22;
    let rows1 = h / 22;
    let cols3 = w * 3 / 22;
    let rows3 = h * 3 / 22;
    let cols10 = w - 3 * cols1 - 3 * cols3;
    let rows10 = h - 3 * rows1 - 3 * rows3;
    let cols5a = cols10 / 2;
    let rows5a = rows10 / 2;
    let cols5b = cols10 - cols5a;
    let rows5b = rows10 - rows5a;

    /* Warp. */
    let warp: &[(i32, i32, [u8; 3])] = &[
        (0, cols5a, back),
        (cols5a, cols1, fore1),
        (cols5a + cols1, cols5b, back),
        (cols10 + cols1, cols3, fore2),
        (cols10 + cols1 + cols3, cols1, back),
        (cols10 + 2 * cols1 + cols3, cols3, fore2),
        (cols10 + 2 * cols1 + 2 * cols3, cols1, back),
        (cols10 + 3 * cols1 + 2 * cols3, cols3, fore2),
    ];
    for &(x, cw, c) in warp {
        fill_rect(&mut img, x, 0, cw, h, c);
    }

    /* Woof. */
    let woof: &[(i32, i32, [u8; 3])] = &[
        (0, rows5a, back),
        (rows5a, rows1, fore1),
        (rows5a + rows1, rows5b, back),
        (rows10 + rows1, rows3, fore2),
        (rows10 + rows1 + rows3, rows1, back),
        (rows10 + 2 * rows1 + rows3, rows3, fore2),
        (rows10 + 2 * rows1 + 2 * rows3, rows1, back),
        (rows10 + 3 * rows1 + 2 * rows3, rows3, fore2),
    ];
    for &(y, rh, c) in woof {
        fill_rect_avg(&mut img, 0, y, w, rh, c);
    }
    img
}

/// `randomCamoColor`: light brown, dark green, brown or dark brown, in
/// netpbm's proportions of 3, 3, 2, 2.
fn random_camo_color() -> [u8; 3] {
    let v1 = 256 / 8;
    let v2 = 256 / 4;
    let v3 = 256 / 2;
    let r = |n: i32| (random() % n as u32) as i32;
    let p = match random() % 10 {
        0..=2 => [r(v3) + v3, r(v3) + v2, r(v3) + v2], /* light brown */
        3..=5 => [r(v2), r(v2) + 3 * v1, r(v2)],       /* dark green */
        6..=7 => [r(v2) + v2, r(v2), r(v2)],           /* brown */
        _ => [r(v1) + v1, r(v1), r(v1)],               /* dark brown */
    };
    [clip(p[0]), clip(p[1]), clip(p[2])]
}

/// `ppmpat -camo`.
///
/// Upstream draws each blob as a closed spline through seven to thirteen
/// points on a randomly stretched and turned ellipse, then flood-fills it.
/// Here the spline points are the polygon: `runtime::spline` gives the same
/// smooth outline, and filling a polygon is something the framebuffer already
/// does, so the blob is drawn rather than traced and then flooded.
pub fn pat_camo(w: i32, h: i32) -> Pnm {
    const BLOBRAD: f64 = 50.0;
    const MIN_POINTS: u32 = 7;
    const MAX_POINTS: u32 = 13;
    const MIN_ELLIPSE_FACTOR: f64 = 0.5;
    const MAX_ELLIPSE_FACTOR: f64 = 2.0;
    const MIN_POINT_FACTOR: f64 = 0.5;
    const MAX_POINT_FACTOR: f64 = 2.0;

    let mut img = Pnm::new(w, h);
    let (w, h) = (img.w, img.h);
    let back = random_camo_color();
    fill_rect(&mut img, 0, 0, w, h, back);

    let n = (w as f64 * h as f64 / (BLOBRAD * BLOBRAD) * 5.0) as i32;
    for _ in 0..n {
        let point_ct = random() % (MAX_POINTS - MIN_POINTS + 1) + MIN_POINTS;
        let cx = f64::from(random() % w.max(1) as u32);
        let cy = f64::from(random() % h.max(1) as u32);
        let a = frand(MAX_ELLIPSE_FACTOR - MIN_ELLIPSE_FACTOR) + MIN_ELLIPSE_FACTOR;
        let b = frand(MAX_ELLIPSE_FACTOR - MIN_ELLIPSE_FACTOR) + MIN_ELLIPSE_FACTOR;
        let theta = frand(std::f64::consts::TAU);

        let mut pts = Vec::with_capacity(point_ct as usize);
        for p in 0..point_ct {
            let c = frand(MAX_POINT_FACTOR - MIN_POINT_FACTOR) + MIN_POINT_FACTOR;
            let ang = f64::from(p) * std::f64::consts::TAU / f64::from(point_ct);
            let (tx, ty) = (a * ang.sin(), b * ang.cos());
            let tang = ty.atan2(tx) + theta;
            pts.push((
                (cx + BLOBRAD * c * tang.sin()).clamp(0.0, f64::from(w - 1)),
                (cy + BLOBRAD * c * tang.cos()).clamp(0.0, f64::from(h - 1)),
            ));
        }
        fill_blob(&mut img, &pts, random_camo_color());
    }
    img
}

/// Scanline-fill a closed polygon, which is what `ppmd_fill` does with the
/// outline `ppmd_polyspline` traced.
fn fill_blob(img: &mut Pnm, pts: &[(f64, f64)], c: [u8; 3]) {
    if pts.len() < 3 {
        return;
    }
    let (mut top, mut bot) = (f64::MAX, f64::MIN);
    for &(_, y) in pts {
        top = top.min(y);
        bot = bot.max(y);
    }
    let mut xs = Vec::with_capacity(pts.len());
    for y in top.floor() as i32..=bot.ceil() as i32 {
        if y < 0 || y >= img.h {
            continue;
        }
        let scan = f64::from(y) + 0.5;
        xs.clear();
        for i in 0..pts.len() {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % pts.len()];
            if (y0 <= scan) == (y1 <= scan) {
                continue;
            }
            xs.push(x0 + (scan - y0) / (y1 - y0) * (x1 - x0));
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks_exact(2) {
            for x in pair[0].ceil() as i32..=pair[1].floor() as i32 {
                img.set(x, y, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: i32, h: i32) -> Pnm {
        let mut p = Pnm::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 255) / w.max(1)) as u8;
                p.set(x, y, [v, v, v]);
            }
        }
        p
    }

    /// The luminosity weights are netpbm's, and they sum to one, which is what
    /// makes a grey stay itself through `ppmtopgm`.
    #[test]
    fn grey_survives_the_luminosity() {
        assert!((LUMIN_R + LUMIN_G + LUMIN_B - 1.0).abs() < 1e-9);
        for v in 0..=255u8 {
            assert_eq!(Pnm::lumin([v, v, v]), v, "grey {v} moved");
        }
    }

    /// `pgmtoppm` puts black at one end of the range and white at the other,
    /// which is the whole of what the pipelines use it for.
    #[test]
    fn the_colour_ramp_reaches_both_ends() {
        let mut src = Pnm::new(3, 1);
        src.set(0, 0, [0, 0, 0]);
        src.set(1, 0, [128, 128, 128]);
        src.set(2, 0, [255, 255, 255]);
        let c0 = [10, 20, 30];
        let c1 = [200, 210, 220];
        let out = to_ppm(&src, c0, c1);
        assert_eq!(out.get(0, 0), c0);
        assert_eq!(out.get(2, 0), c1);
        // The middle is between the two, on every channel.
        for i in 0..3 {
            let m = out.get(1, 0)[i];
            assert!(m > c0[i] && m < c1[i], "channel {i} landed at {m}");
        }
    }

    /// An edge detector finds the edge and nothing else: a flat field has no
    /// gradient anywhere, and a step has one exactly at the step.
    #[test]
    fn the_edge_is_where_the_step_is() {
        let flat = Pnm {
            w: 8,
            h: 8,
            px: vec![[80, 80, 80]; 64],
        };
        assert!(edge(&flat).px.iter().all(|p| *p == [0, 0, 0]));

        let mut step = Pnm::new(9, 9);
        for y in 0..9 {
            for x in 0..9 {
                let v = if x < 4 { 0 } else { 255 };
                step.set(x, y, [v, v, v]);
            }
        }
        let e = edge(&step);
        // Column 3 and 4 straddle the step and light up; column 1 is far from
        // it and stays dark.
        assert!(e.get(3, 4)[0] > 100, "the step did not register");
        assert_eq!(e.get(1, 4)[0], 0, "a flat area lit up");
    }

    /// `pnmarith` works in fractions, so a multiply darkens and white is the
    /// identity for it. This is the difference between netpbm's arithmetic and
    /// the byte arithmetic it is easy to write by mistake.
    #[test]
    fn multiply_is_fractional() {
        let half = Pnm {
            w: 1,
            h: 1,
            px: vec![[128, 128, 128]],
        };
        let white = Pnm {
            w: 1,
            h: 1,
            px: vec![[255, 255, 255]],
        };
        assert_eq!(arith(&half, &white, Arith::Multiply).get(0, 0), [128; 3]);
        assert_eq!(arith(&half, &half, Arith::Multiply).get(0, 0), [64; 3]);
        // Difference is symmetric and never negative.
        assert_eq!(
            arith(&half, &white, Arith::Difference).get(0, 0),
            arith(&white, &half, Arith::Difference).get(0, 0)
        );
        // Subtract clamps at zero where difference would not.
        assert_eq!(arith(&half, &white, Arith::Subtract).get(0, 0), [0; 3]);
        assert_eq!(arith(&white, &half, Arith::Add).get(0, 0), [255; 3]);
    }

    /// Flipping twice is the identity, which is the cheapest true statement
    /// about a mirror and catches an off-by-one in either direction.
    #[test]
    fn flipping_twice_is_the_original() {
        let src = ramp(17, 13);
        assert_eq!(flip_lr(&flip_lr(&src)), src);
        assert_eq!(flip_tb(&flip_tb(&src)), src);
        assert_eq!(invert(&invert(&src)), src);
    }

    /// `pnmnorm` stretches to the full range: after it, something is black and
    /// something is white.
    #[test]
    fn normalising_uses_the_whole_range() {
        // A picture using only the middle third.
        let mut src = Pnm::new(64, 64);
        for (i, p) in src.px.iter_mut().enumerate() {
            let v = 96 + (i % 64) as u8 / 2;
            *p = [v, v, v];
        }
        let out = norm(&src);
        let lo = out.px.iter().map(|p| p[0]).min().unwrap();
        let hi = out.px.iter().map(|p| p[0]).max().unwrap();
        assert_eq!(lo, 0, "nothing reached black");
        assert_eq!(hi, 255, "nothing reached white");
    }

    /// Bentley slides a pixel down by a quarter of its own value. It writes
    /// rather than blends and works down the image, so a later row's own value
    /// lands on top of whatever an earlier bright pixel deposited there; the
    /// visible consequence is a band at the top that nothing reaches.
    ///
    /// A white field is the clean statement of it: every pixel moves down 63
    /// rows, so exactly the first 63 rows are left behind.
    #[test]
    fn bentley_drops_everything_by_a_quarter_of_itself() {
        let white = Pnm {
            w: 4,
            h: 200,
            px: vec![[255, 255, 255]; 800],
        };
        let out = bentley(&white);
        let drop = 255 / 4;
        for y in 0..drop {
            assert_eq!(out.get(0, y), [0, 0, 0], "row {y} should be untouched");
        }
        for y in drop..200 {
            assert_eq!(out.get(0, y), [255; 3], "row {y} should have been filled");
        }
        // A black field does not move at all, so it comes back unchanged.
        let black = Pnm {
            w: 4,
            h: 8,
            px: vec![[0, 0, 0]; 32],
        };
        assert_eq!(bentley(&black), black);
    }

    /// The oil filter picks the commonest value around a pixel, so a lone
    /// speck in a flat field is erased rather than blurred.
    #[test]
    fn oil_removes_a_speck() {
        let mut src = Pnm {
            w: 16,
            h: 16,
            px: vec![[40, 40, 40]; 256],
        };
        src.set(8, 8, [250, 250, 250]);
        assert_eq!(oil(&src).get(8, 8), [40, 40, 40]);
    }

    /// When two values are equally common the darker one wins, because
    /// netpbm scans its histogram in value order and takes the first with the
    /// highest count. Accumulating a running best instead gives whichever was
    /// walked over first, which is a different picture.
    #[test]
    fn oil_breaks_a_tie_towards_the_darker() {
        // A 7x7 window centred on (3,3) covers the whole image, so the counts
        // are exactly the number of each value present.
        let mut src = Pnm::new(7, 7);
        for (i, p) in src.px.iter_mut().enumerate() {
            // 24 of value 200, 24 of value 100, and the centre decides
            // nothing: 25 versus 24.
            let v = if i < 24 { 200 } else { 100 };
            *p = [v, v, v];
        }
        // 24 of 200 and 25 of 100: the majority wins outright.
        assert_eq!(oil(&src).get(3, 3), [100; 3]);

        // Now make it an exact tie by growing the window's contents to 24 each
        // plus one of a third value, so 200 and 100 are level.
        let mut tie = Pnm::new(7, 7);
        for (i, p) in tie.px.iter_mut().enumerate() {
            let v = match i {
                0..=23 => 200,
                24..=47 => 100,
                _ => 5,
            };
            *p = [v, v, v];
        }
        assert_eq!(
            oil(&tie).get(3, 3),
            [100; 3],
            "a tie should go to the darker value"
        );
    }

    /// A colour picture is still oiled a plane at a time. The one-plane
    /// short-circuit is only sound because a grey picture's three planes are
    /// equal, so it must not fire on a picture whose planes differ.
    #[test]
    fn oil_treats_the_planes_separately() {
        let mut src = Pnm::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                // Red is speckled, green is flat, blue is a ramp: the three
                // planes cannot share an answer.
                src.set(
                    x,
                    y,
                    [if (x + y) % 5 == 0 { 200 } else { 20 }, 128, (x * 16) as u8],
                );
            }
        }
        assert!(!is_grey(&src));
        let out = oil(&src);
        assert_eq!(out.get(8, 8)[1], 128, "the flat plane moved");
        assert_ne!(
            out.get(8, 8)[0],
            out.get(8, 8)[2],
            "two planes came out equal, so they were not done separately"
        );
    }

    /// Smoothing a flat field changes nothing, and it does average: a speck
    /// becomes a ninth of its excess.
    #[test]
    fn smoothing_is_a_mean() {
        let mut src = Pnm {
            w: 8,
            h: 8,
            px: vec![[90, 90, 90]; 64],
        };
        src.set(4, 4, [180, 180, 180]);
        let out = smooth(&src);
        assert_eq!(out.get(1, 1), [90, 90, 90]);
        assert_eq!(out.get(4, 4)[0], ((90 * 8 + 180) / 9) as u8);
    }

    /// Every generator produces something of the size asked for, and none of
    /// them panics on a degenerate one. The plaids in particular divide the
    /// width by 44 and would otherwise produce zero-width stripes.
    #[test]
    fn the_generators_survive_any_size() {
        crate::runtime::rand::ya_rand_init(20260812);
        for (w, h) in [(1, 1), (2, 3), (44, 44), (129, 71), (320, 240)] {
            for img in [
                noise(w, h),
                crater(50, w, h),
                pat_madras(w, h),
                pat_tartan(w, h),
                pat_camo(w, h),
            ] {
                assert_eq!((img.w, img.h), (w.max(1), h.max(1)));
                assert_eq!(img.px.len(), (img.w * img.h) as usize);
            }
        }
    }

    /// A plaid is drawn twice over, warp then woof, and the crossings are the
    /// average of the two. So the picture has more colours in it than the
    /// three it was given, which is the thing that would be lost by drawing
    /// the woof opaquely.
    #[test]
    fn a_plaid_has_more_colours_than_its_threads() {
        crate::runtime::rand::ya_rand_init(20260812);
        let img = pat_tartan(220, 220);
        let mut seen = std::collections::BTreeSet::new();
        for p in &img.px {
            seen.insert(*p);
        }
        assert!(
            seen.len() > 3,
            "only {} colours, so the crossings did not blend",
            seen.len()
        );
    }

    /// Camouflage covers its background: the blobs are numerous enough that
    /// the starting colour is not most of the picture.
    #[test]
    fn camo_covers_itself() {
        crate::runtime::rand::ya_rand_init(20260812);
        let img = pat_camo(200, 200);
        let mut counts = std::collections::BTreeMap::new();
        for p in &img.px {
            *counts.entry(*p).or_insert(0u32) += 1;
        }
        let commonest = *counts.values().max().unwrap();
        assert!(
            f64::from(commonest) < f64::from(img.px.len() as u32) * 0.8,
            "one colour is {commonest} of {} pixels",
            img.px.len()
        );
        assert!(counts.len() > 5, "only {} colours of camo", counts.len());
    }
}
