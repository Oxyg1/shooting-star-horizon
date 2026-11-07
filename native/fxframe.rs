// What the spells ask the renderer to draw this frame: the original's
// `PostPass` list (a program of its fx shaders and the uniforms it is given)
// and its `ScreenFx` (the composite's shake, flash, bloom...). The game's half
// builds a frame each time the world is drawn; the render pass takes it.
//
// Plain data on purpose: both crates see it, and it is tested on its own.
use std::sync::Mutex;

/// A uniform's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// One to four floats, or a matrix's sixteen.
    F(Vec<f32>),
    /// A point in the world, made camera-relative by the renderer.
    World([f64; 3]),
    /// An array of vec4s (`Pts[16]`, `Eng[48]`).
    Vec4s(Vec<[f32; 4]>),
}

/// One pass of one of the original's fx programs (`star_beam`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    pub program: &'static str,
    pub uniforms: Vec<(&'static str, Value)>,
    /// Sampler name and the texture it reads (`EarthDay` = `moonfall/earth_day.jpg`).
    pub textures: Vec<(&'static str, &'static str)>,
}

impl Pass {
    pub fn new(program: &'static str) -> Pass {
        Pass { program, uniforms: Vec::new(), textures: Vec::new() }
    }

    pub fn f(mut self, name: &'static str, values: &[f32]) -> Pass {
        self.uniforms.push((name, Value::F(values.to_vec())));
        self
    }

    pub fn at(mut self, name: &'static str, point: [f64; 3]) -> Pass {
        self.uniforms.push((name, Value::World(point)));
        self
    }

    pub fn vec4s(mut self, name: &'static str, values: Vec<[f32; 4]>) -> Pass {
        self.uniforms.push((name, Value::Vec4s(values)));
        self
    }

    pub fn texture(mut self, sampler: &'static str, file: &'static str) -> Pass {
        self.textures.push((sampler, file));
        self
    }
}

/// The composite's knobs, as the original's `ScreenFx`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub shake: f32,
    pub aberration: f32,
    pub desaturate: f32,
    pub zoom_blur: f32,
    /// r, g, b, strength.
    pub flash: [f32; 4],
    pub vignette: [f32; 4],
    pub bloom: f32,
    pub bloom_radius: f32,
    /// strength, mode, x, y (the anime impact frames).
    pub impact: [f32; 4],
    /// contrast, grain, afterimage, keep hot.
    pub grade: [f32; 4],
    /// How much of the picture is the film's own (a space scene, not the world).
    pub synthetic: f32,
    /// Frames a second the whole picture steps at (the impact frames' 12), or 0.
    pub step_fps: f32,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            shake: 0.0,
            aberration: 0.0,
            desaturate: 0.0,
            zoom_blur: 0.0,
            flash: [0.0; 4],
            vignette: [0.0; 4],
            bloom: 0.0,
            bloom_radius: 0.0,
            impact: [0.0, 0.0, 0.5, 0.5],
            grade: [0.0; 4],
            synthetic: 0.0,
            step_fps: 0.0,
        }
    }
}

fn rgb(color: u32) -> [f32; 3] {
    [((color >> 16) & 255) as f32 / 255.0, ((color >> 8) & 255) as f32 / 255.0, (color & 255) as f32 / 255.0]
}

impl Screen {
    pub fn idle(&self) -> bool {
        self.shake < 1e-4
            && self.aberration < 1e-4
            && self.desaturate < 1e-4
            && self.zoom_blur < 1e-4
            && self.flash[3] < 1e-4
            && self.vignette[3] < 1e-4
            && self.impact[0] < 1e-4
            && self.bloom < 1e-4
            && self.grade.iter().all(|g| *g < 1e-4)
    }

    /// The strongest flash wins, in its own colour.
    pub fn add_flash(&mut self, color: u32, strength: f32) {
        if strength > self.flash[3] {
            let [r, g, b] = rgb(color);
            self.flash = [r, g, b, strength];
        }
    }

    pub fn add_vignette(&mut self, color: u32, strength: f32) {
        if strength > self.vignette[3] {
            let [r, g, b] = rgb(color);
            self.vignette = [r, g, b, strength];
        }
    }

    /// The picture steps at `fps` (the slowest asked for wins).
    pub fn step(&mut self, fps: f32) {
        self.step_fps = if self.step_fps > 0.0 { self.step_fps.min(fps) } else { fps };
    }

    pub fn add_impact(&mut self, strength: f32, mode: i32, x: f32, y: f32) {
        if strength > self.impact[0] {
            self.impact = [strength, mode as f32, x, y];
        }
    }
}

/// SS-06 Gravitia's ring engine, posed for this frame: the original's
/// machine mesh (`models/gravitia/machine.bin`) drawn into `MeshColor` and
/// `MeshDist` before the passes, which lay it into the sky.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    /// Projection times view, camera-relative; `None` is the world's own.
    pub view_proj: Option<[f32; 16]>,
    /// Each part's model matrix (column-major), in the file's part order.
    pub models: Vec<[f32; 16]>,
    /// Each part's glow (its glowing panels' strength).
    pub glow: Vec<f32>,
    /// Each part's ring radius, round which the sky draws its band thinner.
    pub thin: Vec<f32>,
    /// How thin (1 is as built).
    pub thin_k: f32,
    pub cam_pos: [f32; 3],
    pub sun_dir: [f32; 3],
    pub sun_col: [f32; 3],
    /// The Earth (radius 1 round the origin) shadows the Sun.
    pub earth_shade: bool,
    pub earth_rel: [f32; 3],
    /// The light at the core: colour and reach.
    pub core: [f32; 4],
    /// Where the nebula lies, that its metal mirrors.
    pub behind: [f32; 3],
}

/// A frame's whole ask.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub passes: Vec<Pass>,
    pub screen: Screen,
    /// Seconds of the world's clock, for the shaders' `GTime`.
    pub time: f32,
    /// Gravitia's machine, while it is in the sky or the film.
    pub machine: Option<Machine>,
    /// The effects' quality: 0 low, 1 medium, 2 high, 3 ultra.
    pub quality: u8,
}

impl Default for Frame {
    fn default() -> Self {
        Frame { passes: Vec::new(), screen: Screen::default(), time: 0.0, machine: None, quality: 2 }
    }
}

impl Frame {
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty() && self.screen.idle()
    }
}

static FRAME: Mutex<Option<Frame>> = Mutex::new(None);

// ---- the original's files ---------------------------------------------------------------------------------------

/// The original's files the light needs, under the content mod's
/// `data/shooting_star_demo/`: its fx shaders, its space textures and the
/// films' tables. The installer takes them from the player's own copy of
/// the original's jar; the game half reads them once and hands them here.
pub const ASSET_FILES: &[&str] = &[
    "fx/common.glsl",
    "fx/star.glsl",
    "fx/space.glsl",
    "fx/title.glsl",
    "fx/tess_engine.glsl",
    "fx/grav_sky.glsl",
    "fx/bloom_down.fsh",
    "fx/bloom_up.fsh",
    "fx/composite.fsh",
    "fx/grav_gold.fsh",
    "fx/grav_space.fsh",
    "fx/grav_world.fsh",
    "fx/gungnir_mech.fsh",
    "fx/gungnir_space.fsh",
    "fx/gungnir_world.fsh",
    "fx/star_beam.fsh",
    "fx/star_gun.fsh",
    "fx/star_rays.fsh",
    "fx/star_space.fsh",
    "fx/stars_space.fsh",
    "fx/stars_world.fsh",
    "fx/tess_engine_view.fsh",
    "fx/tess_sky.fsh",
    "fx/tess_space.fsh",
    "fx/tess_world.fsh",
    "fx/void_sky.fsh",
    "textures/moonfall/earth_day.jpg",
    "textures/moonfall/earth_clouds.jpg",
    "textures/moonfall/earth_height.jpg",
    "textures/moonfall/earth_lights.jpg",
    "textures/moonfall/milky_way.jpg",
    "textures/moonfall/moon_color.jpg",
    "textures/shooting_star/jupiter.jpg",
    "textures/shooting_star/milky_way_top.jpg",
    "textures/shooting_star/planets.jpg",
    "textures/shooting_star/saturn_ring.jpg",
    "textures/gravitia/nebula.jpg",
    "films/star_space.bin",
    "films/star_gun.bin",
    "films/star_rays.bin",
    "films/gungnir_space.bin",
    "films/gungnir_mech.bin",
    "films/stars_space_0.bin",
    "films/stars_space_1.bin",
    "films/stars_space_2.bin",
    "films/stars_space_3.bin",
    "films/tess_space.bin",
    "films/grav_space.bin",
    "models/gravitia/machine.bin",
    "films/tess_sky.bin",
];

static ASSETS: Mutex<Option<std::collections::HashMap<&'static str, &'static [u8]>>> = Mutex::new(None);

fn assets() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<&'static str, &'static [u8]>>> {
    match ASSETS.lock() {
        Ok(a) => a,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The game's half: one of `ASSET_FILES`, read. Kept for the session.
pub fn provide(path: &'static str, bytes: Vec<u8>) {
    let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    assets().get_or_insert_with(Default::default).insert(path, bytes);
}

/// One of the original's files (`fx/star_beam.fsh`), once the game read it.
pub fn asset(path: &str) -> Option<&'static [u8]> {
    assets().as_ref()?.get(path).copied()
}

/// The game's half: this frame's passes.
pub fn publish(frame: Frame) {
    let mut slot = match FRAME.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    *slot = (!frame.is_empty()).then_some(frame);
}

/// The render pass: what to draw, if anything.
pub fn current() -> Option<Frame> {
    match FRAME.lock() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strongest_flash_and_vignette_win() {
        let mut s = Screen::default();
        s.add_flash(0xFF0000, 0.5);
        s.add_flash(0x00FF00, 0.2);
        assert_eq!(s.flash, [1.0, 0.0, 0.0, 0.5]);
        s.add_vignette(0x0000FF, 0.3);
        assert_eq!(s.vignette[2], 1.0);
        assert!(!s.idle());
        assert!(Screen::default().idle());
    }

    #[test]
    fn an_empty_frame_is_not_kept() {
        publish(Frame::default());
        assert!(current().is_none());
        publish(Frame { passes: vec![Pass::new("star_beam").f("P0", &[1.0])], ..Frame::default() });
        assert_eq!(current().map(|f| f.passes.len()), Some(1));
    }
}
