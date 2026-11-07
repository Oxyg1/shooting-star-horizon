// The spells' light, drawn by the original's own shaders.
//
// The original draws every spell as a chain of fullscreen passes over the
// world (`PostFxRenderer`): each of its fx programs reads the picture so far
// and the world's depth and writes the next, a five-level bloom picks out the
// light the spells added, and `composite` lays it all down with the screen's
// shake, flash and grading. This is that chain on wgpu, with those same GLSL
// programs compiled by naga (`shooting_star_glsl`), and better buffers: the
// original passes 8-bit pictures from pass to pass, this keeps 16-bit float
// throughout, so its glows never band.
//
// Every picture of the chain is kept as OpenGL keeps one (its first row is
// the bottom of the picture) and drawn with clip y turned over, so the GLSL's
// `texCoord`, `gl_FragCoord` and projections mean what they meant in
// Minecraft. The world's frame is turned over once on the way in and once on
// the way out.
use super::shooting_star_fxframe::{self as fx, Frame, Value};
use super::shooting_star_glsl as glsl;
use super::shooting_star_hzfx as hzfx;
use crate::gfx::native_render::WorldDrawn;
use std::collections::HashMap;
use std::sync::Mutex;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Pixels the spells' passes draw at most (the original's 2.3 million): past
/// it they draw smaller and `composite` puts the world's detail back.
const PIXEL_BUDGET: f64 = 3_686_400.0;
const BLOOM_LEVELS: usize = 5;

/// One of the original's fx shaders (`star_beam.fsh`), as the game read it.
fn source(name: &str) -> Option<&'static str> {
    if let Some((_, own)) = OWN_PROGRAMS.iter().find(|(file, _)| *file == name) {
        return Some(own);
    }
    fx::asset(&format!("fx/{name}")).and_then(|b| std::str::from_utf8(b).ok())
}

/// The port's own fx programs (not the original's): written for Horizon,
/// in the original's dialect so they share its `common.glsl` and chain.
const OWN_PROGRAMS: &[(&str, &str)] = &[
    ("hz_glow.fsh", hzfx::GLOW),
    ("hz_shock.fsh", hzfx::SHOCK),
    ("hz_streak.fsh", hzfx::STREAK),
    ("hz_horizon.fsh", hzfx::HORIZON),
];

/// One of the original's textures (`moonfall/earth_day.jpg`).
fn texture_bytes(name: &str) -> Option<&'static [u8]> {
    fx::asset(&format!("textures/{name}"))
}

/// The chain's own WGSL: the fullscreen triangle, taking the world's frame
/// in, and handing the result back.
const PLUMBING: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

fn corner(index: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return c[index];
}

// Into the chain's pictures, kept as OpenGL keeps them: clip y turned over.
@vertex
fn vs_gl(@builtin(vertex_index) index: u32) -> VsOut {
    let p = corner(index);
    var out: VsOut;
    out.position = vec4<f32>(p.x, -p.y, 0.0, 1.0);
    out.uv = p * 0.5 + vec2<f32>(0.5);
    return out;
}

// Onto the world's frame, kept as wgpu keeps it.
@vertex
fn vs_frame(@builtin(vertex_index) index: u32) -> VsOut {
    let p = corner(index);
    var out: VsOut;
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = p * 0.5 + vec2<f32>(0.5);
    return out;
}

@group(0) @binding(0) var frame_color: texture_2d<f32>;
@group(0) @binding(1) var frame_depth: texture_depth_2d;

struct Intake {
    @location(0) color: vec4<f32>,
    @location(1) depth: vec4<f32>,
};

// The world's colour and depth, turned over into the chain.
@fragment
fn fs_intake(v: VsOut) -> Intake {
    let cs = vec2<f32>(textureDimensions(frame_color));
    let ds = vec2<f32>(textureDimensions(frame_depth));
    let flip = vec2<f32>(v.uv.x, 1.0 - v.uv.y);
    let ci = vec2<i32>(clamp(flip * cs, vec2<f32>(0.0), cs - vec2<f32>(1.0)));
    let di = vec2<i32>(clamp(flip * ds, vec2<f32>(0.0), ds - vec2<f32>(1.0)));
    var out: Intake;
    out.color = vec4<f32>(textureLoad(frame_color, ci, 0).rgb, 1.0);
    out.depth = vec4<f32>(textureLoad(frame_depth, di, 0), 0.0, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

// A picture of the chain, scaled onto another (or onto the frame).
@fragment
fn fs_copy(v: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(picture, picture_sampler, v.uv).rgb, 1.0);
}
"#;

/// One of the original's programs, ready to draw.
struct Program {
    layout: glsl::Layout,
    binds: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

/// A picture of the chain.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

struct Targets {
    size: [u32; 2],
    fx_size: [u32; 2],
    raw: Target,
    scene: Target,
    depth: Target,
    scene_low: Target,
    ping: [Target; 2],
    down: Vec<Target>,
    up: Vec<Target>,
    out: Target,
    black: Target,
    /// The bloom with its streaks.
    streak: Target,
}

struct State {
    frame_format: wgpu::TextureFormat,
    raw_format: wgpu::TextureFormat,
    plumbing: wgpu::ShaderModule,
    intake_binds: wgpu::BindGroupLayout,
    intake: wgpu::RenderPipeline,
    copy_binds: wgpu::BindGroupLayout,
    copy_gl: wgpu::RenderPipeline,
    copy_frame: wgpu::RenderPipeline,
    programs: HashMap<&'static str, Option<Program>>,
    textures: HashMap<&'static str, Option<Target>>,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    repeat: wgpu::Sampler,
    targets: Option<Targets>,
    buffers: Vec<wgpu::Buffer>,
    warned: bool,
    /// When the picture last moved while it steps (the impact frames' 12 a second).
    stepped: Option<std::time::Instant>,
    /// Gravitia's machine: not tried yet, or read (None when it cannot be).
    mesh: Option<Option<MeshPass>>,
    mesh_targets: Option<MeshTargets>,
    /// How far the warm-up got through `warm_list`.
    warmed: usize,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn target(device: &wgpu::Device, label: &str, size: [u32; 2], format: wgpu::TextureFormat, extra: wgpu::TextureUsages) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | extra,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Target { texture, view, size }
}

impl Targets {
    fn new(device: &wgpu::Device, size: [u32; 2], fx_size: [u32; 2], raw_format: wgpu::TextureFormat) -> Targets {
        let raw = {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Shooting Star frame copy"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: raw_format,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[raw_format.remove_srgb_suffix()],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(raw_format.remove_srgb_suffix()),
                ..Default::default()
            });
            Target { texture, view, size }
        };
        let none = wgpu::TextureUsages::empty();
        let mut down = Vec::new();
        let mut up = Vec::new();
        for i in 0..BLOOM_LEVELS {
            let s = [(fx_size[0] >> (i + 1)).max(1), (fx_size[1] >> (i + 1)).max(1)];
            down.push(target(device, "Shooting Star bloom", s, HDR, none));
            up.push(target(device, "Shooting Star bloom", s, HDR, none));
        }
        Targets {
            size,
            fx_size,
            raw,
            scene: target(device, "Shooting Star scene", size, HDR, none),
            depth: target(device, "Shooting Star depth", size, DEPTH, none),
            scene_low: target(device, "Shooting Star scene, small", fx_size, HDR, none),
            ping: [
                target(device, "Shooting Star pass", fx_size, HDR, none),
                target(device, "Shooting Star pass", fx_size, HDR, none),
            ],
            down,
            up,
            out: target(device, "Shooting Star result", size, HDR, none),
            streak: target(device, "Shooting Star streaks", [(fx_size[0] >> 1).max(1), (fx_size[1] >> 1).max(1)], HDR, none),
            black: target(device, "Shooting Star black", [1, 1], HDR, none),
        }
    }
}

fn sampler(device: &wgpu::Device, filter: wgpu::FilterMode, address: wgpu::AddressMode) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Shooting Star sampler"),
        address_mode_u: address,
        address_mode_v: if address == wgpu::AddressMode::Repeat { wgpu::AddressMode::ClampToEdge } else { address },
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: if filter == wgpu::FilterMode::Linear { wgpu::MipmapFilterMode::Linear } else { wgpu::MipmapFilterMode::Nearest },
        ..Default::default()
    })
}

fn pipeline(
    device: &wgpu::Device,
    label: &str,
    binds: &wgpu::BindGroupLayout,
    vs: (&wgpu::ShaderModule, &str),
    fs: (&wgpu::ShaderModule, &str),
    targets: &[Option<wgpu::ColorTargetState>],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(binds)],
            immediate_size: 0,
        })),
        vertex: wgpu::VertexState {
            module: vs.0,
            entry_point: Some(vs.1),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: fs.0,
            entry_point: Some(fs.1),
            compilation_options: Default::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn color_target(format: wgpu::TextureFormat) -> Option<wgpu::ColorTargetState> {
    Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })
}

fn texture_entry(binding: u32, filterable: bool, depth: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: if depth { wgpu::TextureSampleType::Depth } else { wgpu::TextureSampleType::Float { filterable } },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32, filtering: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(if filtering { wgpu::SamplerBindingType::Filtering } else { wgpu::SamplerBindingType::NonFiltering }),
        count: None,
    }
}

impl State {
    fn new(device: &wgpu::Device, frame_format: wgpu::TextureFormat, raw_format: wgpu::TextureFormat) -> Option<State> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let plumbing = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shooting Star plumbing"),
            source: wgpu::ShaderSource::Wgsl(PLUMBING.into()),
        });
        let intake_binds = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shooting Star intake"),
            entries: &[texture_entry(0, false, false), texture_entry(1, false, true)],
        });
        let intake = pipeline(
            device,
            "Shooting Star intake",
            &intake_binds,
            (&plumbing, "vs_gl"),
            (&plumbing, "fs_intake"),
            &[color_target(HDR), color_target(DEPTH)],
        );
        let copy_binds = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shooting Star copy"),
            entries: &[texture_entry(0, true, false), sampler_entry(1, true)],
        });
        let copy_gl = pipeline(device, "Shooting Star copy", &copy_binds, (&plumbing, "vs_gl"), (&plumbing, "fs_copy"), &[color_target(HDR)]);
        let copy_frame = pipeline(
            device,
            "Shooting Star out",
            &copy_binds,
            (&plumbing, "vs_frame"),
            (&plumbing, "fs_copy"),
            &[color_target(frame_format)],
        );
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::error!("[shooting_star] the effects' plumbing was refused: {error}");
            return None;
        }
        Some(State {
            frame_format,
            raw_format,
            plumbing,
            intake_binds,
            intake,
            copy_binds,
            copy_gl,
            copy_frame,
            programs: HashMap::new(),
            textures: HashMap::new(),
            linear: sampler(device, wgpu::FilterMode::Linear, wgpu::AddressMode::ClampToEdge),
            nearest: sampler(device, wgpu::FilterMode::Nearest, wgpu::AddressMode::ClampToEdge),
            repeat: sampler(device, wgpu::FilterMode::Linear, wgpu::AddressMode::Repeat),
            targets: None,
            buffers: Vec::new(),
            warned: false,
            stepped: None,
            mesh: None,
            mesh_targets: None,
            warmed: 0,
        })
    }

    /// The program `name` (`star_beam`), compiled the first time it is asked
    /// for; None, logged once, when it cannot be.
    fn program(&mut self, device: &wgpu::Device, name: &'static str) -> Option<&Program> {
        if !self.programs.contains_key(name) {
            let made = self.compile(device, name);
            self.programs.insert(name, made);
        }
        self.programs.get(name).and_then(Option::as_ref)
    }

    fn compile(&self, device: &wgpu::Device, name: &'static str) -> Option<Program> {
        let started = std::time::Instant::now();
        let file = format!("{name}.fsh");
        let (module, layout) = match glsl::compile(source, &file, &[]) {
            Ok(made) => made,
            Err(error) => {
                log::error!("[shooting_star] the shader {name} could not be compiled: {error}");
                return None;
            }
        };
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        for (sampler, binding) in &layout.samplers {
            let depth = sampler == "DepthSampler";
            entries.push(texture_entry(*binding, !depth, false));
            entries.push(sampler_entry(binding + 1, !depth));
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let binds = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shooting Star program"),
            entries: &entries,
        });
        let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let made = pipeline(device, name, &binds, (&self.plumbing, "vs_gl"), (&fragment, "main"), &[color_target(HDR)]);
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::error!("[shooting_star] the shader {name} was refused: {error}");
            return None;
        }
        log::info!("[shooting_star] the shader {name} is ready ({} ms)", started.elapsed().as_millis());
        Some(Program { layout, binds, pipeline: made })
    }

    /// A texture of the original's, decoded with its mipmaps the first time.
    fn texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, file: &'static str) -> Option<&Target> {
        if !self.textures.contains_key(file) {
            let made = load_texture(device, queue, file);
            if made.is_none() {
                log::error!("[shooting_star] the texture {file} could not be read");
            }
            self.textures.insert(file, made);
        }
        self.textures.get(file).and_then(Option::as_ref)
    }

    fn buffer(&mut self, device: &wgpu::Device, index: usize, size: u64) -> wgpu::Buffer {
        while self.buffers.len() <= index || self.buffers[index].size() < size {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Shooting Star uniforms"),
                size: size.max(256).next_power_of_two(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if self.buffers.len() <= index {
                self.buffers.push(buffer);
            } else {
                self.buffers[index] = buffer;
            }
        }
        self.buffers[index].clone()
    }
}

/// A JPEG of the bundle as an RGBA8 texture with its mip chain.
/// A JPEG's pixels: its size, and RGB or grey bytes with their channel count.
fn decode_jpeg(file: &str) -> Option<(u32, u32, usize, Vec<u8>)> {
    let bytes = texture_bytes(file)?;
    let mut decoder = zune_jpeg::JpegDecoder::new(bytes);
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let pixels = decoder.decode().ok()?;
    let channels = pixels.len().checked_div((w * h) as usize)?;
    matches!(channels, 1 | 3).then_some((w, h, channels, pixels))
}

/// `a.jpg` as RGBA, or `a.jpg+b.jpg+c.jpg` packed one grey image a channel
/// (the original's `FxTextures.decode`: the Earth's lights, clouds and heights).
fn texture_pixels(file: &str) -> Option<(u32, u32, Vec<u8>)> {
    let parts: Vec<&str> = file.split('+').collect();
    if parts.len() == 1 {
        let (w, h, channels, pixels) = decode_jpeg(file)?;
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for p in pixels.chunks_exact(channels) {
            if channels == 3 {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            } else {
                rgba.extend_from_slice(&[p[0], p[0], p[0], 255]);
            }
        }
        return Some((w, h, rgba));
    }
    let decoded: Vec<_> = parts.iter().map(|p| decode_jpeg(p)).collect::<Option<_>>()?;
    let (w, h) = (decoded[0].0, decoded[0].1);
    let count = (w * h) as usize;
    let mut rgba = vec![0u8; count * 4];
    for (c, (pw, ph, channels, pixels)) in decoded.iter().enumerate().take(4) {
        if (*pw, *ph) != (w, h) {
            continue;
        }
        for i in 0..count {
            let p = &pixels[i * channels..(i + 1) * channels];
            rgba[i * 4 + c] = if *channels == 3 { ((u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3) as u8 } else { p[0] };
        }
    }
    if decoded.len() < 4 {
        for i in 0..count {
            rgba[i * 4 + 3] = 255;
        }
    }
    Some((w, h, rgba))
}

fn load_texture(device: &wgpu::Device, queue: &wgpu::Queue, file: &str) -> Option<Target> {
    let (w, h, mut level) = texture_pixels(file)?;
    let levels = 32 - w.max(h).leading_zeros();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(file),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let (mut lw, mut lh) = (w, h);
    for mip in 0..levels {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &level,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: Some(lh) },
            wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
        );
        if mip + 1 == levels {
            break;
        }
        // The next level: each texel the mean of four.
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(lw - 1);
                        let sy = (y * 2 + dy).min(lh - 1);
                        sum += u32::from(level[((sy * lw + sx) * 4 + c) as usize]);
                    }
                    next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        level = next;
        (lw, lh) = (nw, nh);
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Some(Target { texture, view, size: [w, h] })
}

/// What one draw of a program reads, by sampler name.
struct Inputs<'a> {
    diffuse: &'a wgpu::TextureView,
    aux: &'a wgpu::TextureView,
    named: Vec<(&'static str, &'a wgpu::TextureView)>,
}

/// The values every pass is given, as the original's `bind`.
fn common_values(block: &mut glsl::Block<'_>, world: &WorldDrawn<'_>, out_size: [u32; 2], time: f32) {
    let inv = world.view_proj.inverse();
    block.set("InvProj", &inv.to_cols_array());
    block.set("InvView", &glam::Mat4::IDENTITY.to_cols_array());
    block.set("ViewProj", &world.view_proj.to_cols_array());
    block.set("OutSize", &[out_size[0] as f32, out_size[1] as f32]);
    block.set("GTime", &[time]);
    block.set("Time", &[time]);
    // wgpu's depth runs 0 to 1, as Vulkan's.
    block.set("DepthZeroToOne", &[1.0]);
}

/// The `world_drawn` hook: the spells' passes over the world.
pub fn draw(world: &mut WorldDrawn<'_>) {
    let frame = fx::current();
    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if frame.is_none() && guard.as_ref().is_some_and(|s| s.warmed >= warm_list().len()) {
        return;
    }
    // Nothing to warm before the original's files are read.
    if frame.is_none() && fx::asset("fx/composite.fsh").is_none() {
        return;
    }
    let raw_format = world.color.format();
    if guard.as_ref().is_none_or(|s| s.frame_format != world.color_format || s.raw_format != raw_format) {
        *guard = State::new(world.device, world.color_format, raw_format);
    }
    let Some(state) = guard.as_mut() else { return };
    let Some(frame) = frame else {
        warm(state, world);
        return;
    };
    if !world.color.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        if !state.warned {
            state.warned = true;
            log::warn!("[shooting_star] this frame cannot be read back (no COPY_SRC): the spells' light is left out");
        }
        return;
    }
    run(state, world, &frame);
}

/// What the warm-up readies, one a frame from the world's first: every
/// program, every texture and the machine, so a spell's first cast does not
/// stall on its shaders.
fn warm_list() -> &'static [&'static str] {
    static LIST: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let mut list: Vec<&'static str> = vec!["composite", "bloom_down", "bloom_up"];
        list.extend(OWN_PROGRAMS.iter().map(|(name, _)| name.strip_suffix(".fsh").unwrap_or(name)));
        for file in fx::ASSET_FILES {
            if let Some(name) = file.strip_prefix("fx/").and_then(|f| f.strip_suffix(".fsh"))
                && !list.contains(&name)
            {
                list.push(name);
            }
        }
        list.extend(fx::ASSET_FILES.iter().filter(|f| f.ends_with(".jpg")).copied());
        list.push("models/gravitia/machine.bin");
        list
    })
}

fn warm(state: &mut State, world: &mut WorldDrawn<'_>) {
    let Some(item) = warm_list().get(state.warmed).copied() else { return };
    state.warmed += 1;
    if let Some(file) = item.strip_prefix("textures/") {
        state.texture(world.device, world.queue, file);
    } else if item.ends_with(".bin") {
        if state.mesh.is_none() {
            state.mesh = Some(mesh_pass(world.device));
        }
    } else {
        state.program(world.device, item);
    }
    if state.warmed == warm_list().len() {
        log::info!("[shooting_star] the effects are warmed up");
    }
}

fn run(state: &mut State, world: &mut WorldDrawn<'_>, frame: &Frame) {
    let device = world.device;
    let queue = world.queue;
    let size = world.size;
    // The quality setting: low draws the passes at 720p's pixels at most,
    // medium 1080p's, high the original's budget, ultra at full size.
    let budget = match frame.quality {
        0 => 921_600.0,
        1 => 2_073_600.0,
        3 => f64::INFINITY,
        _ => PIXEL_BUDGET,
    };
    let scale = (budget / (f64::from(size[0]) * f64::from(size[1])).max(1.0)).sqrt().min(1.0);
    let fx_size = [((f64::from(size[0]) * scale) as u32).max(2), ((f64::from(size[1]) * scale) as u32).max(2)];
    if state.targets.as_ref().is_none_or(|t| t.size != size || t.fx_size != fx_size) {
        state.targets = Some(Targets::new(device, size, fx_size, state.raw_format));
        state.stepped = None;
    }
    // Stepping: between steps the last picture is shown again, whole, as the original's `stepFps`.
    let now = std::time::Instant::now();
    let step = frame.screen.step_fps;
    if step > 0.5
        && let (Some(at), Some(targets)) = (state.stepped, state.targets.as_ref())
        && now.duration_since(at).as_secs_f32() < 1.0 / step
    {
        copy(state, world.device, world.encoder, &targets.out.view, world.color_view, &state.copy_frame);
        return;
    }
    state.stepped = (step > 0.5).then_some(now);
    let upscaled = fx_size != size;

    // Programs and textures first: they need `state` mutably.
    let mut wanted: Vec<&'static str> = frame.passes.iter().map(|p| p.program).collect();
    wanted.extend(["bloom_down", "bloom_up", "composite", "hz_streak"]);
    for name in wanted {
        state.program(device, name);
    }
    for pass in &frame.passes {
        for (_, file) in &pass.textures {
            state.texture(device, queue, file);
        }
    }
    // A uniform buffer for each draw: the passes, the bloom's and the composite.
    for i in 0..frame.passes.len() + 2 * BLOOM_LEVELS + 3 {
        state.buffer(device, i, 4096);
    }
    let Some(targets) = state.targets.as_ref() else { return };

    // The world's frame into the chain.
    world.encoder.copy_texture_to_texture(
        world.color.as_image_copy(),
        targets.raw.texture.as_image_copy(),
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shooting Star intake"),
            layout: &state.intake_binds,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&targets.raw.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(world.depth) },
            ],
        });
        let mut pass = world.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shooting Star intake"),
            color_attachments: &[Some(attachment(&targets.scene.view)), Some(attachment(&targets.depth.view))],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&state.intake);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    if upscaled {
        copy(state, world.device, world.encoder, &targets.scene.view, &targets.scene_low.view, &state.copy_gl);
    }

    // Gravitia's machine, for the passes that lay it in.
    let machine = match &frame.machine {
        Some(machine) => draw_machine(state, world, machine, fx_size),
        None => false,
    };
    let Some(targets) = state.targets.as_ref() else { return };
    let mesh_views: Vec<(&'static str, &wgpu::TextureView)> = match (machine, state.mesh_targets.as_ref()) {
        (true, Some(m)) => vec![("MeshColor", &m.color.view), ("MeshDist", &m.dist.view)],
        _ => Vec::new(),
    };

    // The spells' passes, each reading the last.
    let mut buffer_index = 0;
    let mut input: &wgpu::TextureView = &targets.scene.view;
    let mut ping = 0;
    let mut drew = false;
    for pass in &frame.passes {
        // Low quality keeps to the original's own passes.
        if frame.quality == 0 && matches!(pass.program, "hz_glow" | "hz_shock") {
            continue;
        }
        let Some(program) = state.programs.get(pass.program).and_then(Option::as_ref) else { continue };
        let mut block = glsl::Block::new(&program.layout);
        common_values(&mut block, world, fx_size, frame.time);
        let camera = world.camera;
        for (name, value) in &pass.uniforms {
            match value {
                Value::F(v) => block.set(name, v),
                Value::World(p) => block.set(name, &[(p[0] - camera.x) as f32, (p[1] - camera.y) as f32, (p[2] - camera.z) as f32]),
                Value::Vec4s(list) => {
                    for (i, v) in list.iter().enumerate() {
                        block.set_at(name, i as u32, v);
                    }
                }
            }
        }
        let mut named: Vec<(&'static str, &wgpu::TextureView)> = pass
            .textures
            .iter()
            .filter_map(|(sampler, file)| state.textures.get(file).and_then(Option::as_ref).map(|t| (*sampler, &t.view)))
            .collect();
        named.extend(mesh_views.iter().copied());
        let inputs = Inputs { diffuse: input, aux: &targets.scene.view, named };
        let output = &targets.ping[ping];
        draw_program(state, world, program, &block, &inputs, &targets.depth.view, &targets.black.view, &output.view, buffer_index);
        buffer_index += 1;
        input = &output.view;
        ping ^= 1;
        drew = true;
    }

    // The bloom: five levels down from the spells' light, four back up.
    let screen = frame.screen;
    let bloom_on = screen.bloom > 0.001;
    let mut bloom: Option<&wgpu::TextureView> = None;
    if bloom_on
        && let (Some(down), Some(up)) = (
            state.programs.get("bloom_down").and_then(Option::as_ref),
            state.programs.get("bloom_up").and_then(Option::as_ref),
        )
    {
        let mut source = input;
        let mut source_size = fx_size;
        for i in 0..BLOOM_LEVELS {
            let mut block = glsl::Block::new(&down.layout);
            common_values(&mut block, world, targets.down[i].size, frame.time);
            block.set("TexelSize", &[1.0 / source_size[0] as f32, 1.0 / source_size[1] as f32]);
            block.set("Prefilter", &[if i == 0 { 1.0 } else { 0.0 }]);
            block.set("Synthetic", &[screen.synthetic]);
            block.set("MaskAlpha", &[if drew { 1.0 } else { 0.0 }]);
            let inputs = Inputs { diffuse: source, aux: &targets.scene.view, named: Vec::new() };
            draw_program(state, world, down, &block, &inputs, &targets.depth.view, &targets.black.view, &targets.down[i].view, buffer_index);
            buffer_index += 1;
            source = &targets.down[i].view;
            source_size = targets.down[i].size;
        }
        for i in (0..BLOOM_LEVELS - 1).rev() {
            let mut block = glsl::Block::new(&up.layout);
            common_values(&mut block, world, targets.up[i].size, frame.time);
            block.set("TexelSize", &[1.0 / source_size[0] as f32, 1.0 / source_size[1] as f32]);
            block.set("Radius", &[screen.bloom_radius.max(1.0)]);
            let inputs = Inputs { diffuse: source, aux: &targets.down[i].view, named: Vec::new() };
            draw_program(state, world, up, &block, &inputs, &targets.depth.view, &targets.black.view, &targets.up[i].view, buffer_index);
            buffer_index += 1;
            source = &targets.up[i].view;
            source_size = targets.up[i].size;
        }
        bloom = Some(source);
        // Anamorphic streaks out of the brightest light (high and ultra).
        if frame.quality >= 2
            && let Some(streak) = state.programs.get("hz_streak").and_then(Option::as_ref)
        {
            let mut block = glsl::Block::new(&streak.layout);
            common_values(&mut block, world, targets.streak.size, frame.time);
            let small = &targets.down[1.min(BLOOM_LEVELS - 1)];
            block.set("TexelSize", &[1.0 / small.size[0] as f32, 1.0 / small.size[1] as f32]);
            block.set("Strength", &[(screen.bloom * if frame.quality >= 3 { 0.5 } else { 0.35 }).min(1.0)]);
            let inputs = Inputs { diffuse: source, aux: &small.view, named: Vec::new() };
            draw_program(state, world, streak, &block, &inputs, &targets.depth.view, &targets.black.view, &targets.streak.view, buffer_index);
            buffer_index += 1;
            bloom = Some(&targets.streak.view);
        }
    }

    // The composite, onto a picture of the frame's size, then onto the frame.
    let Some(composite) = state.programs.get("composite").and_then(Option::as_ref) else { return };
    let mut block = glsl::Block::new(&composite.layout);
    common_values(&mut block, world, size, frame.time);
    block.set("AutoExposure", &[if bloom.is_some() { 1.0 } else { 0.0 }]);
    block.set("Upscaled", &[if upscaled { 1.0 - screen.synthetic.min(1.0) } else { 0.0 }]);
    block.set("Grade", &[screen.grade[0], screen.grade[1], 0.0, screen.grade[3]]);
    block.set("Bloom", &[if bloom.is_some() { screen.bloom } else { 0.0 }]);
    block.set("Impact", &screen.impact);
    block.set("Shake", &[screen.shake]);
    block.set("Aberration", &[screen.aberration]);
    block.set("Desaturate", &[screen.desaturate]);
    block.set("ZoomBlur", &[screen.zoom_blur]);
    block.set("Flash", &screen.flash);
    block.set("Vignette", &screen.vignette);
    let scene_low = if upscaled { &targets.scene_low.view } else { &targets.scene.view };
    let exposure = if bloom.is_some() { &targets.down[BLOOM_LEVELS - 1].view } else { &targets.black.view };
    let inputs = Inputs {
        diffuse: input,
        aux: bloom.unwrap_or(&targets.black.view),
        named: vec![
            ("SceneSampler", &targets.scene.view),
            ("SceneLowSampler", scene_low),
            ("ExposureSampler", exposure),
            ("HoldSampler", &targets.scene.view),
        ],
    };
    draw_program(state, world, composite, &block, &inputs, &targets.depth.view, &targets.black.view, &targets.out.view, buffer_index);
    copy(state, world.device, world.encoder, &targets.out.view, world.color_view, &state.copy_frame);
}

fn attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
    }
}

fn copy(
    state: &State,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    from: &wgpu::TextureView,
    to: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
) {
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Shooting Star copy"),
        layout: &state.copy_binds,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(from) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&state.linear) },
        ],
    });
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Shooting Star copy"),
        color_attachments: &[Some(attachment(to))],
        depth_stencil_attachment: None,
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind, &[]);
    pass.draw(0..3, 0..1);
}

#[allow(clippy::too_many_arguments)]
fn draw_program(
    state: &State,
    world: &mut WorldDrawn<'_>,
    program: &Program,
    block: &glsl::Block<'_>,
    inputs: &Inputs<'_>,
    depth: &wgpu::TextureView,
    black: &wgpu::TextureView,
    output: &wgpu::TextureView,
    buffer_index: usize,
) {
    let device = world.device;
    let buffer = match state.buffers.get(buffer_index) {
        Some(b) if b.size() >= block.bytes.len() as u64 => b.clone(),
        _ => return,
    };
    world.queue.write_buffer(&buffer, 0, &block.bytes);
    let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }];
    for (name, binding) in &program.layout.samplers {
        let (view, sampler) = match name.as_str() {
            "DiffuseSampler" => (inputs.diffuse, &state.linear),
            "DepthSampler" => (depth, &state.nearest),
            "AuxSampler" => (inputs.aux, &state.linear),
            other => match inputs.named.iter().find(|(n, _)| *n == other) {
                Some((_, view)) => (*view, if other.ends_with("Sampler") { &state.linear } else { &state.repeat }),
                None => (black, &state.linear),
            },
        };
        entries.push(wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::TextureView(view) });
        entries.push(wgpu::BindGroupEntry { binding: binding + 1, resource: wgpu::BindingResource::Sampler(sampler) });
    }
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Shooting Star program"),
        layout: &program.binds,
        entries: &entries,
    });
    let mut pass = world.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Shooting Star program"),
        color_attachments: &[Some(attachment(output))],
        depth_stencil_attachment: None,
        ..Default::default()
    });
    pass.set_pipeline(&program.pipeline);
    pass.set_bind_group(0, &bind, &[]);
    pass.draw(0..3, 0..1);
}

// ---- SS-06 Gravitia's machine -----------------------------------------------------------------------------------
//
// The original draws its ring engine (`models/gravitia/machine.bin`, twenty
// parts) with a mesh program of its own into a colour picture and a distance
// picture, which `grav_world` lays into the sky and `grav_space` into the
// film. The same, on wgpu: one indexed draw a part, lit as the original lights
// it (the Sun unless the Earth hides it, the Earth's blue day side, the core's
// light, what the metal mirrors), into `MeshColor` and `MeshDist`.

const MESH_DIST: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const MESH_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Bytes between two parts' uniforms (dynamic offsets are 256-aligned everywhere).
const PART_STRIDE: u64 = 256;
const MAX_PARTS: usize = 32;
const MAX_MATERIALS: usize = 32;

const MACHINE_WGSL: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_col: vec4<f32>,
    earth_rel: vec4<f32>,
    core: vec4<f32>,
    behind: vec4<f32>,
    mats: array<vec4<f32>, 64>,
}
struct Part {
    model: mat4x4<f32>,
    qmin: vec4<f32>,
    qext: vec4<f32>,
}
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<uniform> part: Part;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) oct: vec2<f32>,
    @location(2) mat: vec2<f32>,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) @interpolate(flat) mat_id: u32,
    @location(3) ao: f32,
}

fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var v = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (v.z < 0.0) {
        let s = vec2<f32>(select(-1.0, 1.0, v.x >= 0.0), select(-1.0, 1.0, v.y >= 0.0));
        v = vec3<f32>((1.0 - abs(v.yx)) * s, v.z);
    }
    return normalize(v);
}

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var p = part.qmin.xyz + v.pos * part.qext.xyz;
    // in the sky a ring's band is drawn thinner round its own middle
    let k = g.behind.w;
    if (k < 0.999) {
        let r = length(p.xy);
        if (r > 1.0e-4) {
            let mid = part.qext.w;
            p = vec3<f32>(p.xy * ((mid + (r - mid) * k) / r), p.z * k);
        }
    }
    let w = part.model * vec4<f32>(p, 1.0);
    var o: VsOut;
    o.rel = w.xyz;
    o.nrm = mat3x3<f32>(part.model[0].xyz, part.model[1].xyz, part.model[2].xyz) * oct_decode(v.oct);
    o.mat_id = min(u32(v.mat.x * 255.0 + 0.5), 31u);
    o.ao = v.mat.y;
    // into the chain's pictures, kept as OpenGL keeps them
    let c = g.view_proj * w;
    o.clip = vec4<f32>(c.x, -c.y, c.z, c.w);
    return o;
}

fn ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (3.14159 * d * d);
}

fn light_on(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, col: vec3<f32>, albedo: vec3<f32>, metal: f32, rough: f32) -> vec3<f32> {
    let nl = max(dot(n, l), 0.0);
    if (nl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let fr = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    let a = max(rough * rough, 0.02);
    let spec = fr * ggx(nh, a) * 0.25 / max(nl * max(dot(n, v), 0.05), 0.05) * nl;
    let diff = albedo * (1.0 - metal) * nl / 3.14159;
    return (diff + spec) * col;
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @location(1) dist: vec4<f32>,
}

@fragment
fn fs_main(i: VsOut) -> FsOut {
    let m0 = g.mats[i.mat_id * 2u];
    let m1 = g.mats[i.mat_id * 2u + 1u];
    let albedo = m0.rgb;
    let metal = m0.a;
    let rough = clamp(m1.x, 0.04, 1.0);
    var n = normalize(i.nrm);
    let v = normalize(-i.rel);
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    let p = g.cam_pos.xyz + i.rel;
    var col = vec3<f32>(0.0);
    // the Sun, unless the Earth stands between
    let b = dot(p, g.sun_dir.xyz);
    let c = dot(p, p) - 1.0;
    let shade = g.cam_pos.w > 0.5 && b < 0.0 && b * b - c > 0.0;
    if (!shade) {
        col += light_on(n, v, g.sun_dir.xyz, g.sun_col.xyz * 3.14159, albedo, metal, rough);
    }
    // the Earth's day side glowing blue on the faces turned to it
    let to_e = normalize(-p);
    let day = clamp(dot(-to_e, g.sun_dir.xyz) * 0.5 + 0.5, 0.0, 1.0);
    col += albedo * (1.0 - metal * 0.6) * max(dot(n, to_e), 0.0) * vec3<f32>(0.25, 0.42, 0.8) * 0.35 * day;
    // the light at the core when it charges
    let dl = g.earth_rel.xyz - i.rel;
    let dd = max(length(dl), 1.0e-4);
    let reach = max(g.core.w, 1.0e-3);
    let lc = g.core.rgb / (1.0 + (dd / reach) * (dd / reach) * 2.0);
    col += light_on(n, v, dl / dd, lc, albedo, metal, rough);
    // what the metal mirrors: dark space, a violet glow from the nebula behind
    let r = reflect(-v, n);
    let env = vec3<f32>(0.004, 0.004, 0.008) + vec3<f32>(0.12, 0.07, 0.25) * pow(max(dot(r, g.behind.xyz), 0.0), 3.0)
        + vec3<f32>(0.2, 0.35, 0.7) * 0.4 * day * pow(max(dot(r, to_e), 0.0), 8.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    col += env * mix(f0, vec3<f32>(1.0), pow(1.0 - max(dot(n, v), 0.0), 5.0)) * (1.0 - rough * 0.7);
    col += albedo * vec3<f32>(0.012, 0.012, 0.02);
    col *= mix(0.35, 1.0, i.ao);
    col += albedo * m1.y * part.qmin.w;
    var o: FsOut;
    o.color = vec4<f32>(col, 1.0);
    o.dist = vec4<f32>(length(i.rel), 0.0, 0.0, 1.0);
    return o;
}
"#;

/// One part of the machine: its indices, and the box its quantised positions fill.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPart {
    pub name: String,
    pub first: u32,
    pub count: u32,
    pub base: i32,
    pub qmin: [f32; 3],
    pub qext: [f32; 3],
}

/// `machine.bin` read: materials (albedo, metal, rough, glow), parts, and the
/// vertices (position 0..1 in the part's box, octahedral normal, material and
/// occlusion) as plain floats, seven a vertex.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshFile {
    pub materials: Vec<[f32; 6]>,
    pub parts: Vec<MeshPart>,
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

/// Reads the original's machine file ("GRV1", little-endian).
pub fn parse_machine(bytes: &[u8]) -> Result<MeshFile, String> {
    struct Reader<'a> {
        b: &'a [u8],
        at: usize,
    }
    impl Reader<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8], String> {
            let end = self.at.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("the machine file is cut short")?;
            let s = &self.b[self.at..end];
            self.at = end;
            Ok(s)
        }
        fn u8(&mut self) -> Result<u8, String> {
            Ok(self.take(1)?[0])
        }
        fn i32(&mut self) -> Result<i32, String> {
            Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap_or_default()))
        }
        fn f32(&mut self) -> Result<f32, String> {
            Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap_or_default()))
        }
        fn u16(&mut self) -> Result<u16, String> {
            Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap_or_default()))
        }
        fn name(&mut self) -> Result<String, String> {
            let n = usize::from(self.u8()?);
            Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
        }
        fn count(&mut self) -> Result<usize, String> {
            usize::try_from(self.i32()?).map_err(|_| "a negative count in the machine file".to_owned())
        }
    }
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(4)? != b"GRV1" {
        return Err("not a machine file".into());
    }
    let mut out = MeshFile::default();
    for _ in 0..r.count()? {
        r.name()?;
        let mut m = [0.0; 6];
        for v in &mut m {
            *v = r.f32()?;
        }
        out.materials.push(m);
    }
    for _ in 0..r.count()? {
        let name = r.name()?;
        r.i32()?;
        for _ in 0..3 {
            r.f32()?;
        }
        let qmin = [r.f32()?, r.f32()?, r.f32()?];
        let qmax = [r.f32()?, r.f32()?, r.f32()?];
        let vertices = r.count()?;
        let indices = r.count()?;
        let base = out.vertices.len() / 7;
        for _ in 0..vertices {
            let (x, y, z) = (r.u16()?, r.u16()?, r.u16()?);
            let (ox, oy) = (r.u8()? as i8, r.u8()? as i8);
            let (mat, ao) = (r.u8()?, r.u8()?);
            out.vertices.extend_from_slice(&[
                f32::from(x) / 65535.0,
                f32::from(y) / 65535.0,
                f32::from(z) / 65535.0,
                (f32::from(ox) / 127.0).max(-1.0),
                (f32::from(oy) / 127.0).max(-1.0),
                f32::from(mat) / 255.0,
                f32::from(ao) / 255.0,
            ]);
        }
        let first = out.indices.len() as u32;
        for _ in 0..indices {
            let i = if vertices > 65535 { r.i32()? as u32 } else { u32::from(r.u16()?) };
            out.indices.push(i);
        }
        out.parts.push(MeshPart {
            name,
            first,
            count: indices as u32,
            base: base as i32,
            qmin,
            qext: [qmax[0] - qmin[0], qmax[1] - qmin[1], qmax[2] - qmin[2]],
        });
    }
    Ok(out)
}

struct MeshPass {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    parts_buffer: wgpu::Buffer,
    parts_bind: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    file: MeshFile,
}

struct MeshTargets {
    size: [u32; 2],
    color: Target,
    dist: Target,
    depth: wgpu::TextureView,
}

fn mesh_targets(device: &wgpu::Device, size: [u32; 2]) -> MeshTargets {
    let none = wgpu::TextureUsages::empty();
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Shooting Star machine depth"),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: MESH_DEPTH,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    MeshTargets {
        size,
        color: target(device, "Shooting Star machine", size, HDR, none),
        dist: target(device, "Shooting Star machine distance", size, MESH_DIST, none),
        depth: depth.create_view(&wgpu::TextureViewDescriptor::default()),
    }
}

fn mesh_pass(device: &wgpu::Device) -> Option<MeshPass> {
    use wgpu::util::DeviceExt as _;
    let bytes = fx::asset("models/gravitia/machine.bin")?;
    let file = match parse_machine(bytes) {
        Ok(file) => file,
        Err(error) => {
            log::error!("[shooting_star] Gravitia's machine could not be read: {error}");
            return None;
        }
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Shooting Star machine"),
        source: wgpu::ShaderSource::Wgsl(MACHINE_WGSL.into()),
    });
    let uniform = |binding: u32, dynamic: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: None },
        count: None,
    };
    let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Shooting Star machine globals"),
        entries: &[uniform(0, false)],
    });
    let parts_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Shooting Star machine part"),
        entries: &[uniform(0, true)],
    });
    let globals = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Shooting Star machine globals"),
        size: 2048,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let parts_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Shooting Star machine parts"),
        size: PART_STRIDE * MAX_PARTS as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Shooting Star machine globals"),
        layout: &globals_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
    });
    let parts_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Shooting Star machine part"),
        layout: &parts_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &parts_buffer,
                offset: 0,
                size: std::num::NonZeroU64::new(96),
            }),
        }],
    });
    let floats: Vec<u8> = file.vertices.iter().flat_map(|f| f.to_le_bytes()).collect();
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Shooting Star machine vertices"),
        contents: &floats,
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_bytes: Vec<u8> = file.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Shooting Star machine indices"),
        contents: &index_bytes,
        usage: wgpu::BufferUsages::INDEX,
    });
    let attributes = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 20, shader_location: 2 },
    ];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Shooting Star machine"),
        layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shooting Star machine"),
            bind_group_layouts: &[Some(&globals_layout), Some(&parts_layout)],
            immediate_size: 0,
        })),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 28,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MESH_DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[color_target(HDR), color_target(MESH_DIST)],
        }),
        multiview_mask: None,
        cache: None,
    });
    if let Some(error) = pollster::block_on(scope.pop()) {
        log::error!("[shooting_star] Gravitia's machine program was refused: {error}");
        return None;
    }
    log::info!("[shooting_star] Gravitia's machine is ready ({} parts, {} triangles)", file.parts.len(), file.indices.len() / 3);
    Some(MeshPass { pipeline, globals, globals_bind, parts_buffer, parts_bind, vertices, indices, file })
}

fn push_floats(out: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// The machine, posed, into the mesh pictures. False when it is not drawn.
fn draw_machine(state: &mut State, world: &mut WorldDrawn<'_>, machine: &fx::Machine, size: [u32; 2]) -> bool {
    let device = world.device;
    if state.mesh.is_none() {
        state.mesh = Some(mesh_pass(device));
    }
    let Some(Some(mesh)) = state.mesh.as_ref() else { return false };
    if state.mesh_targets.as_ref().is_none_or(|t| t.size != size) {
        state.mesh_targets = Some(mesh_targets(device, size));
    }
    let Some(targets) = state.mesh_targets.as_ref() else { return false };
    let view_proj = machine.view_proj.unwrap_or_else(|| world.view_proj.to_cols_array());
    let mut globals = Vec::with_capacity(1184);
    push_floats(&mut globals, &view_proj);
    let earth_shade = if machine.earth_shade { 1.0 } else { 0.0 };
    push_floats(&mut globals, &[machine.cam_pos[0], machine.cam_pos[1], machine.cam_pos[2], earth_shade]);
    push_floats(&mut globals, &[machine.sun_dir[0], machine.sun_dir[1], machine.sun_dir[2], 0.0]);
    push_floats(&mut globals, &[machine.sun_col[0], machine.sun_col[1], machine.sun_col[2], 0.0]);
    push_floats(&mut globals, &[machine.earth_rel[0], machine.earth_rel[1], machine.earth_rel[2], 0.0]);
    push_floats(&mut globals, &machine.core);
    push_floats(&mut globals, &[machine.behind[0], machine.behind[1], machine.behind[2], machine.thin_k]);
    for i in 0..MAX_MATERIALS {
        let m = mesh.file.materials.get(i).copied().unwrap_or([0.0; 6]);
        push_floats(&mut globals, &[m[0], m[1], m[2], m[3], m[4], m[5], 0.0, 0.0]);
    }
    world.queue.write_buffer(&mesh.globals, 0, &globals);
    let count = mesh.file.parts.len().min(MAX_PARTS);
    let mut parts = vec![0u8; PART_STRIDE as usize * count];
    for (i, p) in mesh.file.parts.iter().take(count).enumerate() {
        let mut one = Vec::with_capacity(96);
        let model = machine.models.get(i).or(machine.models.last()).copied().unwrap_or([0.0; 16]);
        push_floats(&mut one, &model);
        let glow = machine.glow.get(i).copied().unwrap_or(0.0);
        let thin = machine.thin.get(i).copied().unwrap_or(1.0);
        push_floats(&mut one, &[p.qmin[0], p.qmin[1], p.qmin[2], glow]);
        push_floats(&mut one, &[p.qext[0], p.qext[1], p.qext[2], thin]);
        let at = i * PART_STRIDE as usize;
        parts[at..at + one.len()].copy_from_slice(&one);
    }
    world.queue.write_buffer(&mesh.parts_buffer, 0, &parts);
    let clear = |view| wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
    };
    let mut pass = world.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Shooting Star machine"),
        color_attachments: &[Some(clear(&targets.color.view)), Some(clear(&targets.dist.view))],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &targets.depth,
            // The world's projection is reversed: 1 near, 0 at the sky.
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    pass.set_pipeline(&mesh.pipeline);
    pass.set_bind_group(0, &mesh.globals_bind, &[]);
    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
    for (i, p) in mesh.file.parts.iter().take(count).enumerate() {
        pass.set_bind_group(1, &mesh.parts_bind, &[(i as u64 * PART_STRIDE) as u32]);
        pass.draw_indexed(p.first..p.first + p.count, p.base, 0..1);
    }
    true
}
