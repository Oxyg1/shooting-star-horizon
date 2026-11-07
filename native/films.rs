// The films' space scenes: the original's `StarPath`, `GungnirPath`,
// `SevenStarsPath`, the Tesseract's and Gravitia's, which set their space
// shaders' uniforms from the film's clock. Those are 1.5.2's own numbers:
// `tools/films` runs the original's path classes and keeps every uniform
// they give over the film's time (more samples where one moves fast), so
// here a film's frame is a lookup and a straight line between two samples.
//
// The windows (when the space covers the world, when the railgun's own
// scene plays) are the original's, checked against 1.5.2's to 1e-7.
use horizon_client_render::gfx::native_render::shooting_star_fxframe::{self as fx, Pass, Screen, Value};
use std::sync::Mutex;

/// One uniform over the film's time.
struct Track {
    name: &'static str,
    len: usize,
    /// Sample times (film ticks); empty: one value throughout.
    times: Vec<f32>,
    values: Vec<f32>,
}

/// Every uniform one program of a film is given.
pub struct Table {
    tracks: Vec<Track>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn f32s(b: &[u8], at: usize, n: usize) -> Option<Vec<f32>> {
    let bytes = b.get(at..at + n * 4)?;
    Some(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

impl Table {
    /// `SSFT` 2: per uniform its name, length and either one value or its
    /// sample times and values.
    pub fn parse(b: &[u8]) -> Option<Table> {
        if b.get(..4)? != b"SSFT" || u32_at(b, 4)? != 2 {
            return None;
        }
        let count = u32_at(b, 8)? as usize;
        let mut at = 12;
        let mut tracks = Vec::with_capacity(count);
        for _ in 0..count {
            let n = usize::from(*b.get(at)?);
            let name = std::str::from_utf8(b.get(at + 1..at + 1 + n)?).ok()?;
            at += 1 + n;
            let len = usize::from(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?));
            at += 2;
            let samples = u32_at(b, at)? as usize;
            at += 4;
            let (times, values) = if samples == 0 {
                let v = f32s(b, at, len)?;
                at += len * 4;
                (Vec::new(), v)
            } else {
                let t = f32s(b, at, samples)?;
                at += samples * 4;
                let v = f32s(b, at, samples * len)?;
                at += samples * len * 4;
                (t, v)
            };
            // Named once for the program's whole life.
            let name: &'static str = Box::leak(name.to_owned().into_boxed_str());
            tracks.push(Track { name, len, times, values });
        }
        Some(Table { tracks })
    }

    /// Every uniform at film tick `ct`.
    pub fn sample(&self, ct: f32) -> Vec<(&'static str, Vec<f32>)> {
        self.tracks.iter().map(|t| (t.name, t.at(ct))).collect()
    }
}

impl Track {
    fn at(&self, ct: f32) -> Vec<f32> {
        if self.times.is_empty() {
            return self.values.clone();
        }
        let n = self.times.len();
        let row = |i: usize| &self.values[i * self.len..(i + 1) * self.len];
        if ct <= self.times[0] {
            return row(0).to_vec();
        }
        if ct >= self.times[n - 1] {
            return row(n - 1).to_vec();
        }
        let i = self.times.partition_point(|t| *t <= ct) - 1;
        let (a, b) = (self.times[i], self.times[i + 1]);
        let k = (ct - a) / (b - a).max(1e-6);
        row(i).iter().zip(row(i + 1)).map(|(x, y)| x + (y - x) * k).collect()
    }
}

static TABLES: Mutex<Vec<(&'static str, Option<&'static Table>)>> = Mutex::new(Vec::new());

/// The film table `name` (`star_space`), read once the game has its file.
pub fn table(name: &str) -> Option<&'static Table> {
    let mut tables = match TABLES.lock() {
        Ok(t) => t,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((_, t)) = tables.iter().find(|(n, _)| *n == name) {
        return *t;
    }
    let path = format!("films/{name}.bin");
    let bytes = fx::asset(&path)?;
    let table = Table::parse(bytes).map(|t| &*Box::leak(Box::new(t)));
    if table.is_none() {
        log::warn!("[shooting_star] the film table {path} could not be read");
    }
    let name: &'static str = Box::leak(name.to_owned().into_boxed_str());
    tables.push((name, table));
    table
}

/// The space scenes' textures, as the original's `StarPath.SAMPLERS`.
pub const SPACE_TEXTURES: &[(&str, &str)] = &[
    ("EarthDay", "moonfall/earth_day.jpg"),
    ("EarthAux", "moonfall/earth_lights.jpg+moonfall/earth_clouds.jpg+moonfall/earth_height.jpg"),
    ("MoonColor", "moonfall/moon_color.jpg"),
    ("MilkyWay", "moonfall/milky_way.jpg"),
    ("MilkyTop", "shooting_star/milky_way_top.jpg"),
    ("Planets", "shooting_star/planets.jpg"),
    ("Rings", "shooting_star/saturn_ring.jpg"),
    ("Jupiter", "shooting_star/jupiter.jpg"),
    ("Nebula", "gravitia/nebula.jpg"),
];

/// A pass of `program` from its film table at `ct`: `slot` (`S0`, `G0`...)
/// takes the spell's seed at `[2]` and, for a space scene, its cover at `[3]`.
pub fn pass(program: &'static str, table: &str, ct: f32, slot: Option<&str>, seed: f32, cover: Option<f32>) -> Option<Pass> {
    let table = self::table(table)?;
    let mut pass = Pass::new(program);
    for (name, mut v) in table.sample(ct) {
        if Some(name) == slot && v.len() == 4 {
            v[2] = seed;
            if let Some(c) = cover {
                v[3] = c;
            }
        }
        let value = if v.len() > 4 { Value::Vec4s(v.chunks(4).map(|c| [c[0], c[1], c[2], *c.get(3).unwrap_or(&0.0)]).collect()) } else { Value::F(v) };
        pass.uniforms.push((name, value));
    }
    for (sampler, file) in SPACE_TEXTURES {
        pass = pass.texture(sampler, file);
    }
    Some(pass)
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn smooth(x: f32) -> f32 {
    let x = clamp01(x);
    x * x * (3.0 - 2.0 * x)
}

fn window(t: f32, in0: f32, in1: f32, out0: f32, out1: f32) -> f32 {
    clamp01((t - in0) / (in1 - in0).max(0.001)) * (1.0 - clamp01((t - out0) / (out1 - out0).max(0.001)))
}

// ---- SS-01 ----------------------------------------------------------------------------------------------------------

pub fn star_dissolve(ct: f32) -> f32 {
    clamp01((ct - 58.0) / 14.0)
}

/// How much of the frame is space (`StarPath.cover`).
pub fn star_cover(ct: f32) -> f32 {
    if !(58.0..342.0).contains(&ct) || star_gun(ct) {
        0.0
    } else if ct >= 318.0 {
        1.0
    } else {
        star_dissolve(ct)
    }
}

/// The railgun's own scene plays (`StarPath.gun`).
pub fn star_gun(ct: f32) -> bool {
    (238.0..318.0).contains(&ct)
}

pub fn star_flat(ct: f32) -> f32 {
    if (331.0..342.0).contains(&ct) { smooth((ct - 331.0) / 2.0) } else { 0.0 }
}

pub fn star_charge(ct: f32) -> f32 {
    clamp01((ct - 291.0) / 19.0)
}

pub fn star_implode(ct: f32) -> f32 {
    if ct >= 314.0 { 0.0 } else { smooth((ct - 310.0) / 2.5) }
}

/// SS-01's film passes; true when they cover the world whole.
pub fn star_passes(passes: &mut Vec<Pass>, ct: f32, weight: f32, seed: f32) -> bool {
    let cover = star_cover(ct) * weight;
    if star_gun(ct) {
        passes.extend(pass("star_gun", "star_gun", ct, Some("S0"), seed, None));
        passes.extend(pass("star_rays", "star_rays", ct, None, seed, None));
        return true;
    }
    if cover > 0.001 {
        passes.extend(pass("star_space", "star_space", ct, Some("S0"), seed, Some(cover)));
    }
    cover >= 0.999
}

/// `StarFx.filmScreen`.
pub fn star_screen(screen: &mut Screen, ct: f32, weight: f32) {
    let cover = star_cover(ct) * weight;
    let gun = star_gun(ct);
    screen.synthetic = screen.synthetic.max(if gun { weight } else { cover });
    let flat = star_flat(ct);
    if cover > 0.001 || gun {
        screen.bloom = screen.bloom.max(0.5 * (1.0 - flat * 0.85));
    }
    for jump in [126.0, 170.0, 204.0, 214.0] {
        let k = window(ct, jump - 1.0, jump, jump + 1.0, jump + 5.0);
        screen.aberration = screen.aberration.max(k * 0.9);
        screen.add_flash(0x9FE7FF, k * 0.18);
    }
    if gun && ct < 314.0 {
        let charge = star_charge(ct);
        let implode = star_implode(ct);
        screen.aberration = screen.aberration.max(charge * charge * 0.35 + implode * 0.5);
        screen.shake = screen.shake.max(charge * charge * 0.12 * (1.0 - implode));
        screen.add_vignette(0, implode * 0.6);
    }
    if (314.0..320.0).contains(&ct) {
        let since = ct - 314.0;
        screen.add_flash(0xFFFFFF, if since < 1.0 { 3.2 } else { 3.2 * (-(since - 1.0) * 1.6).exp() });
        screen.shake = screen.shake.max((-since * 0.5).exp());
        screen.aberration = screen.aberration.max((-since * 0.4).exp() * 1.2);
        screen.zoom_blur = screen.zoom_blur.max((-since * 0.6).exp() * 0.7);
    }
    if (318.0..342.0).contains(&ct) {
        screen.zoom_blur = screen.zoom_blur.max(0.5 * (1.0 - flat));
        screen.shake = screen.shake.max(0.06);
        screen.aberration = screen.aberration.max(0.3 * (1.0 - flat));
    }
    for cut in [331.0, 342.0] {
        let k = window(ct, cut - 0.5, cut, cut + 1.0, cut + 3.0);
        screen.shake = screen.shake.max(k * 0.7);
        screen.aberration = screen.aberration.max(k * 1.1);
    }
    if (341.5..352.0).contains(&ct) {
        let k = smooth((ct - 342.0) / 10.0);
        screen.add_vignette(0x2A0004, 0.2 + 0.35 * k);
    }
}

/// The film covers the world enough that the world's own screen effects wait.
pub fn star_synthetic(ct: f32) -> bool {
    star_cover(ct) > 0.5 || star_gun(ct)
}

// ---- SS-03 ----------------------------------------------------------------------------------------------------------

pub fn gungnir_cover(ct: f32) -> f32 {
    if (50.0..300.0).contains(&ct) { clamp01((ct - 50.0) / 12.0) } else { 0.0 }
}

pub fn gungnir_mech(ct: f32) -> bool {
    (110.0..300.0).contains(&ct)
}

fn lap_time(i: i32) -> f32 {
    (168.0 + 84.0 * (f64::from(i) / 7.0).powf(0.454_545_454_545_454_5)) as f32
}

pub fn gungnir_passes(passes: &mut Vec<Pass>, ct: f32, weight: f32, seed: f32) -> bool {
    let cover = gungnir_cover(ct) * weight;
    if cover > 0.001 {
        passes.extend(pass("gungnir_space", "gungnir_space", ct, Some("G0"), seed, Some(cover)));
        if gungnir_mech(ct) {
            passes.extend(pass("gungnir_mech", "gungnir_mech", ct, Some("N0"), seed, None));
        }
    }
    cover >= 0.999
}

/// `GungnirFx.filmScreen`.
pub fn gungnir_screen(screen: &mut Screen, ct: f32, weight: f32) {
    let cover = gungnir_cover(ct) * weight;
    screen.synthetic = screen.synthetic.max(cover);
    if cover > 0.001 {
        screen.bloom = screen.bloom.max(0.55);
    }
    for jump in [84.0, 110.0] {
        let k = window(ct, jump - 1.0, jump, jump + 1.0, jump + 5.0);
        screen.aberration = screen.aberration.max(k * 0.9);
        screen.add_flash(0xFFC8A0, k * 0.22);
    }
    let ring = window(ct, 124.0, 128.0, 140.0, 146.0);
    screen.aberration = screen.aberration.max(ring * 0.2);
    let push = window(ct, 141.0, 145.0, 146.0, 147.0);
    screen.zoom_blur = screen.zoom_blur.max(push * 0.5);
    let lock = ct - 160.0;
    if (0.0..12.0).contains(&lock) {
        screen.shake = screen.shake.max((-lock * 0.5).exp() * 0.9);
        screen.aberration = screen.aberration.max((-lock * 0.4).exp() * 0.7);
        screen.add_flash(0xFFE7C8, (-lock * 1.2).exp() * 0.6);
    }
    if (168.0..252.0).contains(&ct) {
        let k = (ct - 168.0) / 84.0;
        screen.shake = screen.shake.max(0.05 + 0.35 * k * k);
        screen.aberration = screen.aberration.max(0.1 + 0.6 * k * k);
        if !(202.0..226.0).contains(&ct) {
            screen.zoom_blur = screen.zoom_blur.max(0.55 * k * k);
        }
        for i in 1..7 {
            let at = lap_time(i);
            let lap = window(ct, at - 0.5, at, at + 0.5, at + 3.0);
            screen.shake = screen.shake.max(lap * (0.3 + 0.1 * i as f32));
            screen.aberration = screen.aberration.max(lap * 0.8);
            screen.add_flash(0xFFD8B0, lap * 0.15 * i as f32 / 6.0);
        }
    }
    let rel = ct - 252.0;
    if (0.0..10.0).contains(&rel) {
        screen.add_flash(0xFFFFFF, if rel < 1.0 { 3.6 } else { 3.6 * (-(rel - 1.0) * 1.4).exp() });
        screen.shake = screen.shake.max((-rel * 0.45).exp() * 1.3);
        screen.aberration = screen.aberration.max((-rel * 0.35).exp() * 1.4);
        screen.zoom_blur = screen.zoom_blur.max((-rel * 0.5).exp() * 0.8);
    }
    if (256.0..290.0).contains(&ct) {
        screen.zoom_blur = screen.zoom_blur.max(0.3);
        let swing = window(ct, 262.0, 263.0, 264.0, 265.0);
        screen.aberration = screen.aberration.max(swing * 0.9);
        screen.zoom_blur = screen.zoom_blur.max(swing * 0.5);
        let belt = window(ct, 266.0, 268.0, 275.0, 277.0);
        screen.shake = screen.shake.max(belt * 0.4);
        let moon = window(ct, 284.0, 285.0, 285.5, 287.0);
        screen.shake = screen.shake.max(moon * 0.5);
        screen.aberration = screen.aberration.max(moon * 0.6);
    }
}

// ---- SS-04 ----------------------------------------------------------------------------------------------------------

pub fn stars_cover(ct: f32) -> f32 {
    if (58.0..272.0).contains(&ct) { clamp01((ct - 58.0) / 12.0) } else { 0.0 }
}

/// The table of the Dipper turned nearest `turn` (radians): four are kept,
/// a quarter turn apart; the film is in space, where a few degrees don't show.
fn stars_table(turn: f64) -> &'static str {
    let q = (turn / std::f64::consts::FRAC_PI_2).round().rem_euclid(4.0) as usize;
    ["stars_space_0", "stars_space_1", "stars_space_2", "stars_space_3"][q]
}

pub fn stars_passes(passes: &mut Vec<Pass>, ct: f32, weight: f32, seed: f32, turn: f64) -> bool {
    let cover = stars_cover(ct) * weight;
    if cover > 0.001 {
        passes.extend(pass("stars_space", stars_table(turn), ct, Some("S0"), seed, Some(cover)));
    }
    cover >= 0.999
}

/// `SevenStarsFx.filmScreen`.
pub fn stars_screen(screen: &mut Screen, ct: f32, weight: f32) {
    let cover = stars_cover(ct) * weight;
    screen.synthetic = screen.synthetic.max(cover);
    if cover > 0.001 {
        screen.bloom = screen.bloom.max(0.6);
    }
    for jump in [96.0, 130.0] {
        let k = window(ct, jump - 1.0, jump, jump + 1.0, jump + 5.0);
        screen.aberration = screen.aberration.max(k * 0.9);
        screen.add_flash(0xD8CCFF, k * 0.25);
    }
    for i in 0..7 {
        let at = 150.0 + i as f32 * 6.0;
        let k = window(ct, at, at + 0.5, at + 1.0, at + 4.0);
        screen.shake = screen.shake.max(k * 0.35);
        screen.aberration = screen.aberration.max(k * 0.6);
        let f = 208.0 + i as f32 * 4.0;
        let kf = window(ct, f, f + 0.3, f + 0.6, f + 3.0);
        screen.add_flash(0xF0EAFF, kf * (0.6 + 0.2 * i as f32));
        screen.shake = screen.shake.max(kf * (0.4 + 0.1 * i as f32));
        screen.zoom_blur = screen.zoom_blur.max(kf * 0.5);
    }
    let ride = window(ct, 236.0, 238.0, 270.0, 272.0);
    screen.zoom_blur = screen.zoom_blur.max(ride * 0.4);
    let back = window(ct, 272.0, 272.2, 272.6, 275.0);
    screen.add_flash(0xFFFFFF, back * 1.2);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundle's own copy of the original's files, as the game would hand them over.
    pub(crate) fn provide_all() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../native-data/shooting_star/");
        for path in fx::ASSET_FILES {
            if let Ok(bytes) = std::fs::read(format!("{root}{path}")) {
                fx::provide(path, bytes);
            }
        }
    }

    #[test]
    fn every_film_table_reads() {
        provide_all();
        for path in fx::ASSET_FILES.iter().filter_map(|p| p.strip_prefix("films/")) {
            let name = path.trim_end_matches(".bin");
            let t = table(name).unwrap_or_else(|| panic!("{name} reads"));
            assert!(!t.tracks.is_empty(), "{name} has uniforms");
        }
    }

    #[test]
    fn a_track_is_a_line_between_its_samples() {
        let t = Track { name: "X", len: 2, times: vec![0.0, 10.0], values: vec![0.0, 1.0, 10.0, 3.0] };
        assert_eq!(t.at(5.0), vec![5.0, 2.0]);
        assert_eq!(t.at(-1.0), vec![0.0, 1.0]);
        assert_eq!(t.at(20.0), vec![10.0, 3.0]);
    }

    #[test]
    fn the_railgun_film_gives_the_space_shader_its_uniforms() {
        provide_all();
        let mut passes = Vec::new();
        let whole = star_passes(&mut passes, 150.0, 1.0, 0.25);
        assert!(whole);
        assert_eq!(passes.len(), 1);
        let p = &passes[0];
        assert_eq!(p.program, "star_space");
        let s0 = p.uniforms.iter().find(|(n, _)| *n == "S0").expect("S0");
        match &s0.1 {
            Value::F(v) => {
                assert_eq!(v[2], 0.25);
                assert_eq!(v[3], 1.0);
                // tan(fov / 2) of the climb's 56..62 degrees.
                assert!(v[0] > 0.5 && v[0] < 0.62, "{v:?}");
            }
            other => panic!("{other:?}"),
        }
        let planets = p.uniforms.iter().find(|(n, _)| *n == "Planet").expect("Planet");
        assert!(matches!(&planets.1, Value::Vec4s(v) if v.len() == 8));
        passes.clear();
        star_passes(&mut passes, 260.0, 1.0, 0.25);
        let names: Vec<_> = passes.iter().map(|p| p.program).collect();
        assert_eq!(names, ["star_gun", "star_rays"]);
    }
}
