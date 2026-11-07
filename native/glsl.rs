// The original's own GLSL, compiled for Horizon: its fx shaders were written
// for Minecraft's OpenGL (`#version 150`, loose `uniform`s, combined
// `sampler2D`s), and naga's GLSL frontend (which Horizon's render crate
// carries for shader packs) wants Vulkan-style GLSL. This turns one into the
// other and hands back a naga module, with the place of every uniform in its
// one std140 block so the renderer can fill it by name.
//
// The rewrite:
// - `#include "x.glsl"` pasted in, `#version` taken out;
// - every loose `uniform` moved into `layout(std140, set = 0, binding = 0)
//   uniform HzUniforms { ... }`, whose members keep their bare names;
// - every `uniform sampler2D X;` split into a texture and a sampler at the
//   next two bindings, `X` becoming `sampler2D(X_hz_tex, X_hz_smp)` again;
// - `in`/`out` given location 0.

/// A shader's source by name (`star_beam.fsh`, `star.glsl`).
pub type Sources = fn(&str) -> Option<&'static str>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Float,
    Int,
    Vec2,
    Vec3,
    Vec4,
    Mat3,
    Mat4,
}

impl Ty {
    fn parse(word: &str) -> Option<Ty> {
        Some(match word {
            "float" => Ty::Float,
            "int" => Ty::Int,
            "vec2" => Ty::Vec2,
            "vec3" => Ty::Vec3,
            "vec4" => Ty::Vec4,
            "mat3" => Ty::Mat3,
            "mat4" => Ty::Mat4,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Ty::Float => "float",
            Ty::Int => "int",
            Ty::Vec2 => "vec2",
            Ty::Vec3 => "vec3",
            Ty::Vec4 => "vec4",
            Ty::Mat3 => "mat3",
            Ty::Mat4 => "mat4",
        }
    }

    /// std140: (alignment, size) of one.
    fn layout(self) -> (u32, u32) {
        match self {
            Ty::Float | Ty::Int => (4, 4),
            Ty::Vec2 => (8, 8),
            Ty::Vec3 => (16, 12),
            Ty::Vec4 => (16, 16),
            Ty::Mat3 => (16, 48),
            Ty::Mat4 => (16, 64),
        }
    }
}

/// One member of the uniform block.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    pub ty: Ty,
    /// Elements when it is an array.
    pub count: Option<u32>,
    pub offset: u32,
}

/// A program's uniforms: where each lives, and the block's whole size.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub members: Vec<Member>,
    pub size: u32,
    /// Each sampler's texture binding; its sampler is the next.
    pub samplers: Vec<(String, u32)>,
}

impl Layout {
    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name)
    }
}

/// Fills a block by name. Names the program lacks are left out, so one set of
/// values can feed every pass.
#[derive(Clone, Debug)]
pub struct Block<'a> {
    layout: &'a Layout,
    pub bytes: Vec<u8>,
}

impl<'a> Block<'a> {
    pub fn new(layout: &'a Layout) -> Block<'a> {
        Block { layout, bytes: vec![0; layout.size.max(16) as usize] }
    }

    fn put(&mut self, at: u32, values: &[f32]) {
        for (i, v) in values.iter().enumerate() {
            let o = at as usize + i * 4;
            if o + 4 <= self.bytes.len() {
                self.bytes[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }

    /// A float, vector or (column-major) matrix: as many floats as it holds.
    pub fn set(&mut self, name: &str, values: &[f32]) {
        let Some(m) = self.layout.member(name).cloned() else { return };
        match m.ty {
            Ty::Int => {
                let o = m.offset as usize;
                if o + 4 <= self.bytes.len()
                    && let Some(v) = values.first()
                {
                    self.bytes[o..o + 4].copy_from_slice(&(*v as i32).to_le_bytes());
                }
            }
            Ty::Mat3 => {
                for c in 0..3 {
                    if values.len() >= c * 3 + 3 {
                        self.put(m.offset + c as u32 * 16, &values[c * 3..c * 3 + 3]);
                    }
                }
            }
            _ => {
                let n = match m.ty {
                    Ty::Float => 1,
                    Ty::Vec2 => 2,
                    Ty::Vec3 => 3,
                    Ty::Vec4 => 4,
                    Ty::Mat4 => 16,
                    _ => 0,
                };
                self.put(m.offset, &values[..values.len().min(n)]);
            }
        }
    }

    /// Element `index` of an array uniform (`Pts[3]`).
    pub fn set_at(&mut self, name: &str, index: u32, values: &[f32]) {
        let Some(m) = self.layout.member(name).cloned() else { return };
        let Some(count) = m.count else { return };
        if index >= count {
            return;
        }
        let stride = m.ty.layout().1.div_ceil(16) * 16;
        self.put(m.offset + index * stride, &values[..values.len().min(4)]);
    }
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Comments blanked to spaces (same length, so offsets keep).
fn blank_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                if b[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            for _ in 0..2 {
                if i < b.len() {
                    out[i] = b' ';
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// `#include "name"` pasted in, once each.
fn include(sources: Sources, name: &str, seen: &mut Vec<String>, depth: u32) -> Result<String, String> {
    if depth > 8 {
        return Err(format!("{name}: includes nest too deep"));
    }
    if seen.iter().any(|s| s == name) {
        return Ok(String::new());
    }
    seen.push(name.to_owned());
    let src = sources(name).ok_or_else(|| format!("no shader named {name}"))?;
    let mut out = String::with_capacity(src.len());
    for line in blank_comments(src).lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("#include") {
            let inner = rest.trim().trim_matches(|c| c == '"' || c == '<' || c == '>');
            out.push_str(&include(sources, inner, seen, depth + 1)?);
            out.push('\n');
        } else if t.starts_with("#version") {
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// The rewritten source and its layout, before naga sees it.
pub fn preprocess(sources: Sources, entry: &str, defines: &[(&str, &str)]) -> Result<(String, Layout), String> {
    let mut seen = Vec::new();
    let mut body = include(sources, "common.glsl", &mut seen, 0)?;
    body.push_str(&include(sources, entry, &mut seen, 0)?);

    // Every `uniform` at the top level, block members and samplers apart.
    let defs = int_defines(&body);
    let mut members: Vec<(String, Ty, Option<u32>)> = Vec::new();
    let mut samplers: Vec<String> = Vec::new();
    let mut out = String::with_capacity(body.len() + 2048);
    let mut depth = 0i32;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
        }
        let word_start = depth == 0 && (i == 0 || !is_ident(bytes[i - 1])) && body[i..].starts_with("uniform") && !bytes.get(i + 7).is_some_and(|c| is_ident(*c));
        if word_start {
            let end = body[i..].find(';').map(|e| i + e).ok_or("a uniform without its ;")?;
            let decl = body[i + 7..end].trim();
            let mut parts = decl.splitn(2, char::is_whitespace);
            let ty = parts.next().unwrap_or("");
            let names = parts.next().unwrap_or("");
            for raw in names.split(',') {
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                let (name, count) = match raw.find('[') {
                    Some(b) => {
                        let n = raw[b + 1..].trim_end_matches(']').trim();
                        let size = n.parse::<u32>().ok().or_else(|| defs.get(n).copied()).ok_or_else(|| format!("array size {n}"))?;
                        (raw[..b].trim().to_owned(), Some(size))
                    }
                    None => (raw.to_owned(), None),
                };
                if ty == "sampler2D" {
                    if !samplers.contains(&name) {
                        samplers.push(name);
                    }
                } else {
                    let t = Ty::parse(ty).ok_or_else(|| format!("uniform type {ty} of {name}"))?;
                    match members.iter().find(|(n, _, _)| *n == name) {
                        Some((_, t0, c0)) if *t0 != t || *c0 != count => return Err(format!("uniform {name} declared twice, differently")),
                        Some(_) => {}
                        None => members.push((name, t, count)),
                    }
                }
            }
            // Keep the line count: the declaration becomes blanks.
            for b in &bytes[i..=end] {
                out.push(if *b == b'\n' { '\n' } else { ' ' });
            }
            i = end + 1;
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    // Non-ASCII never appears in the original's shaders; keep it safe anyway.
    let out = if out.is_ascii() { out } else { out.chars().filter(char::is_ascii).collect() };

    // The block, in std140.
    let mut layout = Layout::default();
    let mut offset = 0u32;
    let mut block = String::from("layout(std140, set = 0, binding = 0) uniform HzUniforms {\n");
    for (name, ty, count) in &members {
        let (align, size) = ty.layout();
        let (align, size) = match count {
            Some(n) => (16, size.div_ceil(16) * 16 * n),
            None => (align, size),
        };
        offset = offset.div_ceil(align) * align;
        layout.members.push(Member { name: name.clone(), ty: *ty, count: *count, offset });
        match count {
            Some(n) => block.push_str(&format!("    {} {}[{}];\n", ty.name(), name, n)),
            None => block.push_str(&format!("    {} {};\n", ty.name(), name)),
        }
        offset += size;
    }
    if members.is_empty() {
        block.push_str("    vec4 HzUnused;\n");
        offset = 16;
    }
    block.push_str("};\n");
    layout.size = offset.div_ceil(16) * 16;
    let mut head = String::from("#version 450\n");
    for (name, value) in defines {
        head.push_str(&format!("#define {name} {value}\n"));
    }
    head.push_str(&block);
    for (k, name) in samplers.iter().enumerate() {
        let binding = 1 + 2 * k as u32;
        head.push_str(&format!(
            "layout(set = 0, binding = {binding}) uniform texture2D {name}_hz_tex;\n\
             layout(set = 0, binding = {}) uniform sampler {name}_hz_smp;\n",
            binding + 1
        ));
        layout.samplers.push((name.clone(), binding));
    }
    let body = split_samplers(&out, &samplers)
        .replace("in vec2 texCoord;", "layout(location = 0) in vec2 texCoord;")
        .replace("out vec4 fragColor;", "layout(location = 0) out vec4 fragColor;");
    Ok((format!("{head}{body}"), layout))
}

/// `#define NAME 48`: the integer ones, for array sizes.
fn int_defines(src: &str) -> std::collections::HashMap<String, u32> {
    let mut out = std::collections::HashMap::new();
    for line in src.lines() {
        let mut words = line.split_whitespace();
        if words.next() == Some("#define")
            && let (Some(name), Some(value)) = (words.next(), words.next())
            && let Ok(v) = value.parse::<u32>()
        {
            out.insert(name.to_owned(), v);
        }
    }
    out
}

/// Identifiers and the rest, in order: `(is_ident, start, end)`.
fn tokens(src: &str) -> Vec<(bool, usize, usize)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if is_ident(b[i]) && !b[i].is_ascii_digit() {
            let s = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            out.push((true, s, i));
        } else if b[i].is_ascii_digit() {
            // A number, with its exponent and suffix, is never a name.
            let s = i;
            while i < b.len() && (is_ident(b[i]) || b[i] == b'.' || ((b[i] == b'-' || b[i] == b'+') && (b[i - 1] == b'e' || b[i - 1] == b'E'))) {
                i += 1;
            }
            out.push((false, s, i));
        } else {
            out.push((false, i, i + 1));
            i += 1;
        }
    }
    out
}

/// GLSL for Vulkan has no combined sampler a function can take: a
/// `sampler2D` parameter becomes a texture and a sampler, a sampler passed to
/// such a function goes as its two halves, and anywhere else a sampler is
/// `sampler2D(texture, sampler)` made on the spot.
fn split_samplers(src: &str, globals: &[String]) -> String {
    let toks = tokens(src);
    let word = |k: usize| &src[toks[k].1..toks[k].2];
    let next_solid = |mut k: usize| {
        while k < toks.len() && !toks[k].0 && src[toks[k].1..toks[k].2].trim().is_empty() {
            k += 1;
        }
        k
    };
    let prev_solid = |k: usize| {
        let mut k = k as isize - 1;
        while k >= 0 && !toks[k as usize].0 && src[toks[k as usize].1..toks[k as usize].2].trim().is_empty() {
            k -= 1;
        }
        k
    };
    // The functions taking a sampler: name -> which parameters are samplers.
    let mut takes: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    // Parameters to split, by token index, and their names within each body.
    let mut param_at: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut scoped: Vec<(usize, usize, String)> = Vec::new();
    let mut k = 0;
    while k + 2 < toks.len() {
        if toks[k].0 && word(k) == "sampler2D" {
            let name_k = next_solid(k + 1);
            if name_k < toks.len() && toks[name_k].0 {
                let p = prev_solid(k);
                if p >= 0 && (word(p as usize) == "(" || word(p as usize) == ",") {
                    // Inside a parameter list: find its function.
                    let mut depth = 0;
                    let mut j = p;
                    let mut index = 0;
                    while j >= 0 {
                        let w = word(j as usize);
                        if w == ")" {
                            depth += 1;
                        } else if w == "(" {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        } else if w == "," && depth == 0 {
                            index += 1;
                        }
                        j -= 1;
                    }
                    let f = prev_solid(j.max(0) as usize);
                    if f >= 0 && toks[f as usize].0 {
                        takes.entry(word(f as usize).to_owned()).or_default().push(index);
                        param_at.insert(k);
                        // The body: from the next `{` to its match.
                        let mut b = name_k;
                        while b < toks.len() && word(b) != "{" && word(b) != ";" {
                            b += 1;
                        }
                        if b < toks.len() && word(b) == "{" {
                            let mut d = 0;
                            let mut e = b;
                            while e < toks.len() {
                                match word(e) {
                                    "{" => d += 1,
                                    "}" => {
                                        d -= 1;
                                        if d == 0 {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                                e += 1;
                            }
                            scoped.push((b, e, word(name_k).to_owned()));
                        }
                    }
                }
            }
        }
        k += 1;
    }
    let is_sampler = |k: usize| {
        let w = word(k);
        globals.iter().any(|g| g == w) || scoped.iter().any(|(b, e, n)| k > *b && k < *e && n == w)
    };
    // Is token k a whole argument of a call to a function taking samplers?
    let goes_split = |k: usize| {
        let p = prev_solid(k);
        let n = next_solid(k + 1);
        if p < 0 || n >= toks.len() {
            return false;
        }
        if !(word(p as usize) == "(" || word(p as usize) == ",") || !(word(n) == "," || word(n) == ")") {
            return false;
        }
        let mut depth = 0;
        let mut j = p;
        while j >= 0 {
            let w = word(j as usize);
            if w == ")" {
                depth += 1;
            } else if w == "(" {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            j -= 1;
        }
        let f = prev_solid(j.max(0) as usize);
        f >= 0 && toks[f as usize].0 && takes.contains_key(word(f as usize))
    };
    let mut out = String::with_capacity(src.len() + 512);
    let mut k = 0;
    while k < toks.len() {
        let (ident, s, e) = toks[k];
        if ident && param_at.contains(&k) {
            let name_k = next_solid(k + 1);
            let name = word(name_k);
            out.push_str(&format!("texture2D {name}_hz_tex, sampler {name}_hz_smp"));
            k = name_k + 1;
            continue;
        }
        if ident && is_sampler(k) {
            let name = word(k);
            if goes_split(k) {
                out.push_str(&format!("{name}_hz_tex, {name}_hz_smp"));
            } else {
                out.push_str(&format!("sampler2D({name}_hz_tex, {name}_hz_smp)"));
            }
        } else {
            out.push_str(&src[s..e]);
        }
        k += 1;
    }
    out
}

/// The original declares hooks (`groundGlow`, `cloudCut`) before the scenes
/// define them, and naga's GLSL frontend then puts a caller ahead of the
/// function it calls, which its own validator refuses: the functions are put
/// in an order where each comes after everything it calls.
fn callees_first(module: &mut naga::Module) {
    use naga::{Block, Expression, Function, Handle, Statement};
    fn calls_in(block: &Block, out: &mut Vec<usize>) {
        for statement in block.iter() {
            match statement {
                Statement::Call { function, .. } => out.push(function.index()),
                Statement::Block(b) => calls_in(b, out),
                Statement::If { accept, reject, .. } => {
                    calls_in(accept, out);
                    calls_in(reject, out);
                }
                Statement::Switch { cases, .. } => cases.iter().for_each(|c| calls_in(&c.body, out)),
                Statement::Loop { body, continuing, .. } => {
                    calls_in(body, out);
                    calls_in(continuing, out);
                }
                _ => {}
            }
        }
    }
    fn remap_block(block: &mut Block, map: &[Handle<Function>]) {
        for (statement, _) in block.span_iter_mut() {
            match statement {
                Statement::Call { function, .. } => *function = map[function.index()],
                Statement::Block(b) => remap_block(b, map),
                Statement::If { accept, reject, .. } => {
                    remap_block(accept, map);
                    remap_block(reject, map);
                }
                Statement::Switch { cases, .. } => cases.iter_mut().for_each(|c| remap_block(&mut c.body, map)),
                Statement::Loop { body, continuing, .. } => {
                    remap_block(body, map);
                    remap_block(continuing, map);
                }
                _ => {}
            }
        }
    }
    fn remap(f: &mut Function, map: &[Handle<Function>]) {
        for (_, e) in f.expressions.iter_mut() {
            if let Expression::CallResult(h) = e {
                *h = map[h.index()];
            }
        }
        remap_block(&mut f.body, map);
    }
    let n = module.functions.len();
    let mut calls = vec![Vec::new(); n];
    let mut spans = Vec::with_capacity(n);
    for (h, f) in module.functions.iter() {
        calls_in(&f.body, &mut calls[h.index()]);
        for (_, e) in f.expressions.iter() {
            if let Expression::CallResult(c) = e {
                calls[h.index()].push(c.index());
            }
        }
        spans.push(module.functions.get_span(h));
    }
    // Depth first, callees before callers, otherwise in the order written.
    let mut order = Vec::with_capacity(n);
    let mut state = vec![0u8; n];
    fn visit(i: usize, calls: &[Vec<usize>], state: &mut [u8], order: &mut Vec<usize>) {
        if state[i] != 0 {
            return;
        }
        state[i] = 1;
        for &c in &calls[i] {
            if c < state.len() {
                visit(c, calls, state, order);
            }
        }
        state[i] = 2;
        order.push(i);
    }
    for i in 0..n {
        visit(i, &calls, &mut state, &mut order);
    }
    if order.iter().enumerate().all(|(k, i)| k == *i) {
        return;
    }
    let old = std::mem::take(&mut module.functions).into_inner();
    let mut slots: Vec<Option<Function>> = old.into_iter().map(Some).collect();
    let mut arena = naga::Arena::new();
    let mut new_at = vec![None; n];
    for &i in &order {
        if let Some(f) = slots[i].take() {
            new_at[i] = Some(arena.append(f, spans[i]));
        }
    }
    let map: Vec<Handle<Function>> = new_at.into_iter().map(|h| h.expect("every function placed")).collect();
    module.functions = arena;
    for (_, f) in module.functions.iter_mut() {
        remap(f, &map);
    }
    for ep in module.entry_points.iter_mut() {
        remap(&mut ep.function, &map);
    }
}

/// The fragment shader `entry` compiled to a naga module, and its layout.
pub fn compile(sources: Sources, entry: &str, defines: &[(&str, &str)]) -> Result<(naga::Module, Layout), String> {
    let (text, layout) = preprocess(sources, entry, defines)?;
    let mut module = naga::front::glsl::Frontend::default()
        .parse(&naga::front::glsl::Options::from(naga::ShaderStage::Fragment), &text)
        .map_err(|e| format!("{entry}: {}", e.emit_to_string(&text).lines().take(6).collect::<Vec<_>>().join(" | ")))?;
    callees_first(&mut module);
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
        .validate(&module)
        .map_err(|e| format!("{entry}: {}", e.emit_to_string(&text).lines().take(6).collect::<Vec<_>>().join(" | ")))?;
    Ok((module, layout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(name: &str) -> Option<&'static str> {
        match name {
            "common.glsl" => Some("#version 150\nuniform sampler2D DiffuseSampler;\nuniform vec2 OutSize;\nuniform vec4 Pts[16];\nin vec2 texCoord;\nout vec4 fragColor;\n"),
            "a.fsh" => Some("#include \"b.glsl\"\nuniform float Shake, Bloom;\nuniform vec3 Target;\nvoid main() { fragColor = vec4(Shake); }\n"),
            "b.glsl" => Some("// a comment with uniform float Fake;\nuniform mat4 InvProj;\n"),
            _ => None,
        }
    }

    #[test]
    fn std140_offsets() {
        let (_, layout) = preprocess(sources, "a.fsh", &[]).unwrap();
        let at = |n: &str| layout.member(n).unwrap().offset;
        assert_eq!(at("OutSize"), 0);
        assert_eq!(at("Pts"), 16);
        assert_eq!(at("InvProj"), 16 + 256);
        assert_eq!(at("Shake"), 16 + 256 + 64);
        assert_eq!(at("Bloom"), 16 + 256 + 68);
        assert_eq!(at("Target"), 16 + 256 + 80);
        assert_eq!(layout.size, 16 + 256 + 96);
        assert!(layout.member("Fake").is_none());
        assert_eq!(layout.samplers, vec![("DiffuseSampler".to_owned(), 1)]);
    }

    #[test]
    fn the_text_keeps_its_main() {
        let (text, _) = preprocess(sources, "a.fsh", &[("LOOP_ZERO", "0")]).unwrap();
        assert!(text.starts_with("#version 450\n#define LOOP_ZERO 0\n"));
        assert!(text.contains("layout(location = 0) out vec4 fragColor;"));
        assert!(text.contains("uniform texture2D DiffuseSampler_hz_tex;"));
        assert!(!text.contains("uniform float Shake"));
    }

    #[test]
    fn sampler_parameters_are_split() {
        let src = "vec3 disc(float r, sampler2D tex, float p) { return texture(tex, vec2(r)).rgb; }\nvoid main() { vec3 a = disc(1.0, MilkyWay, 2.0); vec4 b = texture(MilkyWay, texCoord); }\n";
        let out = split_samplers(src, &["MilkyWay".to_owned()]);
        assert!(out.contains("float r, texture2D tex_hz_tex, sampler tex_hz_smp, float p"));
        assert!(out.contains("texture(sampler2D(tex_hz_tex, tex_hz_smp), vec2(r))"));
        assert!(out.contains("disc(1.0, MilkyWay_hz_tex, MilkyWay_hz_smp, 2.0)"));
        assert!(out.contains("texture(sampler2D(MilkyWay_hz_tex, MilkyWay_hz_smp), texCoord)"));
        let n = split_samplers("float x = 1.5e-3 + 2.0;", &[]);
        assert_eq!(n, "float x = 1.5e-3 + 2.0;");
    }

    #[test]
    fn a_block_fills_by_name() {
        let (_, layout) = preprocess(sources, "a.fsh", &[]).unwrap();
        let mut b = Block::new(&layout);
        b.set("Shake", &[2.0]);
        b.set_at("Pts", 1, &[1.0, 2.0, 3.0, 4.0]);
        b.set("Missing", &[9.0]);
        let o = layout.member("Shake").unwrap().offset as usize;
        assert_eq!(f32::from_le_bytes(b.bytes[o..o + 4].try_into().unwrap()), 2.0);
        let p = layout.member("Pts").unwrap().offset as usize + 16 + 8;
        assert_eq!(f32::from_le_bytes(b.bytes[p..p + 4].try_into().unwrap()), 3.0);
    }
}
