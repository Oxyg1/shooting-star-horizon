// SS-H1 Event Horizon: the port's own skill, not the original's. Its light
// frame by frame (the `hz_horizon` pass over the world, the screen), its
// lights and fronts for the shared passes, and its film: a camera in the
// world that watches the hole fall, feed and evaporate.
//
// Timeline (ticks at speed 1, `shooting_star_skills::EH_*`):
//   9..130   the target's rings on the ground
//   70..130  the hole falls from 220 blocks up, a violet trail over it
//   130      it lands: impact frames, a shock front
//   130..330 its disk spins up and the sphere is eaten, the front glowing
//   330..346 it shrinks to nothing: the world drains of colour
//   346      it evaporates: white flash, a sphere of light, two jets
//   346..    the rim of fused obsidian cools
use super::shooting_star_cutscenes::{self as cs, Pose, Shot};
use super::shooting_star_skills::{self as sk, EH_COLLAPSE, EH_DROP, EH_FEED, EH_FLASH, EH_LAND};
use super::shooting_star_visuals::{self as vis, Spell, window};
use horizon_client_render::gfx::native_render::shooting_star_fxframe::{Frame, Pass, Screen};

const DRAWING: f32 = 1.0 / 12.0;
/// The impact frames as it lands, and as it evaporates.
const LAND_FRAMES: [i32; 4] = [17, 15, 20, 16];
const FLASH_FRAMES: [i32; 8] = [1, 0, 19, 20, 17, 6, 15, 18];

fn k_of(t: f32, from: i32, to: i32) -> f32 {
    ((t - from as f32) / (to - from) as f32).clamp(0.0, 1.0)
}

/// The hole's radius (its shadow's) at a tick, in blocks.
pub fn shadow(t: f32, radius: f32) -> f32 {
    let t32 = t;
    if t32 < EH_DROP as f32 || t32 >= EH_FLASH as f32 {
        return 0.0;
    }
    let full = radius * 0.2;
    if t32 < EH_LAND as f32 {
        return 0.5 + 0.4 * k_of(t, EH_DROP, EH_LAND);
    }
    if t32 < EH_FEED as f32 {
        return 0.9 + (radius * 0.1 - 0.9).max(0.0) * vis::smooth(k_of(t, EH_LAND, EH_FEED));
    }
    if t32 < EH_COLLAPSE as f32 {
        let eaten = sk::eh_eaten(f64::from(t)) as f32;
        let base = (radius * 0.1).max(0.9);
        return base + (full - base).max(0.0) * eaten;
    }
    let k = k_of(t, EH_COLLAPSE, EH_FLASH);
    (full.max(0.9) * (1.0 - k * k)).max(0.05)
}

/// How bright its disk burns.
fn disk(t: f32) -> f32 {
    if t < EH_LAND as f32 || t >= EH_FLASH as f32 {
        return 0.0;
    }
    if t < EH_COLLAPSE as f32 {
        return vis::smooth(k_of(t, EH_LAND, EH_FEED + 10));
    }
    1.0 + 0.8 * k_of(t, EH_COLLAPSE, EH_FLASH)
}

/// How far from the camera the light still reaches full strength.
fn nearness(camera: [f64; 3], hole: [f64; 3], radius: f64) -> f32 {
    let d = ((camera[0] - hole[0]).powi(2) + (camera[1] - hole[1]).powi(2) + (camera[2] - hole[2]).powi(2)).sqrt();
    (1.0 - (d - radius * 2.0).max(0.0) / (radius * 20.0 + 500.0)).clamp(0.2, 1.0) as f32
}

/// The light of an Event Horizon this frame.
pub fn frame(frame: &mut Frame, spell: &Spell, camera: [f64; 3]) {
    let t = spell.t;
    let start = &spell.start;
    let radius = sk::eh_radius(start.power);
    let r = radius as f32;
    let hole = sk::eh_hole(start.target, start.power, f64::from(t));
    let centre = sk::eh_center(start.target, start.power);
    if !(8.0..=start.skill.fx_duration() as f32).contains(&t) {
        return;
    }
    let rs = shadow(t, r);
    let din = rs * 3.0;
    // The disk lives in the hollow it has eaten (and a little more as it lands).
    let grown = r * (0.6 + 0.65 * sk::eh_eaten(f64::from(t)) as f32);
    let dout = (din + 4.0).max(grown);
    let into_collapse = (t - EH_COLLAPSE as f32).max(0.0);
    let seconds = t / 20.0 + into_collapse * into_collapse / 80.0;
    let eaten = if (EH_FEED as f32..EH_FLASH as f32).contains(&t) { (radius * sk::eh_eaten(f64::from(t))) as f32 } else { 0.0 };
    let since = t - EH_FLASH as f32;
    let rim = if since > 0.0 { window(since, 0.0, 6.0, 1.0e9, 1.0e9) * (-since / 360.0).exp() } else { 0.0 };
    let fall = window(t, EH_DROP as f32, EH_DROP as f32 + 5.0, EH_LAND as f32 - 4.0, EH_LAND as f32 + 3.0);
    let flash = if (0.0..90.0).contains(&since) { (-since / 18.0).exp() } else { 0.0 };
    let flash_r = if since >= 0.0 { r * (0.4 + 3.2 * (1.0 - (-since / 10.0).exp())) } else { 0.0 };
    let jet = if (0.0..30.0).contains(&since) { since / 30.0 } else { 0.0 };
    let reticle = window(t, 9.0, 22.0, EH_LAND as f32 - 6.0, EH_LAND as f32 + 6.0);
    let collapse = k_of(t, EH_COLLAPSE, EH_FLASH);
    let at = if since >= 0.0 { centre } else { hole };
    frame.passes.push(
        Pass::new("hz_horizon")
            .at("Center", at)
            .f("P0", &[rs, din, dout, seconds])
            .f("P1", &[disk(t), 1.0, flash, spell.seed_f()])
            .f("P2", &[reticle, eaten, rim, fall])
            .f("P3", &[r, collapse, flash_r, jet]),
    );
    vis::focus_on(at);
    screen(&mut frame.screen, t, nearness(camera, at, radius));
}

fn screen(screen: &mut Screen, t: f32, near: f32) {
    // The fall: the edges darken, the picture strains.
    if (EH_DROP as f32..EH_LAND as f32).contains(&t) {
        let k = k_of(t, EH_DROP, EH_LAND);
        screen.add_vignette(0x14001E, k * 0.45 * near);
        screen.aberration = screen.aberration.max(k * k * 0.35 * near);
        screen.zoom_blur = screen.zoom_blur.max(k * k * k * 0.2 * near);
    }
    // The landing.
    let land = (t - EH_LAND as f32) / 20.0;
    if (0.0..3.0).contains(&land) {
        if land < LAND_FRAMES.len() as f32 * DRAWING {
            vis::impact_sequence(screen, land, near, DRAWING, &LAND_FRAMES);
            screen.step(12.0);
        }
        screen.add_flash(0xFFB070, (-land * 6.0).exp() * 1.3 * near);
        screen.shake = screen.shake.max(window(land, 0.0, 0.02, 0.4, 1.6) * 1.2 * near);
        screen.aberration = screen.aberration.max(window(land, 0.0, 0.02, 0.3, 1.2) * near);
    }
    // Feeding: the ground trembles harder as it eats; the light pulls inward.
    if (EH_LAND as f32..EH_COLLAPSE as f32).contains(&t) {
        let e = sk::eh_eaten(f64::from(t)) as f32;
        let on = window(t, EH_LAND as f32, EH_FEED as f32, EH_COLLAPSE as f32 - 6.0, EH_COLLAPSE as f32);
        screen.shake = screen.shake.max((0.1 + 0.3 * e) * on * near);
        screen.bloom = screen.bloom.max(0.9 * on);
        screen.bloom_radius = screen.bloom_radius.max(1.0 + 0.8 * on);
        screen.add_vignette(0x1C0700, (0.25 + 0.2 * e) * on * near);
        screen.zoom_blur = screen.zoom_blur.max(0.12 * e * on * near);
        screen.grade[0] = screen.grade[0].max(0.15 * on);
    }
    // The collapse: a breath with the colour drained out of the world.
    if (EH_COLLAPSE as f32..EH_FLASH as f32).contains(&t) {
        let k = k_of(t, EH_COLLAPSE, EH_FLASH);
        vis::stopped_world(screen, vis::smooth(k * 3.0) * 0.85 * near);
        screen.zoom_blur = screen.zoom_blur.max(k * k * 0.8 * near);
        screen.aberration = screen.aberration.max(k * 1.2 * near);
        screen.bloom = screen.bloom.max(1.0);
    }
    // The flash.
    let s = (t - EH_FLASH as f32) / 20.0;
    if (0.0..10.0).contains(&s) {
        if s < 0.06 {
            screen.add_flash(0xFFFFFF, 6.0 * near);
        } else if s - 0.06 < FLASH_FRAMES.len() as f32 * DRAWING {
            vis::impact_sequence(screen, s - 0.06, near, DRAWING, &FLASH_FRAMES);
            screen.step(12.0);
            let drawing = ((s - 0.06) / DRAWING) as i32;
            screen.zoom_blur = screen.zoom_blur.max(if drawing % 2 == 0 { 1.0 } else { 0.3 } * near);
        }
        screen.add_flash(0xFFE6C8, (-s * 2.2).exp() * 1.6 * near);
        screen.shake = screen.shake.max(window(s, 0.0, 0.02, 1.2, 4.5) * 2.0 * near);
        screen.aberration = screen.aberration.max(window(s, 0.0, 0.02, 0.5, 2.5) * 1.4 * near);
        screen.bloom = screen.bloom.max(1.6 * window(s, 0.0, 0.1, 3.0, 9.0));
        screen.bloom_radius = screen.bloom_radius.max(1.0 + 1.4 * window(s, 0.0, 0.1, 2.0, 7.0));
        let heat = window(s, 0.5, 1.5, 4.0, 10.0);
        screen.add_vignette(0x3A1000, heat * 0.4 * near);
        screen.grade[1] = screen.grade[1].max(0.2 * heat);
    }
}

/// Its lights for `hz_glow`: (camera-relative point, reach, colour times strength, column height).
pub fn lights(spell: &Spell, out: &mut Vec<vis::Light>) {
    let t = spell.t;
    let start = &spell.start;
    let radius = sk::eh_radius(start.power);
    let r = radius as f32;
    let hole = sk::eh_hole(start.target, start.power, f64::from(t));
    let centre = sk::eh_center(start.target, start.power);
    let d = disk(t);
    if d > 0.0 {
        out.push(vis::Light { at: hole, reach: r * 1.6 + 6.0, color: [1.6 * d, 0.8 * d, 0.3 * d], tall: 0.0 });
    }
    if (EH_DROP as f32..EH_LAND as f32).contains(&t) {
        out.push(vis::Light { at: hole, reach: 18.0, color: [2.5, 1.8, 4.0], tall: 0.0 });
    }
    let reticle = window(t, 9.0, 22.0, EH_LAND as f32 - 6.0, EH_LAND as f32 + 6.0);
    if reticle > 0.0 {
        out.push(vis::Light { at: [start.target[0], start.target[1] + 2.0, start.target[2]], reach: r * 1.1, color: [1.2 * reticle, 0.5 * reticle, 0.15 * reticle], tall: 0.0 });
    }
    let since = t - EH_FLASH as f32;
    if (0.0..120.0).contains(&since) {
        let k = (-since / 14.0).exp();
        out.push(vis::Light { at: centre, reach: r * 5.0, color: [30.0 * k, 27.0 * k, 24.0 * k], tall: 0.0 });
    }
    if since > 0.0 {
        let k = window(since, 0.0, 6.0, 1.0e9, 1.0e9) * (-since / 360.0).exp();
        out.push(vis::Light { at: centre, reach: r * 1.3, color: [3.2 * k, 0.9 * k, 0.15 * k], tall: 0.0 });
    }
}

/// Its fronts and heat for `hz_shock`.
pub fn fronts(spell: &Spell, shocks: &mut Vec<vis::Front>, heat: &mut Vec<vis::Front>) {
    let t = spell.t;
    let start = &spell.start;
    let radius = sk::eh_radius(start.power) as f32;
    let centre = sk::eh_center(start.target, start.power);
    let land = t - EH_LAND as f32;
    if (0.0..40.0).contains(&land) {
        shocks.push(vis::Front { at: centre, radius: land * 4.0 + 2.0, strength: 1.0 - land / 40.0, thick: 3.0 });
    }
    let since = t - EH_FLASH as f32;
    if (0.0..70.0).contains(&since) {
        shocks.push(vis::Front { at: centre, radius: since * 7.0 + radius, strength: 1.6 * (1.0 - since / 70.0), thick: 5.0 });
    }
    if (EH_FEED as f32..EH_FLASH as f32).contains(&t) {
        heat.push(vis::Front { at: centre, radius: radius * 0.9, strength: 0.6 * sk::eh_eaten(f64::from(t)) as f32, thick: 0.0 });
    }
    if since > 0.0 {
        heat.push(vis::Front { at: centre, radius: radius * 1.1, strength: (-since / 500.0).exp(), thick: 0.0 });
    }
}

/// Its film: a camera in the world, round the target.
pub fn cutscene(start: &sk::Start) -> cs::Cutscene {
    let power = start.power;
    let radius = sk::eh_radius(power);
    let above = |g: &dyn cs::Ground, mut p: cs::V, margin: f64| {
        if let Some(top) = g.top(p[0], p[2]) {
            p[1] = p[1].max(top + margin);
        }
        p
    };
    let mut shots: Vec<Shot> = Vec::new();
    // The hand on the remote.
    shots.push(Shot::cut(0, 9, |x, _, s| Pose {
        position: x.frame(x.feet, 1.35 - 0.15 * f64::from(s), 1.7 - 0.1 * f64::from(s), 2.1 - 0.25 * f64::from(s)),
        look: x.frame(x.feet, 0.36, 1.38, 0.62),
        fov: 40.0,
        roll: 0.0,
    }));
    // Over the shoulder, up at the sky where it will come from.
    shots.push(Shot::cut(9, 70, move |x, g, s| {
        let position = above(g, x.frame(x.feet, 1.2, 2.2, -3.5), 1.5);
        let target = x.target;
        let sky = [target[0], target[1] + 220.0, target[2]];
        let look = [
            target[0] + (sky[0] - target[0]) * f64::from(s),
            target[1] + 20.0 + 200.0 * f64::from(s),
            target[2] + (sky[2] - target[2]) * f64::from(s),
        ];
        Pose { position, look, fov: 62.0 - 8.0 * s, roll: 0.0 }
    }));
    // The fall, seen from the ground off to the side.
    shots.push(Shot::cut(70, 130, move |x, g, s| {
        let t = EH_DROP as f64 + f64::from(s) * f64::from(EH_LAND - EH_DROP);
        let hole = sk::eh_hole(x.target, power, t);
        let side = radius * 4.0 + 20.0;
        let position = above(g, x.frame(x.target, side, 3.0, -side * 0.6), 2.0);
        Pose { position, look: hole, fov: 70.0 - 10.0 * s, roll: 0.0 }
    }));
    // Feeding: a slow low orbit round it.
    shots.push(Shot::sweep(130, 230, 6, move |x, g, s| {
        let c = sk::eh_center(x.target, power);
        let a = (40.0 + 70.0 * f64::from(s)).to_radians();
        let d = radius * 3.6 + 10.0;
        let position = above(g, [c[0] + a.cos() * d, c[1] + radius * 0.5 + 3.0, c[2] + a.sin() * d], 2.0);
        Pose { position, look: c, fov: 64.0, roll: 3.0 * (1.0 - s) }
    }));
    // Pulled back and up: the whole sphere going.
    shots.push(Shot::sweep(230, 306, 14, move |x, g, s| {
        let c = sk::eh_center(x.target, power);
        let a = (110.0 + 40.0 * f64::from(s)).to_radians();
        let d = radius * (5.5 + 1.5 * f64::from(s)) + 20.0;
        let position = above(g, [c[0] + a.cos() * d, c[1] + radius * 1.6 + 8.0, c[2] + a.sin() * d], 3.0);
        Pose { position, look: c, fov: 60.0, roll: 0.0 }
    }));
    // In close as it shrinks, the lens drawing in.
    shots.push(Shot::cut(306, 346, move |x, g, s| {
        let c = sk::eh_center(x.target, power);
        let a = 160.0_f64.to_radians();
        let d = radius * (2.8 - 0.6 * f64::from(s)) + 6.0;
        let position = above(g, [c[0] + a.cos() * d, c[1] + radius * 0.25, c[2] + a.sin() * d], 2.0);
        Pose { position, look: c, fov: 55.0 - 20.0 * s, roll: 0.0 }
    }));
    // The flash, wide.
    shots.push(Shot::cut(346, 392, move |x, g, s| {
        let c = sk::eh_center(x.target, power);
        let a = 200.0_f64.to_radians();
        let d = radius * 9.0 + 40.0;
        let position = above(g, [c[0] + a.cos() * d, c[1] + radius * 2.0 + 15.0 + 10.0 * f64::from(s), c[2] + a.sin() * d], 4.0);
        Pose { position, look: c, fov: 72.0, roll: 0.0 }
    }));
    cs::Cutscene::from_shots(shots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hole_grows_and_evaporates() {
        assert_eq!(shadow(60.0, 14.0), 0.0);
        assert!(shadow(100.0, 14.0) > 0.0);
        assert!(shadow(320.0, 14.0) > shadow(140.0, 14.0));
        assert!(shadow(345.0, 14.0) < shadow(330.0, 14.0));
        assert_eq!(shadow(EH_FLASH as f32, 14.0), 0.0);
    }

    #[test]
    fn it_falls_from_the_sky_and_settles() {
        let target = [0.5, 70.0, 0.5];
        let high = sk::eh_hole(target, 1.0, 60.0);
        let low = sk::eh_hole(target, 1.0, 140.0);
        assert!(high[1] > low[1] + 200.0);
        assert!((low[1] - sk::eh_center(target, 1.0)[1]).abs() < 1e-9);
        assert!(sk::eh_hole(target, 1.0, 100.0)[1] < high[1]);
    }
}
