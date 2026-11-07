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
    fx::asset(&format!("fx/{name}")).and_then(|b| std::str::from_utf8(b).ok())
}

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
    let Some(frame) = fx::current() else { return };
    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let raw_format = world.color.format();
    if guard.as_ref().is_none_or(|s| s.frame_format != world.color_format || s.raw_format != raw_format) {
        *guard = State::new(world.device, world.color_format, raw_format);
    }
    let Some(state) = guard.as_mut() else { return };
    if !world.color.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        if !state.warned {
            state.warned = true;
            log::warn!("[shooting_star] this frame cannot be read back (no COPY_SRC): the spells' light is left out");
        }
        return;
    }
    run(state, world, &frame);
}

fn run(state: &mut State, world: &mut WorldDrawn<'_>, frame: &Frame) {
    let device = world.device;
    let queue = world.queue;
    let size = world.size;
    let scale = (PIXEL_BUDGET / (f64::from(size[0]) * f64::from(size[1])).max(1.0)).sqrt().min(1.0);
    let fx_size = [((f64::from(size[0]) * scale) as u32).max(2), ((f64::from(size[1]) * scale) as u32).max(2)];
    if state.targets.as_ref().is_none_or(|t| t.size != size || t.fx_size != fx_size) {
        state.targets = Some(Targets::new(device, size, fx_size, state.raw_format));
    }
    let upscaled = fx_size != size;

    // Programs and textures first: they need `state` mutably.
    let mut wanted: Vec<&'static str> = frame.passes.iter().map(|p| p.program).collect();
    wanted.extend(["bloom_down", "bloom_up", "composite"]);
    for name in wanted {
        state.program(device, name);
    }
    for pass in &frame.passes {
        for (_, file) in &pass.textures {
            state.texture(device, queue, file);
        }
    }
    // A uniform buffer for each draw: the passes, the bloom's and the composite.
    for i in 0..frame.passes.len() + 2 * BLOOM_LEVELS + 2 {
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

    // The spells' passes, each reading the last.
    let mut buffer_index = 0;
    let mut input: &wgpu::TextureView = &targets.scene.view;
    let mut ping = 0;
    let mut drew = false;
    for pass in &frame.passes {
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
        let named: Vec<(&'static str, &wgpu::TextureView)> = pass
            .textures
            .iter()
            .filter_map(|(sampler, file)| state.textures.get(file).and_then(Option::as_ref).map(|t| (*sampler, &t.view)))
            .collect();
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
