//! The country outlines `worldpieces` cuts up.
//!
//! Upstream carries them as `countries.c`, 3.5 MB of C source: 306 countries,
//! 1,724 polygons, 108,966 points of latitude and longitude. Most of that
//! bulk is the source itself. The same numbers as little-endian `f32` pairs
//! are 890 KB, so they are converted once and `include_bytes!` here, which
//! also keeps a file of a hundred thousand float literals out of the compiler.
//!
//! A country is a MultiPolygon: a list of polygons, each a list of closed
//! rings, the first being the outline and any others being holes cut out of
//! it. Only ten of the 1,724 have a hole at all, and they are the enclaves:
//! Lesotho inside South Africa, San Marino and the Vatican inside Italy.
//!
//! The layout is length-prefixed throughout:
//!
//! ```text
//! u32  country count
//!   u8[2]  ISO code          u16 name length   name bytes
//!   u16  polygon count
//!     u16  ring count
//!       u32  point count     f32[2] * count    (longitude, latitude)
//! ```

/// The blob, converted from upstream's `countries.c`.
const DATA: &[u8] = include_bytes!("../../images/countries.bin");

/// One closed ring of longitude and latitude, in degrees.
pub type Ring = Vec<[f64; 2]>;

/// One polygon: an outline and the holes in it.
pub struct Polygon {
    pub outer: Ring,
    pub holes: Vec<Ring>,
}

/// One country, which may be many islands.
pub struct Country {
    pub code: String,
    pub name: String,
    pub polygons: Vec<Polygon>,
}

struct Reader<'a> {
    at: usize,
    data: &'a [u8],
}

impl Reader<'_> {
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_le_bytes(self.data.get(self.at..self.at + 2)?.try_into().ok()?);
        self.at += 2;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.data.get(self.at..self.at + 4)?.try_into().ok()?);
        self.at += 4;
        Some(v)
    }
    fn f32(&mut self) -> Option<f32> {
        let v = f32::from_le_bytes(self.data.get(self.at..self.at + 4)?.try_into().ok()?);
        self.at += 4;
        Some(v)
    }
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let v = self.data.get(self.at..self.at + n)?;
        self.at += n;
        Some(v)
    }
}

/// Read the whole world.
///
/// A truncated or malformed blob yields what it managed rather than panicking:
/// it is compiled in, so a bad one is a broken build, but a saver that draws
/// half a world is better than one that takes the page down.
pub fn load() -> Vec<Country> {
    let mut r = Reader { at: 0, data: DATA };
    let Some(ncountries) = r.u32() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(ncountries as usize);
    for _ in 0..ncountries {
        let Some(code) = r.bytes(2).map(|b| String::from_utf8_lossy(b).into_owned()) else {
            break;
        };
        let Some(namelen) = r.u16() else { break };
        let Some(name) = r
            .bytes(namelen as usize)
            .map(|b| String::from_utf8_lossy(b).into_owned())
        else {
            break;
        };
        let Some(npolys) = r.u16() else { break };
        let mut polygons = Vec::with_capacity(npolys as usize);
        for _ in 0..npolys {
            let Some(nrings) = r.u16() else { break };
            let mut rings: Vec<Ring> = Vec::with_capacity(nrings as usize);
            for _ in 0..nrings {
                let Some(npoints) = r.u32() else { break };
                let mut ring = Vec::with_capacity(npoints as usize);
                for _ in 0..npoints {
                    let (Some(x), Some(y)) = (r.f32(), r.f32()) else {
                        break;
                    };
                    ring.push([f64::from(x), f64::from(y)]);
                }
                rings.push(ring);
            }
            if rings.is_empty() {
                continue;
            }
            let outer = rings.remove(0);
            polygons.push(Polygon {
                outer,
                holes: rings,
            });
        }
        out.push(Country {
            code,
            name,
            polygons,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blob is what the conversion said it was, and the countries in it
    /// are the ones anyone can check by eye.
    #[test]
    fn the_world_loads() {
        let world = load();
        assert_eq!(world.len(), 306, "countries");
        let polys: usize = world.iter().map(|c| c.polygons.len()).sum();
        assert_eq!(polys, 1724, "polygons");
        let points: usize = world
            .iter()
            .flat_map(|c| &c.polygons)
            .map(|p| p.outer.len() + p.holes.iter().map(Vec::len).sum::<usize>())
            .sum();
        assert_eq!(points, 108_966, "points");

        let zw = world.iter().find(|c| c.code == "ZW").expect("Zimbabwe");
        assert_eq!(zw.name, "Republic of Zimbabwe");

        // Ten polygons have a hole, and they are the enclaves.
        let holed = world
            .iter()
            .flat_map(|c| &c.polygons)
            .filter(|p| !p.holes.is_empty())
            .count();
        assert_eq!(holed, 10);
    }

    /// The polygons that actually exercise the hard path: every one with a
    /// hole in it, which is where a triangulator with holes goes wrong.
    ///
    /// The full sweep over all 1,724 is `measure_the_world_triangulates`,
    /// which is kept as a measurement because the flip pass is quadratic in
    /// the triangles of a polygon and the whole world takes a while unless it
    /// is optimised. It reports 1,724 exact and none wrong. This is the ten
    /// that would break first.
    #[test]
    fn every_country_with_a_hole_triangulates_exactly() {
        use crate::runtime::cdt;
        let mut checked = 0;
        for c in load() {
            for p in c.polygons.iter().filter(|p| !p.holes.is_empty()) {
                let want = cdt::polygon_area(&p.outer, &p.holes);
                let mesh = cdt::triangulate(&p.outer, &p.holes);
                let got: f64 = mesh
                    .triangles
                    .iter()
                    .map(|t| {
                        let (a, b, cc) = (mesh.points[t[0]], mesh.points[t[1]], mesh.points[t[2]]);
                        ((b[0] - a[0]) * (cc[1] - a[1]) - (b[1] - a[1]) * (cc[0] - a[0])).abs()
                            / 2.0
                    })
                    .sum();
                assert!(
                    (got - want).abs() < want * 1e-6 + 1e-12,
                    "{} ({} holes): mesh covers {got}, polygon is {want}",
                    c.code,
                    p.holes.len()
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 10, "the enclaves");
    }

    /// The triangulator handles the actual world, which is the only question
    /// that matters about it: the adversarial random tests in
    /// `runtime::cdt` are harder than anything here, and what counts is
    /// whether every real polygon comes out covering itself exactly.
    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn measure_the_world_triangulates() {
        use crate::runtime::cdt;
        let (mut ok, mut bad, mut tris) = (0, Vec::new(), 0usize);
        for c in load() {
            for (i, p) in c.polygons.iter().enumerate() {
                if p.outer.len() < 3 {
                    continue;
                }
                let want = cdt::polygon_area(&p.outer, &p.holes);
                let mesh = cdt::triangulate(&p.outer, &p.holes);
                tris += mesh.triangles.len();
                let got: f64 = mesh
                    .triangles
                    .iter()
                    .map(|t| {
                        let (a, b, cc) = (mesh.points[t[0]], mesh.points[t[1]], mesh.points[t[2]]);
                        ((b[0] - a[0]) * (cc[1] - a[1]) - (b[1] - a[1]) * (cc[0] - a[0])).abs()
                            / 2.0
                    })
                    .sum();
                if (got - want).abs() < want * 1e-6 + 1e-12 {
                    ok += 1;
                } else {
                    bad.push((c.code.clone(), i, p.holes.len(), got, want));
                }
            }
        }
        println!("exact: {ok}, wrong: {}, triangles: {tris}", bad.len());
        for (code, i, holes, got, want) in bad.iter().take(12) {
            println!("  {code} polygon {i} ({holes} holes): {got:.4} vs {want:.4}");
        }
    }

    /// Every point is a real place: longitude within half a turn, latitude
    /// within a quarter. A coordinate pair read the wrong way round would
    /// show up here immediately.
    #[test]
    fn every_point_is_on_the_earth() {
        for c in load() {
            for p in &c.polygons {
                for ring in std::iter::once(&p.outer).chain(p.holes.iter()) {
                    for &[lon, lat] in ring {
                        assert!(
                            (-180.0..=180.0).contains(&lon),
                            "{}: longitude {lon}",
                            c.code
                        );
                        assert!((-90.0..=90.0).contains(&lat), "{}: latitude {lat}", c.code);
                    }
                }
            }
        }
    }
}
