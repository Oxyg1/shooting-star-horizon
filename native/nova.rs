// SS-H2 Supernova: the port's own skill, not the original's. Its light frame
// by frame (the `hz_nova` pass: the star, its shell, the remnant, the ground
// front, the burnt land, the night made day), its lights and fronts for the
// shared passes, and its film, a camera in the world.
//
// Timeline (ticks at speed 1, `shooting_star_skills::SN_*`):
//   30..60   a star is lit 300 blocks over the target, a glint
//   60..220  it swells into a red giant, pulsing harder
//   220..250 it falls in on itself to a blue-white point, the world drained
//   250      it explodes: white, impact frames, a shell of light going out
//   262      the blast reaches the ground: a front sweeps out in a ring
//   250..    the night is day for a minute; the remnant glows, its pulsar turning
use super::shooting_star_cutscenes::{self as cs, Pose, Shot};
use super::shooting_star_skills::{self as sk, SN_BLAST, SN_HIT, SN_LIGHT, SN_PEAK, SN_SWELL};
use super::shooting_star_visuals::{self as vis, Spell, window};
use horizon_client_render::gfx::native_render::shooting_star_fxframe::{Frame, Pass, Screen};

const DRAWING: f32 = 1.0 / 12.0;
/// The blast's impact frames: the azure ones, and a white one.
const BLAST_FRAMES: [i32; 9] = [36, 41, 1, 37, 40, 38, 33, 39, 36];

fn k_of(t: f32, from: i32, to: i32) -> f32 {
    ((t - from as f32) / (to - from) as f32).clamp(0.0, 1.0)
}

/// The star's radius in blocks.
pub fn star_radius(t: f32) -> f32 {
    if t < SN_LIGHT as f32 || t >= SN_BLAST as f32 {
        return 0.0;
    }
    if t < SN_SWELL as f32 {
        return 0.8 + 2.2 * vis::smooth(k_of(t, SN_LIGHT, SN_SWELL));
    }
    let peak = 42.0;
    if t < SN_PEAK as f32 {
        let k = k_of(t, SN_SWELL, SN_PEAK);
        let breathe = 1.0 + 0.05 * k * ((t - SN_SWELL as f32) * std::f32::consts::TAU / 25.0).sin();
        return (3.0 + (peak - 3.0) * k.powf(1.3)) * breathe;
    }
    let k = k_of(t, SN_PEAK, SN_BLAST);
    (peak * (1.0 - k).powi(3)).max(0.4)
}

/// Its colour: 0 blue-white, 1 a red giant's.
fn temperature(t: f32) -> f32 {
    if t < SN_SWELL as f32 {
        return 0.12;
    }
    if t < SN_PEAK as f32 {
        return 0.12 + 0.88 * k_of(t, SN_SWELL, SN_PEAK).powf(0.8);
    }
    1.0 - k_of(t, SN_PEAK, SN_PEAK + 14)
}

/// Its pulse: a throb every 25 ticks, harder as it swells.
fn pulse(t: f32) -> f32 {
    if !(SN_SWELL as f32..SN_PEAK as f32).contains(&t) {
        return 0.0;
    }
    let p = ((t - SN_SWELL as f32) * std::f32::consts::TAU / 25.0).sin().max(0.0).powi(6);
    p * k_of(t, SN_SWELL, SN_PEAK)
}

/// The shell of light thrown out by the blast (blocks).
fn shell_radius(t: f32) -> f32 {
    let s = t - SN_BLAST as f32;
    if s < 0.0 {
        return 0.0;
    }
    if s < 12.0 { 25.0 * s } else { 300.0 + 160.0 * (1.0 - (-(s - 12.0) / 120.0).exp()) }
}

/// What a glint the star makes: a point when small, blinding as it goes.
fn glint(t: f32) -> f32 {
    let r = star_radius(t);
    if (SN_LIGHT as f32..SN_SWELL as f32).contains(&t) {
        return k_of(t, SN_LIGHT, SN_LIGHT + 8) * 1.2;
    }
    if (SN_SWELL as f32..SN_BLAST as f32).contains(&t) {
        let small = 1.0 - vis::smooth((r - 3.0) / 10.0);
        let collapse = k_of(t, SN_PEAK, SN_BLAST);
        return small * 0.8 + collapse * collapse * 2.5;
    }
    let s = t - SN_BLAST as f32;
    if (0.0..120.0).contains(&s) { 5.0 * (-s / 15.0).exp() } else { 0.0 }
}

fn day(t: f32) -> f32 {
    let b = SN_BLAST as f32;
    window(t, b, b + 15.0, b + 900.0, b + 1300.0)
}

fn nearness(camera: [f64; 3], target: [f64; 3], radius: f64) -> f32 {
    let d = (camera[0] - target[0]).hypot(camera[2] - target[2]);
    (1.0 - (d - radius).max(0.0) / (radius * 3.0 + 900.0)).clamp(0.35, 1.0) as f32
}

/// The light of a Supernova this frame.
pub fn frame(frame: &mut Frame, spell: &Spell, camera: [f64; 3]) {
    let t = spell.t;
    let start = &spell.start;
    if !(8.0..=start.skill.fx_duration() as f32).contains(&t) {
        return;
    }
    let power = start.power;
    let radius = sk::sn_radius(power);
    let star = sk::sn_star(start.target);
    let ground = start.target;
    let s = t - SN_BLAST as f32;
    let ring = sk::sn_ring(f64::from(t), power) as f32;
    let ring_end = SN_HIT as f32 + sk::sn_ring_ticks(power) as f32;
    let ring_k = window(t, SN_HIT as f32, SN_HIT as f32 + 2.0, ring_end, ring_end + 15.0);
    let shell = shell_radius(t);
    let shell_k = if s >= 0.0 { (-s / 40.0).exp() } else { 0.0 };
    let nebula = if s >= 0.0 { window(s, 10.0, 60.0, 1.0e9, 1.0e9) * (-(s - 60.0).max(0.0) / 900.0).exp() } else { 0.0 };
    let ember = if t >= SN_HIT as f32 { (-(t - SN_HIT as f32) / 600.0).exp() } else { 0.0 };
    let flash = if (0.0..60.0).contains(&s) { (-s / 8.0).exp() } else { 0.0 };
    let implode = k_of(t, SN_PEAK, SN_BLAST);
    let seconds = t / 20.0;
    frame.passes.push(
        Pass::new("hz_nova")
            .at("Star", star)
            .at("Ground", ground)
            .f("P0", &[star_radius(t), temperature(t), seconds, pulse(t)])
            .f("P1", &[shell, shell_k, ring, ring_k])
            .f("P2", &[ring, day(t), nebula, ember])
            .f("P3", &[radius as f32, implode, flash, glint(t)])
            .f("Seed", &[spell.seed_f()]),
    );
    vis::focus_on(star);
    screen(&mut frame.screen, t, nearness(camera, ground, radius), camera, ground, power);
}

fn screen(screen: &mut Screen, t: f32, near: f32, camera: [f64; 3], ground: [f64; 3], power: f32) {
    // The swell: the edges glow red with it, each throb shakes the picture.
    if (SN_SWELL as f32..SN_PEAK as f32).contains(&t) {
        let k = k_of(t, SN_SWELL, SN_PEAK);
        screen.add_vignette(0x3A0A00, k * 0.4 * near);
        screen.bloom = screen.bloom.max(0.6 + 0.6 * k);
        screen.bloom_radius = screen.bloom_radius.max(1.0 + k);
        let p = pulse(t);
        screen.shake = screen.shake.max(p * 0.35 * near);
        screen.aberration = screen.aberration.max(p * 0.4 * near);
        screen.grade[0] = screen.grade[0].max(0.12 * k);
    }
    // The fall inward: colour drained, the picture pulled in.
    if (SN_PEAK as f32..SN_BLAST as f32).contains(&t) {
        let k = k_of(t, SN_PEAK, SN_BLAST);
        vis::stopped_world(screen, vis::smooth(k * 2.5) * 0.85 * near);
        screen.zoom_blur = screen.zoom_blur.max(k * k * 0.9 * near);
        screen.aberration = screen.aberration.max(k * 1.4 * near);
        screen.add_vignette(0x000000, k * 0.5);
    }
    // The blast.
    let s = (t - SN_BLAST as f32) / 20.0;
    if (0.0..12.0).contains(&s) {
        if s < 0.07 {
            screen.add_flash(0xFFFFFF, 7.0 * near);
        } else if s - 0.07 < BLAST_FRAMES.len() as f32 * DRAWING {
            vis::impact_sequence(screen, s - 0.07, near, DRAWING, &BLAST_FRAMES);
            screen.step(12.0);
            let drawing = ((s - 0.07) / DRAWING) as i32;
            screen.zoom_blur = screen.zoom_blur.max(if drawing % 2 == 0 { 1.0 } else { 0.25 } * near);
            screen.aberration = screen.aberration.max(if drawing % 2 == 0 { 0.4 } else { 1.6 } * near);
        }
        screen.add_flash(0xD8ECFF, (-s * 3.2).exp() * 1.4 * near);
        screen.shake = screen.shake.max(window(s, 0.0, 0.02, 1.0, 4.0) * 1.2 * near);
        // Short: the day after must not bloom (the bloom takes all the light a pass adds).
        screen.bloom = screen.bloom.max(1.8 * window(s, 0.0, 0.1, 0.6, 2.5));
        screen.bloom_radius = screen.bloom_radius.max(1.0 + 1.6 * window(s, 0.0, 0.1, 1.0, 3.0));
    }
    // The ground front: hardest when it passes the camera.
    if t >= SN_HIT as f32 {
        let ring = sk::sn_ring(f64::from(t), power);
        let d = (camera[0] - ground[0]).hypot(camera[2] - ground[2]);
        let gap = (d - ring).abs() as f32;
        let passing = (1.0 - gap / 30.0).clamp(0.0, 1.0) * (t < SN_HIT as f32 + sk::sn_ring_ticks(power) as f32 + 10.0) as i32 as f32;
        screen.shake = screen.shake.max(passing * passing * 2.2);
        screen.aberration = screen.aberration.max(passing * 1.2);
        screen.add_vignette(0x4A2A12, passing * 0.55);
        screen.grade[1] = screen.grade[1].max(0.3 * passing);
    }
    // The day after: warm and bright.
    let d = day(t);
    if d > 0.0 {
        screen.grade[3] = screen.grade[3].max(0.25 * d);
        screen.bloom = screen.bloom.max(0.12 * d);
    }
}

fn star_colour(temp: f32) -> [f32; 3] {
    let stops = [[0.62, 0.78, 1.0], [1.0, 0.96, 0.88], [1.0, 0.78, 0.42], [1.0, 0.38, 0.16]];
    let x = (temp.clamp(0.0, 1.0) * 2.999).min(2.999);
    let (i, f) = (x as usize, x.fract());
    [0, 1, 2].map(|c| stops[i][c] + (stops[i + 1][c] - stops[i][c]) * f)
}

/// Its lights for `hz_glow`.
pub fn lights(spell: &Spell, camera: [f64; 3], out: &mut Vec<vis::Light>) {
    let t = spell.t;
    let start = &spell.start;
    let star = sk::sn_star(start.target);
    let ground = start.target;
    let radius = sk::sn_radius(start.power) as f32;
    let r = star_radius(t);
    if r > 0.0 {
        let c = star_colour(temperature(t));
        let k = (0.15 + 0.7 * (r / 42.0)) * (1.0 + pulse(t) * 0.6);
        out.push(vis::Light { at: star, reach: 700.0, color: c.map(|v| v * k), tall: 0.0 });
    }
    let s = t - SN_BLAST as f32;
    if (0.0..200.0).contains(&s) {
        let k = 2.2 * (-s / 12.0).exp();
        out.push(vis::Light { at: star, reach: 1400.0, color: [0.85 * k, 0.92 * k, 1.0 * k], tall: 0.0 });
    }
    let ring = sk::sn_ring(f64::from(t), start.power);
    let ring_end = SN_HIT as f32 + sk::sn_ring_ticks(start.power) as f32;
    let ring_k = window(t, SN_HIT as f32, SN_HIT as f32 + 2.0, ring_end, ring_end + 15.0);
    if ring_k > 0.0 {
        // The front's light where it is nearest the camera.
        let (dx, dz) = (camera[0] - ground[0], camera[2] - ground[2]);
        let d = dx.hypot(dz).max(1.0);
        let at = [ground[0] + dx / d * ring, ground[1] + 3.0, ground[2] + dz / d * ring];
        out.push(vis::Light { at, reach: 34.0, color: [5.0 * ring_k, 3.2 * ring_k, 1.6 * ring_k], tall: 0.0 });
    }
    if t >= SN_HIT as f32 {
        let k = (-(t - SN_HIT as f32) / 600.0).exp();
        out.push(vis::Light { at: [ground[0], ground[1] + 2.0, ground[2]], reach: radius * 0.45, color: [1.6 * k, 0.5 * k, 0.12 * k], tall: 0.0 });
    }
}

/// Its fronts and heat for `hz_shock`.
pub fn fronts(spell: &Spell, shocks: &mut Vec<vis::Front>, heat: &mut Vec<vis::Front>) {
    let t = spell.t;
    let start = &spell.start;
    let ground = start.target;
    let radius = sk::sn_radius(start.power) as f32;
    let ring = sk::sn_ring(f64::from(t), start.power) as f32;
    let ring_end = SN_HIT as f32 + sk::sn_ring_ticks(start.power) as f32;
    let ring_k = window(t, SN_HIT as f32, SN_HIT as f32 + 2.0, ring_end, ring_end + 15.0);
    if ring_k > 0.0 {
        shocks.push(vis::Front { at: ground, radius: ring, strength: 1.4 * ring_k, thick: 4.0 });
    }
    let s = t - SN_BLAST as f32;
    if (0.0..60.0).contains(&s) {
        shocks.push(vis::Front { at: sk::sn_star(ground), radius: shell_radius(t), strength: 0.6 * (1.0 - s / 60.0), thick: 14.0 });
    }
    if t >= SN_HIT as f32 {
        heat.push(vis::Front { at: ground, radius: radius * 0.4, strength: (-(t - SN_HIT as f32) / 500.0).exp(), thick: 0.0 });
    }
}

/// Its film: a camera on the ground, looking up at the star, then down at the front.
pub fn cutscene(start: &sk::Start) -> cs::Cutscene {
    let power = start.power;
    let radius = sk::sn_radius(power);
    let above = |g: &dyn cs::Ground, mut p: cs::V, margin: f64| {
        if let Some(top) = g.top(p[0], p[2]) {
            p[1] = p[1].max(top + margin);
        }
        p
    };
    let mut shots: Vec<Shot> = Vec::new();
    shots.push(Shot::cut(0, 9, |x, _, s| Pose {
        position: x.frame(x.feet, 1.35 - 0.15 * f64::from(s), 1.7 - 0.1 * f64::from(s), 2.1 - 0.25 * f64::from(s)),
        look: x.frame(x.feet, 0.36, 1.38, 0.62),
        fov: 40.0,
        roll: 0.0,
    }));
    // Up at the sky where it is lit.
    shots.push(Shot::cut(9, 64, move |x, g, s| {
        let position = above(g, x.frame(x.feet, 1.0, 1.8, -2.5), 1.5);
        let star = sk::sn_star(x.target);
        Pose { position, look: star, fov: 58.0 - 10.0 * s, roll: 0.0 }
    }));
    // The swell, from out on the land: the giant over the hills.
    shots.push(Shot::sweep(64, 150, 8, move |x, g, s| {
        let a = (200.0 + 25.0 * f64::from(s)).to_radians();
        let d = radius * 1.4 + 60.0;
        let t = x.target;
        let position = above(g, [t[0] + a.cos() * d, t[1] + 4.0, t[2] + a.sin() * d], 2.0);
        Pose { position, look: sk::sn_star(t), fov: 62.0 - 8.0 * s, roll: 0.0 }
    }));
    // In close on its boiling face.
    shots.push(Shot::cut(150, 222, move |x, g, s| {
        let a = 225.0_f64.to_radians();
        let d = radius * 1.4 + 60.0;
        let t = x.target;
        let position = above(g, [t[0] + a.cos() * d, t[1] + 4.0, t[2] + a.sin() * d], 2.0);
        Pose { position, look: sk::sn_star(t), fov: 22.0 - 8.0 * s, roll: 0.0 }
    }));
    // It falls in: the lens snaps out.
    shots.push(Shot::cut(222, 252, move |x, g, s| {
        let a = 225.0_f64.to_radians();
        let d = radius * 1.4 + 60.0;
        let t = x.target;
        let position = above(g, [t[0] + a.cos() * d, t[1] + 4.0, t[2] + a.sin() * d], 2.0);
        Pose { position, look: sk::sn_star(t), fov: 14.0 + 56.0 * s * s, roll: 0.0 }
    }));
    // The front coming for the camera, low on the land.
    shots.push(Shot::cut(252, 330, move |x, g, s| {
        let a = 120.0_f64.to_radians();
        let d = radius * 0.85;
        let t = x.target;
        let position = above(g, [t[0] + a.cos() * d, t[1] + 2.0, t[2] + a.sin() * d], 1.6);
        let look = [t[0], t[1] + 18.0 + 20.0 * f64::from(s), t[2]];
        Pose { position, look, fov: 74.0, roll: 2.0 * s }
    }));
    // Up over the burnt land under a sky made day.
    shots.push(Shot::sweep(330, 410, 12, move |x, g, s| {
        let a = (120.0 + 30.0 * f64::from(s)).to_radians();
        let d = radius * (0.85 + 0.6 * f64::from(s));
        let t = x.target;
        let position = above(g, [t[0] + a.cos() * d, t[1] + 6.0 + radius * 0.9 * f64::from(s), t[2] + a.sin() * d], 3.0);
        Pose { position, look: [t[0], t[1] + 40.0 * f64::from(s), t[2]], fov: 70.0, roll: 0.0 }
    }));
    cs::Cutscene::from_shots(shots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_star_swells_falls_in_and_goes() {
        assert_eq!(star_radius(20.0), 0.0);
        assert!(star_radius(200.0) > star_radius(80.0));
        assert!(star_radius(245.0) < 2.0);
        assert_eq!(star_radius(SN_BLAST as f32), 0.0);
        assert!(temperature(210.0) > 0.8 && temperature(245.0) < 0.1);
    }

    #[test]
    fn the_front_sweeps_out_to_its_edge() {
        assert_eq!(sk::sn_ring(250.0, 1.0), 0.0);
        let mid = sk::sn_ring(280.0, 1.0);
        assert!(mid > 0.0 && mid < sk::sn_radius(1.0));
        assert!((sk::sn_ring(1000.0, 1.0) - sk::sn_radius(1.0)).abs() < 1e-9);
    }
}
