// Each spell's light, frame by frame: the original's `StarFx`,
// `GungnirFx` and `SevenStarsFx` (`worldPass` and `screen`), which set its fx
// shaders' uniforms and the screen's shake, flash, bloom and impact frames on
// the spell's own clock. The window's spells go in, one `Frame` comes out for
// the render pass (`shooting_star_fxframe`).
//
// Plain functions of the spells and the camera: tested on their own.
use super::shooting_star_skills::{self as sk, Skill};
use super::shooting_star_films as films;
use super::shooting_star_gravitia as gravitia;
use horizon_client_render::gfx::native_render::shooting_star_fxframe::{Frame, Pass, Screen};

/// The spell's film as this frame sees it: its clock and how much it holds the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Film {
    pub ct: f32,
    pub weight: f32,
}

/// One spell as its light needs it.
#[derive(Clone, Copy, Debug)]
pub struct Spell {
    pub start: sk::Start,
    /// The spell's own ticks, with the partial tick and its speed in.
    pub t: f32,
    /// The tick Gungnir's sonic boom reached this player, once it has.
    pub boomed: Option<f32>,
    /// The film, while it plays.
    pub film: Option<Film>,
}

impl Spell {
    fn seed_f(&self) -> f32 {
        (self.start.seed & 0xFFFF) as f32 / 65535.0
    }
}

pub fn window(t: f32, in0: f32, in1: f32, out0: f32, out1: f32) -> f32 {
    ((t - in0) / (in1 - in0).max(0.001)).clamp(0.0, 1.0) * (1.0 - ((t - out0) / (out1 - out0).max(0.001)).clamp(0.0, 1.0))
}

pub fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The point of a zone's edge nearest the camera, as the original's `edgeToward`.
fn edge_toward(target: [f64; 3], from: [f64; 3], radius: f64) -> [f64; 3] {
    let dx = from[0] - target[0];
    let dz = from[2] - target[2];
    let d = dx.hypot(dz);
    if d < 0.001 {
        return target;
    }
    let k = d.min(radius) / d;
    [target[0] + dx * k, target[1] + (from[1] - target[1]).clamp(0.0, 80.0), target[2] + dz * k]
}

/// `impactSequence`: anime impact frames, one of `modes` each twelfth of a second.
fn impact_sequence(screen: &mut Screen, since: f32, strength: f32, step: f32, modes: &[i32]) {
    if since < 0.0 || strength <= 0.0 {
        return;
    }
    let i = (since / step) as usize;
    if let Some(mode) = modes.get(i)
        && *mode >= 0
    {
        screen.add_impact(strength, *mode, 0.5, 0.5);
    }
}

/// `stoppedWorld`: the world drained of colour for a breath.
fn stopped_world(screen: &mut Screen, k: f32) {
    if k > 0.001 {
        screen.desaturate = screen.desaturate.max(k);
        screen.grade[0] = screen.grade[0].max(0.3 * k);
        screen.grade[3] = screen.grade[3].max(k);
        screen.grade[1] = screen.grade[1].max(0.35 * k);
    }
}

fn pulse_phase(t: f64, span: f64, total: f64) -> f64 {
    if t < 29.0 {
        return 0.0;
    }
    let s = (t - 29.0).min(total) / 20.0;
    let k = s / span;
    0.6 * s + 3.4 * span * k * k * k / 3.0
}

/// What the light needs of the world beyond the spells and the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Context {
    pub world: gravitia::World,
    /// The camera's projection, for the impact frames' focus.
    pub proj: Option<gravitia::glam_like::Proj>,
    /// The camera is in Imaginary Space: its city's heading, for the sky.
    pub void: Option<(f64, f64)>,
}

impl Default for Context {
    fn default() -> Self {
        Context { world: gravitia::World { day_time: 18000, render_blocks: 192.0, banished: false }, proj: None, void: None }
    }
}

/// This frame's passes and screen for every spell playing, oldest first.
pub fn frame(spells: &[Spell], camera: [f64; 3], min_y: i32, max_y: i32, time: f32) -> Frame {
    frame_in(spells, camera, min_y, max_y, time, &Context::default())
}

/// The same, knowing the world.
pub fn frame_in(spells: &[Spell], camera: [f64; 3], min_y: i32, max_y: i32, time: f32, context: &Context) -> Frame {
    let mut frame = Frame { time, ..Frame::default() };
    for spell in spells {
        match spell.start.skill {
            Skill::ShootingStar => star(&mut frame, spell, camera, min_y),
            Skill::Gungnir => gungnir(&mut frame, spell, camera, max_y),
            Skill::SevenStars => seven(&mut frame, spell, camera),
            Skill::Tesseract => tess(&mut frame, spell, camera, min_y, max_y),
            Skill::Gravitia => gravitia::frame(&mut frame, spell, camera, context.world, context.proj.as_ref()),
        }
    }
    if let Some(heading) = context.void {
        // Imaginary Space's sky: space for good.
        gravitia::void_sky(&mut frame, heading);
    }
    frame
}

// ---- SS-01 ----------------------------------------------------------------------------------------------------------

fn star(frame: &mut Frame, spell: &Spell, camera: [f64; 3], min_y: i32) {
    let t = spell.t;
    let start = &spell.start;
    let r = start.power;
    let mut space = Vec::new();
    let mut whole = false;
    if let Some(film) = spell.film {
        films::star_screen(&mut frame.screen, film.ct, film.weight);
        whole = films::star_passes(&mut space, film.ct, film.weight, spell.seed_f());
    }
    if !whole && (8.0..=1386.0).contains(&t) {
        let laser = window(t, 9.0, 10.5, 352.0, 353.0);
        let incoming = if t < 352.0 { smooth((t - 308.0) / 44.0) } else { 0.0 };
        let perimeter = window(t, 15.0, 39.0, 351.0, 353.0);
        let beam = window(t, 352.0, 352.5, 485.0, 487.0);
        let since = if t >= 352.0 { (t - 352.0) / 20.0 } else { -1.0 };
        let collapse = ((t - 462.0) / 24.0).clamp(0.0, 1.0);
        let heat = if t < 474.0 { 0.0 } else { smooth((t - 486.0 + 12.0) / 12.0) * (-(t - 486.0).max(0.0) / 420.0).exp() };
        let reticle = window(t, 9.0, 21.0, 332.0, 352.0);
        let pulses = pulse_phase(f64::from(t), 16.15, 323.0) as f32;
        let radius = sk::beam_radius(f64::from(t), f64::from(r)) as f32;
        let floor_rel = (f64::from(min_y) - camera[1]) as f32;
        let seconds = t / 20.0;
        frame.passes.push(
            Pass::new("star_beam")
                .at("Target", start.target)
                .f("P0", &[radius, seconds, spell.seed_f(), laser])
                .f("P1", &[laser, perimeter, incoming, beam])
                .f("P2", &[since, collapse, heat, r])
                .f("P3", &[reticle, pulses, floor_rel, (start.target[1] - camera[1]) as f32]),
        );
    }
    frame.passes.append(&mut space);
    if spell.film.is_some_and(|f| films::star_synthetic(f.ct)) {
        return;
    }
    let screen = &mut frame.screen;
    let wall = distance(camera, edge_toward(start.target, camera, f64::from(r)));
    let near = (1.0 - wall / 900.0).clamp(0.15, 1.0) as f32;
    if t > 308.0 && t < 352.0 {
        let k = smooth((t - 352.0 + 44.0) / 44.0);
        screen.add_vignette(0x3A0006, k * 0.4 * near);
        screen.shake = screen.shake.max(k * k * 0.35 * near);
        screen.aberration = screen.aberration.max(k * k * 0.3 * near);
    }
    let since = (t - 352.0) / 20.0;
    if since >= 0.0 && t < 506.0 {
        impact_sequence(screen, since, near, 0.05, &[7, 7, 8, 7, 8, 8]);
        screen.shake = screen.shake.max(window(since, 0.0, 0.05, 3.5, 6.0) * 0.9 * near);
        screen.aberration = screen.aberration.max(window(since, 0.0, 0.05, 0.6, 3.0) * near);
        screen.zoom_blur = screen.zoom_blur.max(window(since, 0.0, 0.03, 0.35, 1.4) * 0.55 * near);
        let hold = window(t, 352.0, 358.0, 482.0, 490.0);
        screen.bloom = screen.bloom.max(1.4 * hold);
        screen.bloom_radius = screen.bloom_radius.max(1.0 + 1.2 * hold);
        screen.add_vignette(0x4A0008, hold * 0.35 * near);
        screen.shake = screen.shake.max(hold * 0.22 * near);
        screen.aberration = screen.aberration.max(hold * 0.12 * near);
        let snap = window(t, 483.0, 486.0, 488.0, 502.0);
        screen.add_flash(0xFF6070, snap * 1.2 * near);
        screen.shake = screen.shake.max(snap * 0.5 * near);
    }
}

// ---- SS-05 ----------------------------------------------------------------------------------------------------------

fn tess(frame: &mut Frame, spell: &Spell, camera: [f64; 3], min_y: i32, max_y: i32) {
    use super::shooting_star_tesseract as tess;
    let sq = tess::Square::new(&spell.start, min_y, max_y);
    let t = spell.t;
    let cover = spell.film.map_or(0.0, |f| tess::cover(f.ct) * f.weight);
    if cover < 0.999 {
        tess::passes(frame, &sq, t, spell.seed_f(), camera);
    }
    if let Some(film) = spell.film {
        tess::film(frame, film.ct, film.weight, spell.seed_f());
    }
    if spell.film.is_some_and(|f| tess::cover(f.ct) > 0.5) {
        return;
    }
    // The nearest point of the square's box, as the original's `edgeToward`.
    let edge = [
        camera[0].clamp(f64::from(sq.ox), f64::from(sq.ox + sq.zone)),
        camera[1].clamp(f64::from(sq.ground), f64::from(sq.engine_y(t)).max(f64::from(sq.ground))),
        camera[2].clamp(f64::from(sq.oz), f64::from(sq.oz + sq.zone)),
    ];
    let near = if spell.film.is_some() { 1.0 } else { (1.0 - distance(camera, edge) / 1200.0).clamp(0.2, 1.0) as f32 };
    tess::screen(&mut frame.screen, &sq, t, near);
}

// ---- SS-03 ----------------------------------------------------------------------------------------------------------

const SILENT: [i32; 14] = [17, 20, 15, 18, 19, 20, 16, 0, 18, 17, 20, 6, 15, 19];
const RINGING: [i32; 11] = [20, 16, 18, 20, 15, 19, 17, 20, 1, 18, 15];
const DRAWING: f32 = 1.0 / 12.0;

/// The tick Gungnir's shock front reaches a distance.
pub fn gungnir_reached_at(distance: f64, radius: f64) -> f64 {
    322.0 + 36.0 * (distance.max(0.0) / radius.max(1.0)).powf(1.666_666_7)
}

fn gungnir(frame: &mut Frame, spell: &Spell, camera: [f64; 3], max_y: i32) {
    let t = spell.t;
    let start = &spell.start;
    let scale = sk::gungnir_scale(start.power);
    let radius = sk::gungnir_radius(start.power);
    let spire_r = 6.5 * scale.sqrt().min(1.0);
    let mut space = Vec::new();
    let mut whole = false;
    if let Some(film) = spell.film {
        films::gungnir_screen(&mut frame.screen, film.ct, film.weight);
        whole = films::gungnir_passes(&mut space, film.ct, film.weight, spell.seed_f());
    }
    if !whole && (8.0..=1222.0).contains(&t) {
        let mark = window(t, 9.0, 10.5, 300.0, 318.0);
        let incoming = if (300.0..322.0).contains(&t) { (t - 300.0) / 22.0 } else { 0.0 };
        let since = if t >= 322.0 { (t - 322.0) / 20.0 } else { -1.0 };
        let front = sk::gungnir_front(f64::from(t), radius) as f32;
        let dust = window(t, 323.0, 328.0, 522.0, 642.0);
        let reticle = window(t, 9.0, 21.0, 292.0, 300.0);
        let pulses = pulse_phase(f64::from(t), 13.05, 261.0) as f32;
        let heat = if t < 322.0 { 0.0 } else { (-(t - 322.0) / 700.0).exp() * smooth((t - 322.0) / 3.0) };
        let spire = if t < 325.0 { 0.0 } else { window(t, 325.0, 352.0, 1142.0, 1222.0) };
        let trail = if t < 322.0 { 0.0 } else { smooth((t - 322.0) / 4.0) * (-(t - 322.0) / 160.0).exp() };
        let haze = window(t, 324.0, 342.0, 822.0, 1122.0);
        let telemetry = if t < 322.0 { 0.0 } else { window(t, 322.0, 322.5, 1142.0, 1222.0) };
        let shells = if t < 322.0 { 0.0 } else { window(t, 322.0, 322.5, 392.0, 432.0) };
        let schematic = if t < 322.0 { 0.0 } else { window(t, 322.4, 323.6, 338.0, 352.0) };
        let top = (f64::from(max_y) - camera[1]) as f32;
        let bottom = (start.target[1].floor() - 1.0 - 108.0 * scale - camera[1]) as f32;
        let target_rel_y = (start.target[1] - camera[1]) as f32;
        frame.passes.push(
            Pass::new("gungnir_world")
                .at("Target", start.target)
                .f("P0", &[t / 20.0, spell.seed_f(), mark, incoming])
                .f("P1", &[since, front, radius as f32, dust])
                .f("P2", &[reticle, pulses, target_rel_y, heat])
                .f("P3", &[spire, trail, 0.0, haze])
                .f("P4", &[top, bottom, spire_r as f32, 1.0])
                .f("P5", &[telemetry, shells, schematic, scale as f32]),
        );
    }
    frame.passes.append(&mut space);
    if spell.film.is_some_and(|f| films::gungnir_cover(f.ct) > 0.5) {
        return;
    }
    let screen = &mut frame.screen;
    let wall = distance(camera, edge_toward(start.target, camera, radius));
    let near = if spell.film.is_some() { 1.0 } else { (1.0 - wall / 1200.0).clamp(0.2, 1.0) as f32 };
    if t > 300.0 && t < 322.0 {
        let k = smooth((t - 322.0 + 22.0) / 22.0);
        screen.add_vignette(0x0A0200, k * 0.55 * near);
        screen.aberration = screen.aberration.max(k * k * 0.35 * near);
        screen.zoom_blur = screen.zoom_blur.max(k * k * k * 0.25 * near);
    }
    let since = (t - 322.0) / 20.0;
    if (0.0..6.0).contains(&since) {
        let burst = since - 0.05;
        if burst >= 0.0 && burst < SILENT.len() as f32 * DRAWING {
            impact_sequence(screen, burst, near, DRAWING, &SILENT);
            screen.step(12.0);
            let drawing = (burst / DRAWING) as i32;
            let into = burst / DRAWING - drawing as f32;
            if drawing % 2 == 0 {
                screen.zoom_blur = screen.zoom_blur.max((1.0 - into) * 0.9 * near);
            } else {
                screen.aberration = screen.aberration.max((1.0 - into) * 1.4 * near);
            }
        }
        screen.zoom_blur = screen.zoom_blur.max(window(since, 0.0, 0.02, 0.4, 1.4) * 0.7 * near);
        screen.aberration = screen.aberration.max(window(since, 0.0, 0.02, 0.5, 2.0) * near);
        screen.bloom = screen.bloom.max(1.2 * window(since, 0.0, 0.1, 2.0, 5.0));
        screen.bloom_radius = screen.bloom_radius.max(1.0 + 1.2 * window(since, 0.0, 0.1, 1.0, 3.0));
        if spell.boomed.is_none() && since > SILENT.len() as f32 * DRAWING {
            stopped_world(screen, window(since, 0.9, 1.1, 6.0, 6.0) * 0.6 * near);
        }
    }
    if let Some(boomed) = spell.boomed {
        let b = (t - boomed) / 20.0;
        if (0.0..8.0).contains(&b) {
            if b < RINGING.len() as f32 * DRAWING {
                impact_sequence(screen, b, near, DRAWING, &RINGING);
                screen.step(12.0);
                let drawing = (b / DRAWING) as i32;
                screen.zoom_blur = screen.zoom_blur.max(if drawing % 2 == 0 { near } else { 0.3 * near });
            }
            screen.shake = screen.shake.max(window(b, 0.0, 0.02, 0.8, 3.5) * 2.4 * near);
            screen.aberration = screen.aberration.max(window(b, 0.0, 0.02, 0.3, 1.6) * 1.3 * near);
            screen.zoom_blur = screen.zoom_blur.max(window(b, 0.0, 0.02, 0.2, 0.9) * 0.6 * near);
            let dust = window(b, 0.2, 1.0, 4.0, 8.0);
            screen.add_vignette(0x2A1A10, dust * 0.45 * near);
            screen.grade[1] = screen.grade[1].max(0.25 * dust * near);
        }
    }
    if since >= 0.0 {
        screen.bloom = screen.bloom.max(0.8 * window(since, 0.5, 3.0, 20.0, 40.0));
    }
}

// ---- SS-04 ----------------------------------------------------------------------------------------------------------

const LANDING: [i32; 6] = [23, 21, 26, 24, 22, 25];
const FINALE_BURST: [i32; 16] = [23, 21, 26, 24, 25, 22, 23, 1, 21, 26, 24, 6, 25, 22, 21, 26];

fn fall(t: f32, i: usize) -> f32 {
    let land = sk::dipper_land_tick(i) as f32;
    if t >= land { 0.0 } else { ((t - (land - 16.0)) / 16.0).clamp(0.0, 1.0) }
}

fn lock(t: f32, i: usize) -> f32 {
    smooth((t - (24.0 + i as f32 * 4.0)) / 4.0)
}

/// The Dipper's stars where they land, at the target's height.
pub fn dipper_nodes(start: &sk::Start) -> [[f64; 3]; 7] {
    let scale = f64::from(start.power);
    let turn = sk::dipper_turn(start.origin[0], start.origin[2], start.target[0], start.target[2]);
    let mut out = [[0.0; 3]; 7];
    for (i, node) in out.iter_mut().enumerate() {
        let n = sk::dipper_node(i, turn, scale);
        *node = [start.target[0] + n[0], start.target[1], start.target[2] + n[1]];
    }
    out
}

fn seven(frame: &mut Frame, spell: &Spell, camera: [f64; 3]) {
    let t = spell.t;
    let start = &spell.start;
    let k = f64::from(start.power);
    let nodes = dipper_nodes(start);
    let mut space = Vec::new();
    let mut whole = false;
    if let Some(film) = spell.film {
        films::stars_screen(&mut frame.screen, film.ct, film.weight);
        let turn = sk::dipper_turn(start.origin[0], start.origin[2], start.target[0], start.target[2]);
        whole = films::stars_passes(&mut space, film.ct, film.weight, spell.seed_f(), turn);
    }
    if !whole && (8.0..=1340.0).contains(&t) {
        let mut pts = vec![[0.0f32; 4]; 16];
        for i in 0..7 {
            let land = sk::dipper_land_tick(i) as f32;
            let n = nodes[i];
            pts[i] = [
                (n[0] - camera[0]) as f32,
                (n[1] - camera[1]) as f32,
                (n[2] - camera[2]) as f32,
                if t >= land { (t - land) / 20.0 } else { -1.0 },
            ];
            pts[7 + i] = [
                sk::dipper_crater_radius(i, k) as f32,
                sk::dipper_crater_depth(i, k) as f32,
                fall(t, i),
                lock(t, i) * if t > 1040.0 { 0.0 } else { 1.0 },
            ];
        }
        let mark = window(t, 9.0, 10.5, 260.0, 290.0);
        let night = window(t, 18.0, 38.0, 740.0, 940.0);
        let link = if t >= 398.0 { (t - 398.0) / 20.0 } else { -1.0 };
        let finale = if t >= 440.0 { (t - 440.0) / 20.0 } else { -1.0 };
        let brackets = smooth((t - 9.0) / 14.0);
        let cores = if t < 440.0 { 0.0 } else { window(t, 460.0, 500.0, 740.0, 940.0) };
        frame.passes.push(
            Pass::new("stars_world")
                .vec4s("Pts", pts)
                .at("Target", start.target)
                .f("P0", &[t / 20.0, spell.seed_f(), mark, night])
                .f("P1", &[link, finale, (900.0 * k) as f32, 0.0])
                .f("P2", &[brackets, pulse_phase(f64::from(t), 13.05, 261.0) as f32, (start.target[1] - camera[1]) as f32, k as f32])
                .f("P3", &[0.0, 0.0, cores, 0.0]),
        );
    }
    frame.passes.append(&mut space);
    if spell.film.is_some_and(|f| films::stars_cover(f.ct) > 0.5) {
        return;
    }
    let film = spell.film.is_some();
    let screen = &mut frame.screen;
    let night = window(t, 24.0, 44.0, 640.0, 840.0);
    screen.add_vignette(0x0A0620, night * 0.3);
    for (i, node) in nodes.iter().enumerate() {
        let near = if film { 1.0 } else { (1.0 - distance(camera, *node) / (1200.0 * k)).clamp(0.2, 1.0) as f32 };
        let f = fall(t, i);
        if f > 0.0 {
            screen.aberration = screen.aberration.max(f * f * 0.5 * near);
            screen.zoom_blur = screen.zoom_blur.max(f * f * f * 0.35 * near);
        }
        let s = (t - sk::dipper_land_tick(i) as f32) / 20.0;
        if (0.0..2.5).contains(&s) {
            if s < 0.05 {
                screen.add_flash(0xFFFFFF, 4.5 * near);
            } else if s - 0.05 < LANDING.len() as f32 * DRAWING {
                let order: Vec<i32> = (0..LANDING.len()).map(|j| LANDING[(j + i) % LANDING.len()]).collect();
                impact_sequence(screen, s - 0.05, near, DRAWING, &order);
                screen.step(12.0);
                let drawing = ((s - 0.05) / DRAWING) as i32;
                screen.zoom_blur = screen.zoom_blur.max(if drawing % 2 == 0 { 0.9 } else { 0.25 } * near);
            }
            screen.add_flash(0xD8C8FF, (-s * 5.0).exp() * near);
            screen.shake = screen.shake.max(window(s, 0.0, 0.02, 0.5, 2.0) * 1.1 * near);
            screen.aberration = screen.aberration.max(window(s, 0.0, 0.02, 0.4, 1.5) * 1.1 * near);
            screen.bloom = screen.bloom.max(1.1 * window(s, 0.0, 0.1, 1.0, 2.5));
        }
    }
    let link = window(t, 398.0, 402.0, 436.0, 440.0);
    screen.shake = screen.shake.max(link * 0.25);
    screen.bloom = screen.bloom.max(link * 0.55);
    stopped_world(screen, window(t, 432.0, 435.0, 439.0, 440.0) * 0.6);
    let fs = (t - 440.0) / 20.0;
    if (0.0..8.0).contains(&fs) {
        if fs < 0.06 {
            screen.add_flash(0xFFFFFF, 6.0);
        } else if fs - 0.06 < FINALE_BURST.len() as f32 * DRAWING {
            impact_sequence(screen, fs - 0.06, 1.0, DRAWING, &FINALE_BURST);
            screen.step(12.0);
            let drawing = ((fs - 0.06) / DRAWING) as i32;
            screen.zoom_blur = screen.zoom_blur.max(if drawing % 2 == 0 { 1.0 } else { 0.3 });
            screen.aberration = screen.aberration.max(if drawing % 2 == 0 { 0.4 } else { 1.6 });
        }
        screen.add_flash(0xC8B8FF, (-fs * 2.5).exp() * 1.4);
        screen.shake = screen.shake.max(window(fs, 0.0, 0.02, 1.0, 4.0) * 1.8);
        screen.bloom = screen.bloom.max(1.5 * window(fs, 0.0, 0.1, 3.0, 8.0));
        screen.bloom_radius = screen.bloom_radius.max(1.0 + 1.2 * window(fs, 0.0, 0.1, 2.0, 6.0));
        screen.grade[1] = screen.grade[1].max(0.2 * window(fs, 0.5, 1.0, 3.0, 6.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(skill: Skill, power: f32) -> sk::Start {
        sk::Start { skill, mine: true, origin: [0.0, 70.0, -50.0], target: [0.0, 64.0, 0.0], yaw: 0.0, seed: 7, power, speed: 1.0 }
    }

    #[test]
    fn windows_open_and_close() {
        assert_eq!(window(5.0, 9.0, 10.5, 352.0, 353.0), 0.0);
        assert_eq!(window(100.0, 9.0, 10.5, 352.0, 353.0), 1.0);
        assert_eq!(window(353.0, 9.0, 10.5, 352.0, 353.0), 0.0);
    }

    #[test]
    fn the_railgun_lays_its_beam_down_at_the_impact() {
        let s = Spell { start: start(Skill::ShootingStar, 200.0), t: 400.0, boomed: None, film: None };
        let f = frame(&[s], [0.0, 90.0, -100.0], -64, 319, 20.0);
        assert_eq!(f.passes.len(), 1);
        assert_eq!(f.passes[0].program, "star_beam");
        assert!(f.screen.bloom > 1.0);
        let early = frame(&[Spell { t: 3.0, ..s }], [0.0, 90.0, -100.0], -64, 319, 0.1);
        assert!(early.passes.is_empty());
    }

    #[test]
    fn the_stars_pass_carries_all_seven() {
        let s = Spell { start: start(Skill::SevenStars, 1.0), t: 300.0, boomed: None, film: None };
        let f = frame(&[s], [0.0, 90.0, -100.0], -64, 319, 15.0);
        let pts = f.passes[0].uniforms.iter().find(|(n, _)| *n == "Pts").expect("Pts");
        match &pts.1 {
            horizon_client_render::gfx::native_render::shooting_star_fxframe::Value::Vec4s(v) => assert_eq!(v.len(), 16),
            _ => panic!("Pts is a vec4 array"),
        }
    }
}
