/* mapscroller, Copyright © 2021-2026 Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/glx/mapscroller.c`.
//!
//! A slowly scrolling map of a random place on Earth, drifting in a direction
//! that turns every so often, with the nearest city named in the corner.
//!
//! Upstream forks a perl helper to do the network, and says why in a comment:
//! "Network access and management of the image file cache happens in the
//! mapscroller.pl helper program, since doing https from C code is untenable.
//! Sadly, this division of labor means that this program won't work on iOS or
//! Android." A browser is the one place where that sentence is false, and
//! openstreetmap.org's tiles send `access-control-allow-origin: *`, so the
//! whole helper reduces to `runtime::tiles`: the saver asks for a URL, the
//! host fetches it. There is no cache to manage either, because the browser
//! already has one.
//!
//! The one thing done differently is how the sea is recognised. Upstream keeps
//! `oceantiles_12.png` as an image and calls `XGetPixel` on it: one two-bit
//! pixel for each of the 4096 by 4096 level-12 tiles, blue for open sea. That
//! image expands to 67 MB of framebuffer here to answer a yes-or-no question,
//! which is eight times the largest picture this port decodes for anything
//! else, so `runtime::png::decode_mask` stops at the defiltered scanlines and
//! returns two megabytes of bits instead.
//!
//! It is worth knowing why the map is needed at all rather than approximated.
//! The obvious shortcut is to call a position "sea" when the nearest city is
//! far away, since the cities table is already here for the caption. Measured
//! against that table, land reaches 1,273 km from the nearest city (central
//! Australia) and sea comes as close as 1,072 km (the Coral Sea), so the two
//! ranges overlap and no threshold separates them.

use crate::runtime::gl::{Blend, Shape};
use crate::runtime::texfont::TexFont;
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::{About, Ease, Gl, Hack3d, Opt, Runner3d, Saver3d, SaverDef, StartArgs};
#[cfg(target_arch = "wasm32")]
use crate::runtime::{About, Ease, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs};
use crate::runtime::{SelectItem, XEvent, ease, frand, random};

use super::mapcities::CITIES;

/// The last few entries of the table are not cities: Null Island, Point Nemo,
/// the poles and the Antarctic bases. They can be named as the nearest place
/// but are never somewhere to start.
const NONCITIES_COUNT: usize = 15;

/// A tile is 256 pixels square, as every slippy map's is.
const TILE_PIXEL_SIZE: f64 = 256.0;

/// The level-12 sea map: one pixel per tile, blue where there is no coastline
/// and no land.
const OCEAN_TILES: &[u8] = include_bytes!("../../images/oceantiles_12.png");
const OCEAN_LEVEL: i32 = 12;

/// A point on the Earth.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ll {
    lat: f64,
    lon: f64,
}

/// `lon2tilex`.
fn lon2tilex(lon: f64, z: i32) -> i32 {
    ((lon + 180.0) / 360.0 * f64::from(1 << z)).floor() as i32
}

/// `lat2tiley`.
fn lat2tiley(lat: f64, z: i32) -> i32 {
    let latrad = lat.to_radians();
    ((1.0 - latrad.tan().asinh() / std::f64::consts::PI) / 2.0 * f64::from(1 << z)).floor() as i32
}

/// `tilex2lon`.
fn tilex2lon(x: i32, z: i32) -> f64 {
    f64::from(x) / f64::from(1 << z) * 360.0 - 180.0
}

/// `tiley2lat`.
fn tiley2lat(y: i32, z: i32) -> f64 {
    let n = std::f64::consts::PI - 2.0 * std::f64::consts::PI * f64::from(y) / f64::from(1 << z);
    (180.0 / std::f64::consts::PI) * (0.5 * (n.exp() - (-n).exp())).atan()
}

/// `constrain_mercator`. True if it had to move the point.
///
/// Upstream: "The map tiles use the Mercator projection with an aspect ratio
/// of 1, meaning that any areas +-85.05113 latitude are inaccessible."
fn constrain_mercator(ll: &mut Ll) -> bool {
    let mut changed = false;
    if ll.lat > 85.0 {
        changed = true;
        ll.lat = 85.0;
    }
    if ll.lat < -85.0 {
        changed = true;
        ll.lat = -85.0;
    }
    while ll.lon > 180.0 {
        ll.lon -= 360.0;
    }
    while ll.lon <= -180.0 {
        ll.lon += 360.0;
    }
    changed
}

/// `lat_lon_distance`: metres between two points, by the haversine.
fn lat_lon_distance(a: Ll, b: Ll) -> f64 {
    let dlat = (b.lat - a.lat).to_radians();
    let dlon = (b.lon - a.lon).to_radians();
    let aa = (dlat / 2.0).sin().powi(2)
        + a.lat.to_radians().cos() * b.lat.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let cc = 2.0 * aa.sqrt().atan2((1.0 - aa).sqrt());
    6371.0 * cc * 1000.0 /* Radius of Earth in KM, distance in M */
}

/// `nearest_city`: what to call where we are.
fn nearest_city(pos: Ll) -> String {
    let mut nearest_i = 0;
    let mut nearest_d = f64::MAX;
    for (i, &(lat, lon, _)) in CITIES.iter().enumerate() {
        let d = lat_lon_distance(Ll { lat, lon }, pos);
        if d < nearest_d {
            nearest_d = d;
            nearest_i = i;
        }
    }
    let name = CITIES[nearest_i].2;
    let km = (nearest_d / 1000.0 + 0.5) as i64;
    if km < 10 {
        name.to_string()
    } else if km > 1000 {
        format!("{},{:03} km from {name}", km / 1000, km % 1000)
    } else {
        format!("{km} km from {name}")
    }
}

/// Which map to draw. Upstream's default is a template with an `{a-c}` server
/// alternation in it; the others are the ones its configuration file offers.
fn url_for(kind: &str) -> &'static str {
    match kind {
        "cycle" => "https://{a-c}.tile-cyclosm.openstreetmap.fr/cyclosm/{z}/{x}/{y}.png",
        "topo" => "https://{a-c}.tile.opentopomap.org/{z}/{x}/{y}.png",
        "humanitarian" => "https://{a-b}.tile.openstreetmap.fr/hot/{z}/{x}/{y}.png",
        _ => "https://{a-c}.tile.openstreetmap.org/{z}/{x}/{y}.png",
    }
}

/// Expand a slippy-map URL template: `{z}`, `{x}` and `{y}` are the tile, and
/// `{a-c}` is a run of interchangeable server names to spread the load over.
fn expand_url(template: &str, z: i32, x: i32, y: i32) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find('}') else {
            break;
        };
        let key = &rest[at + 1..at + end];
        rest = &rest[at + end + 1..];
        match key.trim_start_matches('$') {
            "z" => out.push_str(&z.to_string()),
            "x" => out.push_str(&x.to_string()),
            "y" => out.push_str(&y.to_string()),
            other => {
                // An `{a-c}`: pick one of the letters in the range.
                let bytes = other.as_bytes();
                if bytes.len() == 3 && bytes[1] == b'-' && bytes[0] <= bytes[2] {
                    let n = u32::from(bytes[2] - bytes[0]) + 1;
                    out.push((bytes[0] + (random() % n) as u8) as char);
                } else {
                    out.push_str(other);
                }
            }
        }
    }
    out.push_str(rest);
    out
}

/// How a tile is getting on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Blank,
    Loading,
    Ok,
    Failed,
}

#[derive(Clone)]
struct Tile {
    /// Which tile of the world, at `map_level`.
    map: (i32, i32),
    status: Status,
    /// Non-zero once there is a texture to draw.
    texid: u32,
    opacity: f32,
}

/// Where the map is going, and where it is on the way to.
#[derive(Clone, Copy, Default)]
struct Xy {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    FadeOut,
    Run,
}

struct MapState {
    url_template: String,
    map_level: i32,
    speed: f64,
    duration: f64,
    origin: String,
    pos: Ll,
    /// Where it was going, where it is going, and the eased blend of the two.
    heading: [Xy; 3],
    heading_ratio: f64,
    grid_w: i32,
    grid_h: i32,
    tiles: Vec<Tile>,
    font: TexFont,
    nearest_city: String,
    nearest_city_at: f64,
    /// One bit per level-12 tile: whether it is open sea.
    oceans: Vec<u64>,
    ocean_w: i32,
    ocean_p: bool,
    mode: Mode,
    change_time: f64,
    opacity: f64,
    titles: bool,
    arrow: bool,
    wire: bool,
    width: i32,
    height: i32,
    /// Whether anything is going to answer a tile request, so the tests and a
    /// hostless run draw the grid rather than waiting forever.
    tiles_available: bool,
}

impl MapState {
    /// `ocean_tile_p`: is this point in a tile that is open sea?
    fn ocean_tile_p(&self, lat: f64, lon: f64) -> bool {
        let x = lon2tilex(lon, OCEAN_LEVEL);
        let y = lat2tiley(lat, OCEAN_LEVEL);
        if x < 0 || y < 0 || x >= self.ocean_w || y >= self.ocean_w {
            return false;
        }
        let i = (y as usize) * (self.ocean_w as usize) + x as usize;
        self.oceans
            .get(i / 64)
            .is_some_and(|w| w >> (i % 64) & 1 == 1)
    }

    /// `mostly_ocean_p`: the centre and the eight tiles around it.
    fn mostly_ocean_p(&self) -> bool {
        let deg = 360.0 / f64::from(1 << self.map_level);
        for dlat in [-deg, 0.0, deg] {
            for dlon in [-deg, 0.0, deg] {
                if !self.ocean_tile_p(self.pos.lat + dlat, self.pos.lon + dlon) {
                    return false;
                }
            }
        }
        true
    }

    /// `randomize_position`.
    fn randomize_position(&mut self) {
        let city_p = self.origin.eq_ignore_ascii_case("random-city");
        let mut pos = Ll { lat: 0.0, lon: 0.0 };
        for i in 0..1000 {
            /* Don't get stuck */
            if city_p {
                if i == 0 {
                    /* Random city center */
                    let n = CITIES.len() - NONCITIES_COUNT;
                    let c = CITIES[(random() as usize) % n];
                    pos = Ll { lat: c.0, lon: c.1 };
                }
                /* Offset by a few miles, but not into the ocean */
                self.pos.lat = pos.lat + frand(0.05) - 0.025;
                self.pos.lon = pos.lon + frand(0.05) - 0.025;
            } else {
                let (north, south) = (70.0, -55.0); /* Greenland, Chile */
                self.pos.lat = frand(north - south) + south;
                self.pos.lon = frand(360.0) - 180.0;
            }
            constrain_mercator(&mut self.pos);
            self.ocean_p = self.mostly_ocean_p();
            if !self.ocean_p {
                break;
            }
        }
        self.nearest_city_at = 0.0;
    }

    /// `reshape_tiles`: rebuild the grid around the current position, keeping
    /// any tile whose image we already have.
    fn reshape_tiles(&mut self, g: &mut Gl) {
        let map_w = 1 << self.map_level;

        /* Two tile border around viewport, and round up. */
        let w2 = (f64::from(self.width) / TILE_PIXEL_SIZE + 4.0 + 0.5) as i32;
        let h2 = (f64::from(self.height) / TILE_PIXEL_SIZE + 4.0 + 0.5) as i32;

        let cx = lon2tilex(self.pos.lon, self.map_level);
        let cy = lat2tiley(self.pos.lat, self.map_level);
        let (tlx, tly) = (cx - w2 / 2, cy - h2 / 2);

        let old = std::mem::take(&mut self.tiles);
        let (ow, oh) = (self.grid_w, self.grid_h);
        let _ = (ow, oh);
        let mut tiles = Vec::with_capacity((w2 * h2) as usize);
        for y in 0..h2 {
            for x in 0..w2 {
                let mut mx = tlx + x;
                let my = tly + y;
                /* Wrap horizontally. */
                while mx < 0 {
                    mx += map_w;
                }
                while mx >= map_w {
                    mx -= map_w;
                }
                /* Hard edge vertically. */
                let map = if my < 0 || my >= map_w {
                    (-1, -1)
                } else {
                    (mx, my)
                };
                tiles.push(Tile {
                    map,
                    status: Status::Blank,
                    texid: 0,
                    opacity: 0.0,
                });
            }
        }

        /* If any of the old tiles have the same map coordinates, preserve
        their contents (images and textures) */
        for t in &mut tiles {
            if t.map.0 < 0 {
                continue;
            }
            if let Some(o) = old
                .iter()
                .find(|o| o.map == t.map && o.status != Status::Blank)
            {
                t.status = o.status;
                t.texid = o.texid;
                t.opacity = o.opacity;
            }
        }
        let _ = g;

        self.grid_w = w2;
        self.grid_h = h2;
        self.tiles = tiles;
    }

    /// Ask the host for anything the grid is missing.
    fn request_tiles(&mut self, g: &mut Gl) {
        if !self.tiles_available {
            return;
        }
        for t in &mut self.tiles {
            if t.status != Status::Blank || t.map.0 < 0 {
                continue;
            }
            t.status = Status::Loading;
            let key = tile_key(self.map_level, t.map.0, t.map.1);
            let url = expand_url(&self.url_template, self.map_level, t.map.0, t.map.1);
            g.request_tile(key, url);
        }
    }

    /// Collect whatever the host has fetched since the last frame.
    fn collect_tiles(&mut self, g: &mut Gl) {
        while let Some((key, image)) = g.take_tile() {
            let Some(t) = self
                .tiles
                .iter_mut()
                .find(|t| t.map.0 >= 0 && tile_key(self.map_level, t.map.0, t.map.1) == key)
            else {
                continue; /* scrolled off while it was in flight */
            };
            match image {
                Some(img) => {
                    let mut pixels = Vec::with_capacity((img.width() * img.height() * 4) as usize);
                    for p in img.pixels() {
                        let (r, gg, b) = crate::runtime::color::unrgb(*p);
                        pixels.extend_from_slice(&[r, gg, b, 255]);
                    }
                    let id = g.glx.gen_texture();
                    g.glx.bind_texture(id);
                    g.glx.tex_image_2d(img.width(), img.height(), pixels);
                    g.glx.tex_nearest(false);
                    g.glx.tex_clamp(true);
                    t.texid = id;
                    t.status = Status::Ok;
                    t.opacity = 0.0;
                }
                None => t.status = Status::Failed,
            }
        }
    }

    /// `draw_tile`.
    fn draw_tile(&self, g: &mut Gl, i: usize) {
        let t = &self.tiles[i];
        let w = TILE_PIXEL_SIZE as f32;
        if !self.wire {
            g.glx.bind_texture(t.texid);
            g.glx.texturing(t.texid != 0);
        }

        if self.wire {
            g.glx.color3f(0.0, 1.0, 0.0);
        } else if t.texid == 0 {
            g.glx.color4f(0.0, 0.0, 0.0, 0.03 * self.opacity as f32); /* grid color */
        } else {
            g.glx
                .color4f(1.0, 1.0, 1.0, t.opacity * self.opacity as f32);
        }

        g.glx.front_face_cw(false);
        g.glx.begin(if self.wire || t.texid == 0 {
            Shape::LineLoop
        } else {
            Shape::Quads
        });
        // The texture coordinates are upstream's upside down, because the
        // pixels arrive the other way up. A tile's first row is its north
        // edge, and that is what gets uploaded as v = 0; upstream's image
        // loader hands GL its rows bottom-up, so there v = 0 is the south
        // edge. Flipping v here rather than reversing the rows on the way in
        // costs nothing and keeps the tile bytes untouched.
        g.glx.tex_coord2f(0.0, 1.0);
        g.glx.vertex3f(0.0, 0.0, 0.0);
        g.glx.tex_coord2f(1.0, 1.0);
        g.glx.vertex3f(w, 0.0, 0.0);
        g.glx.tex_coord2f(1.0, 0.0);
        g.glx.vertex3f(w, w, 0.0);
        g.glx.tex_coord2f(0.0, 0.0);
        g.glx.vertex3f(0.0, w, 0.0);
        g.glx.end();

        if t.status == Status::Failed {
            /* X them out */
            let s = 0.15;
            if !self.wire {
                g.glx.texturing(false);
                g.glx.color4f(0.0, 0.0, 0.0, 0.2 * self.opacity as f32);
            }
            g.glx.push_matrix();
            g.glx.translate(w / 2.0, w / 2.0, 0.0);
            g.glx.scale(s, s, s);
            g.glx.translate(-w / 2.0, -w / 2.0, 0.0);
            g.glx.begin(Shape::Lines);
            g.glx.vertex3f(0.0, 0.0, 0.0);
            g.glx.vertex3f(w, w, 0.0);
            g.glx.vertex3f(w, 0.0, 0.0);
            g.glx.vertex3f(0.0, w, 0.0);
            g.glx.end();
            g.glx.pop_matrix();
        }
    }

    /// `draw_tiles`: the grid, scrolled so that `pos` is in the middle.
    fn draw_tiles(&mut self, g: &mut Gl) {
        /* What tile contains pos? */
        let px = lon2tilex(self.pos.lon, self.map_level);
        let py = lat2tiley(self.pos.lat, self.map_level);

        /* What is that tile's origin and extent? */
        let lon0 = tilex2lon(px, self.map_level);
        let lat0 = tiley2lat(py, self.map_level);
        let lon1 = tilex2lon(px + 1, self.map_level);
        let lat1 = tiley2lat(py + 1, self.map_level);

        /* How far should the tile be scrolled? */
        let mut offx = (self.pos.lon - lon0) / (lon1 - lon0) * TILE_PIXEL_SIZE;
        let mut offy = -(self.pos.lat - lat0) / (lat1 - lat0) * TILE_PIXEL_SIZE;

        /* And center */
        offx += f64::from(self.grid_w) / 2.0 * TILE_PIXEL_SIZE - TILE_PIXEL_SIZE / 2.0;
        offy -= f64::from(self.grid_h) / 2.0 * TILE_PIXEL_SIZE - TILE_PIXEL_SIZE / 2.0;

        g.glx.push_matrix();
        g.glx
            .scale(1.0 / self.width as f32, 1.0 / self.height as f32, 1.0);

        for y in 0..self.grid_h {
            for x in 0..self.grid_w {
                g.glx.push_matrix();
                g.glx.translate(
                    (-f64::from(self.grid_w) / 2.0 - offx + f64::from(x) * TILE_PIXEL_SIZE) as f32,
                    (-f64::from(self.grid_h) / 2.0
                        - offy
                        - f64::from(y) * TILE_PIXEL_SIZE
                        - TILE_PIXEL_SIZE) as f32,
                    0.0,
                );
                self.draw_tile(g, (y * self.grid_w + x) as usize);
                g.glx.pop_matrix();
            }
        }
        g.glx.pop_matrix();

        /* Fade each tile in once it has arrived. */
        for t in &mut self.tiles {
            if t.texid != 0 && t.opacity < 1.0 {
                t.opacity = (t.opacity + 0.1).min(1.0);
            }
        }
    }

    /// `draw_arrow`: which way we are going.
    fn draw_arrow(&self, g: &mut Gl) {
        let s = 0.02;
        g.glx.texturing(false);
        g.glx.push_matrix();
        g.glx.scale(s, s, s);
        g.glx
            .scale(self.height as f32 / self.width as f32, 1.0, 1.0);
        g.glx.rotate(
            (-self.heading[2].x.atan2(self.heading[2].y)).to_degrees() as f32,
            0.0,
            0.0,
            1.0,
        );
        g.glx.color4f(0.0, 0.0, 0.0, 0.4 * self.opacity as f32);
        g.glx.begin(Shape::LineLoop);
        for (x, y) in [(0.0, 1.0), (-0.5, -1.0), (0.0, -0.5), (0.5, -1.0)] {
            g.glx.vertex3f(x, y, 0.0);
        }
        g.glx.end();
        g.glx.pop_matrix();
    }

    /// The corner caption: where we are, and what is nearest.
    fn title(&self) -> String {
        let alat = self.pos.lat.abs();
        let alon = self.pos.lon.abs();
        let mut buf = format!(
            "{}\u{b0} {}' {}, {}\u{b0} {}' {}",
            alat as i32,
            ((alat - alat.floor()) * 60.0) as i32,
            if self.pos.lat >= 0.0 { 'N' } else { 'S' },
            alon as i32,
            ((alon - alon.floor()) * 60.0) as i32,
            if self.pos.lon >= 0.0 { 'E' } else { 'W' },
        );
        if !self.nearest_city.is_empty() {
            buf.push('\n');
            buf.push_str(&self.nearest_city);
        }
        buf
    }
}

/// A tile's identity, packed so the host can hand it back.
fn tile_key(z: i32, x: i32, y: i32) -> u64 {
    (z as u64) << 56 | (x as u64 & 0xFFF_FFFF) << 28 | (y as u64 & 0xFFF_FFFF)
}

impl Hack3d for MapState {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        let s = 2.0;
        g.glx.scale(s, s, s);
        self.reshape_tiles(g);
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let now = g.elapsed();
        g.glx.clear_color(1.0, 1.0, 1.0, 1.0);
        g.glx.clear();
        g.glx.blend(Blend::Alpha);
        g.glx.depth_test(false);
        g.glx.lighting(false);
        g.glx.cull_face(false);

        self.collect_tiles(g);

        if self.mode == Mode::Run {
            let tile_degrees = 360.0 / f64::from(1 << self.map_level);
            let degrees_per_pixel = tile_degrees / TILE_PIXEL_SIZE;
            let mut force_change_p;

            let (dx, dy) = (self.heading[2].x, self.heading[2].y);
            let d = (dx * dx + dy * dy).sqrt().max(1e-9);
            let (dx, dy) = (dx / d, dy / d); /* normalize */

            /* Scroll more slowly when not all the tiles are loaded. */
            let total = self.tiles.len().max(1) as f64;
            let loaded = self
                .tiles
                .iter()
                .filter(|t| t.status != Status::Loading)
                .count() as f64;
            let failed = self
                .tiles
                .iter()
                .filter(|t| t.status == Status::Failed)
                .count() as f64;
            let loaded_ratio = (loaded / total).max(0.1);
            let failed_ratio = failed / total;

            self.pos.lon += dx * self.speed * degrees_per_pixel * 0.3 * loaded_ratio;
            self.pos.lat += dy * self.speed * degrees_per_pixel * 0.3 * loaded_ratio;
            force_change_p = constrain_mercator(&mut self.pos);

            /* There goes the neighborhood. If we're getting a lot of 404s,
            move. */
            if failed_ratio > 0.8 {
                force_change_p = true;
            }

            if !force_change_p && self.heading_ratio >= 1.0 {
                let was_ocean_p = self.ocean_p;
                self.ocean_p = if was_ocean_p {
                    /* Be more strict on "land ho" */
                    self.ocean_tile_p(self.pos.lat, self.pos.lon)
                } else {
                    self.mostly_ocean_p()
                };
                /* If we have just landed in the ocean, reverse gears and move
                in the opposite direction until we're out. */
                if !was_ocean_p && self.ocean_p {
                    self.heading[0] = self.heading[2];
                    self.heading[1] = self.heading[2];
                    self.heading[1].x = -self.heading[1].x;
                    self.heading[1].y = -self.heading[1].y;
                    self.heading_ratio = 0.0;
                }
            }

            let wanders = self.origin.eq_ignore_ascii_case("random")
                || self.origin.eq_ignore_ascii_case("random-city");
            if wanders && (force_change_p || now > self.change_time + self.duration) {
                self.mode = Mode::FadeOut;
                force_change_p = false;
            }

            if force_change_p
                || (!self.ocean_p && self.heading_ratio >= 1.0 && random().is_multiple_of(1000))
            {
                let (mut nx, mut ny) = (frand(2.0) - 1.0, frand(2.0) - 1.0);
                let d = (nx * nx + ny * ny).sqrt().max(1e-9);
                nx /= d;
                ny /= d;
                self.heading[0] = self.heading[2];
                self.heading[1] = Xy { x: nx, y: ny };
                self.heading_ratio = 0.0;
            }
        }

        if self.mode == Mode::Run && self.heading_ratio < 1.0 {
            self.heading_ratio = (self.heading_ratio + 0.003).min(1.0);
            let mut th0 = self.heading[0].x.atan2(self.heading[0].y);
            let mut th1 = self.heading[1].x.atan2(self.heading[1].y);
            let tau = std::f64::consts::TAU;
            while th0 < 0.0 {
                th0 += tau;
            }
            while th1 < 0.0 {
                th1 += tau;
            }
            if th1 - th0 > std::f64::consts::PI || th1 - th0 <= -std::f64::consts::PI {
                if th1 < th0 {
                    th1 += tau;
                } else {
                    th0 += tau;
                }
            }
            let th2 = th0 + (th1 - th0) * ease(Ease::InOutSine, self.heading_ratio);
            self.heading[2] = Xy {
                x: th2.sin(),
                y: th2.cos(),
            };
        }

        if self.mode == Mode::FadeOut {
            self.opacity -= 0.02;
            if self.opacity < 0.0 {
                self.opacity = 1.0;
                self.mode = Mode::Run;
                self.change_time = now;
                self.randomize_position();
                self.reshape_tiles(g);
            }
        }

        if self.nearest_city_at < now {
            /* Only update this once a second. */
            self.nearest_city = nearest_city(self.pos);
            self.nearest_city_at = now + 1.0;
        }

        if self.mode == Mode::Run {
            self.reshape_tiles(g);
        }
        self.request_tiles(g);
        self.draw_tiles(g);
        if self.arrow {
            self.draw_arrow(g);
        }

        if self.titles && self.mode == Mode::Run {
            let title = self.title();
            self.font.print_label(
                &mut g.glx,
                &title,
                self.width,
                self.height,
                1,
                [0.0, 0.0, 0.0, self.opacity as f32],
            );
        }

        g.res.int("delay").max(0) as u32
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let (ocean_w, oceans) = match crate::runtime::png::decode_mask(
        OCEAN_TILES,
        crate::runtime::color::rgb(0, 0, 255),
    ) {
        Some((w, _h, bits)) => (w, bits),
        None => (0, Vec::new()),
    };

    let mut st = MapState {
        url_template: url_for(g.res.string("map")).to_string(),
        map_level: g.res.int("mapLevel").clamp(1, 19),
        speed: g.res.float("speed"),
        duration: f64::from(g.res.int("duration").max(1)),
        origin: g.res.string("origin").to_string(),
        pos: Ll { lat: 0.0, lon: 0.0 },
        heading: [Xy { x: 0.0, y: 1.0 }; 3],
        heading_ratio: 1.0,
        grid_w: 0,
        grid_h: 0,
        tiles: Vec::new(),
        font: TexFont::load(&mut g.glx, "sans-serif 18"),
        nearest_city: String::new(),
        nearest_city_at: 0.0,
        oceans,
        ocean_w,
        ocean_p: false,
        mode: Mode::Run,
        change_time: 0.0,
        opacity: 1.0,
        titles: g.res.bool("titles"),
        arrow: g.res.bool("arrow"),
        wire,
        width: g.width(),
        height: g.height(),
        tiles_available: g.tiles_available(),
    };

    let (dx, dy) = (frand(2.0) - 1.0, frand(2.0) - 1.0);
    let d = (dx * dx + dy * dy).sqrt().max(1e-9);
    st.heading = [Xy {
        x: dx / d,
        y: dy / d,
    }; 3];

    st.randomize_position();
    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      30000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*map:        osm",
    "*mapLevel:   15",
    "*speed:      1.0",
    "*duration:   1800",
    "*origin:     random-city",
    "*titles:     True",
    "*arrow:      True",
];

const MAPS: &[SelectItem] = &[
    SelectItem {
        value: "osm",
        label: "Open Street Map",
    },
    SelectItem {
        value: "cycle",
        label: "CyclOSM",
    },
    SelectItem {
        value: "topo",
        label: "Open Topo Map",
    },
    SelectItem {
        value: "humanitarian",
        label: "Humanitarian",
    },
];

const ORIGINS: &[SelectItem] = &[
    SelectItem {
        value: "random-city",
        label: "Random city",
    },
    SelectItem {
        value: "random",
        label: "Fully random location",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("mapLevel", "Zoom", 3.0, 19.0, 1.0, 0, "15"),
    Opt::slider(
        "duration",
        "Seconds per place",
        30.0,
        3600.0,
        30.0,
        0,
        "1800",
    ),
    Opt::select("map", "Map", MAPS, "osm"),
    Opt::select("origin", "Origin", ORIGINS, "random-city"),
    Opt::boolean("titles", "Show location", "true"),
    Opt::boolean("arrow", "Show heading", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "mapscroller",
    label: "Map Scroller",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2021",
        video: Some("https://www.youtube.com/watch?v=99w8VfCU3Pg"),
        blurb: "A slowly scrolling map of somewhere on Earth.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    /// The slippy-map transforms are inverses of each other, which is the one
    /// thing every tile coordinate depends on.
    ///
    /// The invariant is containment, not an exact round trip of the tile's
    /// corner: `tiley2lat` of a tile index gives the boundary latitude between
    /// two tiles, and converting that back can land on either side of it in
    /// floating point. Asking whether a point is inside the tile it was
    /// assigned to is the question that actually matters, so the round trip is
    /// taken from the middle of a tile rather than its edge.
    #[test]
    fn a_point_lands_inside_the_tile_it_is_given() {
        for z in [3, 12, 15, 19] {
            for &(lat, lon, name) in CITIES.iter().take(40) {
                let (x, y) = (lon2tilex(lon, z), lat2tiley(lat, z));

                // The tile's own bounds contain the point.
                let (west, east) = (tilex2lon(x, z), tilex2lon(x + 1, z));
                let (north, south) = (tiley2lat(y, z), tiley2lat(y + 1, z));
                assert!(
                    west <= lon && lon < east,
                    "{name} at z{z}: {lon} not in {west}..{east}"
                );
                assert!(
                    south <= lat && lat <= north,
                    "{name} at z{z}: {lat} not in {south}..{north}"
                );

                // And the middle of a tile round-trips back to it.
                let mid_lon = (west + east) / 2.0;
                let mid_lat = (north + south) / 2.0;
                assert_eq!(lon2tilex(mid_lon, z), x, "{name} at z{z}");
                assert_eq!(lat2tiley(mid_lat, z), y, "{name} at z{z}");
            }
        }
    }

    /// Mercator cannot reach the poles, and longitude wraps rather than
    /// clamping. Upstream returns whether it had to move the point, and that
    /// return is what makes the saver pick somewhere new.
    #[test]
    fn the_map_wraps_sideways_and_stops_at_the_poles() {
        let mut p = Ll {
            lat: 89.0,
            lon: 0.0,
        };
        assert!(constrain_mercator(&mut p));
        assert_eq!(p.lat, 85.0);

        let mut p = Ll {
            lat: 0.0,
            lon: 190.0,
        };
        assert!(
            !constrain_mercator(&mut p),
            "longitude wrapping is not a move"
        );
        assert_eq!(p.lon, -170.0);

        let mut p = Ll {
            lat: 0.0,
            lon: -190.0,
        };
        constrain_mercator(&mut p);
        assert_eq!(p.lon, 170.0);

        // Anywhere already in range is left alone.
        let mut p = Ll {
            lat: 12.0,
            lon: -34.0,
        };
        assert!(!constrain_mercator(&mut p));
        assert_eq!((p.lat, p.lon), (12.0, -34.0));
    }

    /// The URL template is upstream's slippy-map form, including the `{a-c}`
    /// alternation that spreads requests over the tile servers.
    #[test]
    fn urls_name_the_right_tile() {
        crate::runtime::rand::ya_rand_init(20260812);
        let t = "https://{a-c}.tile.openstreetmap.org/{z}/{x}/{y}.png";
        let mut servers = std::collections::BTreeSet::new();
        for _ in 0..60 {
            let u = expand_url(t, 15, 17602, 10748);
            assert!(u.ends_with("/15/17602/10748.png"), "{u}");
            assert!(u.starts_with("https://"), "{u}");
            servers.insert(u["https://".len()..].chars().next().unwrap());
        }
        assert_eq!(
            servers,
            "abc".chars().collect(),
            "the server alternation should use every letter of the range"
        );
    }

    /// The sea map answers for real places. This is the whole reason the
    /// 4096-square image is carried at all: without it the saver scrolls into
    /// the Pacific and stays there.
    #[test]
    fn the_sea_map_knows_sea_from_land() {
        let (w, _h, bits) =
            crate::runtime::png::decode_mask(OCEAN_TILES, crate::runtime::color::rgb(0, 0, 255))
                .expect("ocean map");
        let st = MapState {
            oceans: bits,
            ocean_w: w,
            map_level: 15,
            ..blank_state()
        };
        for (name, lat, lon) in [
            ("mid Pacific", -10.0, -140.0),
            ("south Atlantic", -35.0, -20.0),
            ("Indian Ocean", -30.0, 80.0),
        ] {
            assert!(st.ocean_tile_p(lat, lon), "{name} should be sea");
        }
        for (name, lat, lon) in [
            ("Sahara", 23.0, 13.0),
            ("central Australia", -25.0, 132.0),
            ("Kansas", 38.5, -98.0),
        ] {
            assert!(!st.ocean_tile_p(lat, lon), "{name} should be land");
        }
    }

    /// Nearly every city in the table is on land, which is the premise of
    /// starting at one.
    ///
    /// Exactly one is not, and it is the data being right rather than wrong:
    /// Funafuti is an atoll narrower than a level-12 tile, so the tile holding
    /// it is all sea. Upstream copes by giving up after a thousand tries and
    /// moving on, which this port does too. The test is here to catch a
    /// systematic coordinate error, which would put hundreds of cities out to
    /// sea rather than one.
    #[test]
    fn the_cities_are_ashore_bar_one_atoll() {
        let (w, _h, bits) =
            crate::runtime::png::decode_mask(OCEAN_TILES, crate::runtime::color::rgb(0, 0, 255))
                .expect("ocean map");
        let st = MapState {
            oceans: bits,
            ocean_w: w,
            map_level: 15,
            ..blank_state()
        };
        let mut wet = Vec::new();
        for &(lat, lon, name) in CITIES.iter().take(CITIES.len() - NONCITIES_COUNT) {
            if st.ocean_tile_p(lat, lon) {
                wet.push(name);
            }
        }
        assert_eq!(
            wet,
            vec!["Funafuti, Tuvalu"],
            "a different set of cities came out in the sea"
        );
    }

    /// Naming the nearest place: on top of a city it is just the name, and far
    /// away it is a distance and a name.
    #[test]
    fn the_caption_names_the_nearest_place() {
        let tokyo = Ll {
            lat: 35.6897,
            lon: 139.6922,
        };
        assert_eq!(nearest_city(tokyo), "Tokyo, Japan");
        // A hundred-odd km out, it says how far.
        let out = nearest_city(Ll {
            lat: 34.5,
            lon: 139.6922,
        });
        assert!(out.contains("km from"), "{out}");
        assert!(out.ends_with("Japan"), "{out}");
        // And the middle of the Pacific is a very long way from anywhere.
        let nemo = nearest_city(Ll {
            lat: -48.877,
            lon: -123.393,
        });
        assert_eq!(nemo, "Point Nemo", "the non-cities are still nameable");
    }

    fn blank_state() -> MapState {
        MapState {
            url_template: String::new(),
            map_level: 15,
            speed: 1.0,
            duration: 1800.0,
            origin: "random-city".into(),
            pos: Ll { lat: 0.0, lon: 0.0 },
            heading: [Xy { x: 0.0, y: 1.0 }; 3],
            heading_ratio: 1.0,
            grid_w: 0,
            grid_h: 0,
            tiles: Vec::new(),
            font: TexFont::load(&mut crate::runtime::gl::Glx::new(), "sans-serif 18"),
            nearest_city: String::new(),
            nearest_city_at: 0.0,
            oceans: Vec::new(),
            ocean_w: 0,
            ocean_p: false,
            mode: Mode::Run,
            change_time: 0.0,
            opacity: 1.0,
            titles: true,
            arrow: true,
            wire: false,
            width: 640,
            height: 480,
            tiles_available: false,
        }
    }

    /// The same, with a host answering: the saver asks for tiles, is given
    /// them, and draws them. This is the path the browser takes, and it is
    /// separate from the hostless one because everything about textures only
    /// happens when something answers.
    #[test]
    fn it_draws_the_tiles_a_host_gives_it() {
        let mut r = Runner3d::start(
            &DEF,
            init,
            StartArgs::new(640, 480, "", 20260812).with_tile_host(true),
        );
        let mut served = 0;
        for _ in 0..90 {
            r.step();
            for (key, url) in r.take_tile_requests() {
                assert!(url.starts_with("https://"), "{url}");
                // A plain grey tile is enough: the question is whether the
                // saver can take one at all.
                let mut img = crate::runtime::XImage::new(256, 256);
                for y in 0..256 {
                    for x in 0..256 {
                        img.put_pixel(x, y, crate::runtime::color::rgb(128, 128, 128));
                    }
                }
                r.deliver_tile(key, Some(img));
                served += 1;
            }
        }
        assert!(served > 4, "only {served} tiles were ever asked for");
        let f = r.frame();
        assert!(!f.vertices.is_empty());
    }

    /// With no host to fetch tiles the saver still runs: it draws the empty
    /// grid and keeps scrolling, which is what the native tests see.
    #[test]
    fn it_runs_with_no_tiles_at_all() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        for _ in 0..120 {
            r.step();
        }
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "the grid was not drawn");
        assert!(
            f.batches.len() < 400,
            "a frame came to {} batches",
            f.batches.len()
        );
    }
}
