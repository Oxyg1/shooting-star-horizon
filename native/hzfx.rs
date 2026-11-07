// The port's own fx programs: not the original's, written for this port in
// the original's dialect (`#version 150` GLSL that includes its
// `common.glsl`), so they run in the same chain with the same plumbing.
//
// - `hz_glow`: the spells' light on the world (blocks lit by the beam, the
//   lava, the falling stars), from a list of lights.
// - `hz_shock`: the air bent by shock fronts and heat over craters.
// - `hz_streak`: anamorphic streaks drawn out of the bloom.
// - `hz_horizon`: SS-H1 Event Horizon in the world: the hole, its bent
//   light, the accretion disk, the marks on the ground, the rim's glow.

pub const GLOW: &str = r#"
#include "common.glsl"

uniform vec4 Light[8];
uniform vec4 LightC[8];
uniform float Lights;

void main() {
vec2 uv = texCoord;
vec3 base = sampleScene(uv);
if (skyAt(uv) || Lights < 0.5) {
fragColor = vec4(base, 1.0);
return;
}
vec3 p = scenePosAt(uv);
vec2 px = 1.0 / OutSize;
vec3 ax = scenePosAt(uv + vec2(px.x, 0.0)) - p;
vec3 ay = scenePosAt(uv + vec2(0.0, px.y)) - p;
vec3 n = cross(ax, ay);
float nl = length(n);
n = nl > 1e-6 ? n / nl : -normalize(p);
if (dot(n, p) > 0.0) {
n = -n;
}
vec3 added = vec3(0.0);
for (int i = 0; i < 8; i++) {
if (float(i) >= Lights) {
break;
}
vec3 c = Light[i].xyz;
float r = max(Light[i].w, 0.5);
float tall = LightC[i].w;
// A column of light (the beam) lights from its nearest point.
vec3 q = c;
if (tall > 0.0) {
q.y = clamp(p.y, c.y, c.y + tall);
}
vec3 l = q - p;
float d = length(l);
float att = 1.0 / (1.0 + d * d / (r * r)) * smoothstep(r * 4.0, r * 0.6, d);
float lambert = max(dot(n, l / max(d, 1e-3)), 0.0) * 0.8 + 0.2;
added += LightC[i].rgb * att * lambert;
}
// Soft-limited, that a strong light never floods the picture; lit as the
// block's own colour, and a little of the light's on dark ground.
added = added * 2.0 / (2.0 + lum(added));
vec3 lit = base * added + added * 0.03;
fragColor = vec4(base + lit, 1.0);
}
"#;

pub const SHOCK: &str = r#"
#include "common.glsl"

uniform vec4 Shock[6];
uniform vec4 ShockK[6];
uniform float Shocks;
uniform vec4 Heat[4];
uniform vec4 HeatK[4];
uniform float Heats;

void main() {
vec2 uv = texCoord;
vec3 rd = viewDir(uv);
vec3 surf = scenePosAt(uv);
bool sky = skyAt(uv);
float aspect = OutSize.x / OutSize.y;
vec2 off = vec2(0.0);
float dust = 0.0;
for (int i = 0; i < 6; i++) {
if (float(i) >= Shocks) {
break;
}
vec3 c = Shock[i].xyz;
float r = Shock[i].w;
float strength = ShockK[i].x;
float thick = max(ShockK[i].y, 0.5);
if (r <= 0.0 || strength <= 0.0) {
continue;
}
// The front seen edge on: where the eye grazes the sphere.
float tc;
float b = rayPointDist(vec3(0.0), rd, c, tc);
float edge = exp(-sqr((r - b) / thick)) * step(b, r + thick * 3.0);
// The front along the ground.
float ground = sky ? 0.0 : exp(-sqr((length(surf - c) - r) / thick));
vec3 pc = project(c);
vec2 dir = pc.z > 0.0 ? uv - pc.xy : uv - 0.5;
dir.x *= aspect;
float dl = length(dir);
dir = dl > 1e-5 ? dir / dl : vec2(0.0);
dir.x /= aspect;
// Nearer fronts bend more; a far one is a ripple.
float near = 1.0 / (1.0 + length(c) / max(r, 1.0) * 0.35);
off += dir * (edge * 0.012 + ground * 0.02) * strength * near;
dust += ground * strength * 0.35;
}
for (int i = 0; i < 4; i++) {
if (float(i) >= Heats) {
break;
}
vec3 c = Heat[i].xyz;
float r = max(Heat[i].w, 1.0);
float strength = HeatK[i].x;
float tc;
float b = rayPointDist(vec3(0.0), rd, c, tc);
if (!sky && tc > length(surf) + r) {
continue;
}
// Hot air rises: it shimmers over the source, up to a few radii.
vec3 at = rd * tc - c;
float rise = smoothstep(-r * 0.3, r * 0.3, at.y) * (1.0 - smoothstep(r * 2.0, r * 4.0, at.y));
float k = strength * rise * (1.0 - smoothstep(r * 0.6, r * 1.4, length(at.xz)));
if (k > 0.001) {
vec2 w = vec2(vnoise2(uv * vec2(90.0, 45.0) + vec2(0.0, -GTime * 3.1)), vnoise2(uv * vec2(70.0, 38.0) + vec2(13.0, -GTime * 2.3))) - 0.5;
off += w * 0.0045 * k;
}
}
vec3 col = sampleScene(uv + off);
col = mix(col, vec3(lum(col)) * vec3(1.05, 0.95, 0.82), saturate(dust * 0.4));
fragColor = vec4(col, 1.0);
}
"#;

pub const STREAK: &str = r#"
#include "common.glsl"

uniform vec2 TexelSize;
uniform float Strength;

void main() {
vec2 uv = texCoord;
vec3 base = texture(DiffuseSampler, uv).rgb;
vec3 acc = vec3(0.0);
float total = 0.0;
for (int i = -16; i <= 16; i++) {
float k = float(i);
float w = exp(-abs(k) / 5.0);
vec3 s = texture(AuxSampler, clamp(uv + vec2(k * TexelSize.x * 3.0, 0.0), 0.001, 0.999)).rgb;
acc += max(s - 0.25, 0.0) * w;
total += w;
}
vec3 streak = acc / total * vec3(0.55, 0.72, 1.0) * 2.2;
fragColor = vec4(base + streak * Strength, 1.0);
}
"#;

pub const HORIZON: &str = r#"
#include "common.glsl"

uniform vec3 Center;

const vec3 AXIS = vec3(0.36, 0.92, 0.16);

vec3 diskColor(float k) {
vec3 hot = vec3(1.0, 0.93, 0.82) * 5.0;
vec3 orange = vec3(1.0, 0.52, 0.16) * 2.4;
vec3 red = vec3(0.55, 0.07, 0.03) * 0.8;
return mix(mix(hot, orange, smoothstep(0.0, 0.3, k)), red, smoothstep(0.3, 1.0, k));
}

void main() {
vec2 uv = texCoord;
vec3 base = sampleScene(uv);
float rs = P0.x;
float din = P0.y;
float dout = P0.z;
float sec = P0.w;
float diskK = P1.x;
float lensK = P1.y;
float flash = P1.z;
float seed = P1.w;
float ret = P2.x;
float eaten = P2.y;
float rim = P2.z;
float fall = P2.w;
float R = P3.x;
float collapse = P3.y;
float flashR = P3.z;
float jet = P3.w;
vec3 axis = normalize(AXIS);
vec3 rd = viewDir(uv);
vec3 surf = scenePosAt(uv);
bool sky = skyAt(uv);
float sceneT = sky ? FAR : length(surf);
vec3 col = base;

// The marks on the ground: the target's rings, the molten edge of what is
// eaten, the fused rim cooling.
if (!sky) {
vec3 p = surf - Center;
float d3 = length(p);
float fl = length(p.xz);
float ang = atan(p.z, p.x);
if (ret > 0.001) {
float near = 1.0 - smoothstep(R * 0.6, R * 1.2 + 6.0, abs(p.y + R * 0.25));
float ring = exp(-abs(fl - R) * 1.4);
float ticks = step(0.72, fract(ang * 16.0 / TAU + sec * 0.12)) * exp(-abs(fl - R * 0.84) * 3.0);
float inner = step(0.5, fract(ang * 32.0 / TAU - sec * 0.35)) * exp(-abs(fl - R * 0.5) * 4.0);
float cross1 = exp(-min(abs(p.x), abs(p.z)) * 3.0) * step(fl, R * 0.3);
col += vec3(1.0, 0.48, 0.12) * (ring * 2.4 + ticks * 1.4 + inner * 0.9 + cross1 * 0.7) * ret * near;
}
if (eaten > 0.5) {
float e = exp(-abs(d3 - eaten) * 1.6);
float n = vnoise3(p * 0.5 + vec3(0.0, sec * 0.8, 0.0));
col += vec3(1.0, 0.36, 0.08) * e * smoothstep(0.35, 0.8, n) * 1.6 * (1.0 - collapse);
}
if (rim > 0.001) {
float e = smoothstep(2.6, 0.0, abs(d3 - R - 0.7));
float n = vnoise3(p * 0.6);
float cracks = smoothstep(0.52, 0.72, n);
col += vec3(1.0, 0.3, 0.05) * e * (0.2 + cracks * 1.8) * rim * 2.2;
}
}

// The trail it draws as it falls.
if (fall > 0.001) {
float tr;
float b = raySegmentDist(vec3(0.0), rd, Center, Center + vec3(0.0, 60.0 + 120.0 * fall, 0.0), tr);
if (tr < sceneT) {
float w = max(rs, 0.6);
col += vec3(0.75, 0.55, 1.0) * exp(-b / (w * 1.8)) * fall * 1.6;
}
}

// The hole: rays near it are bent (a photon's path round a mass, stepped),
// cross the disk, fall in, or go on to whatever lies behind it.
if (rs > 0.01) {
float infl = rs * 16.0;
vec2 hit = raySphere(vec3(0.0), rd, Center, infl);
float tStart = max(hit.x, 0.0);
if (hit.y > 0.0 && tStart < sceneT) {
vec3 pos = rd * tStart - Center;
vec3 v = rd;
vec3 hc = cross(pos, v);
float h2 = dot(hc, hc);
vec3 acc = vec3(0.0);
float alpha = 0.0;
bool captured = false;
bool blocked = false;
float travelled = tStart;
vec3 e1 = normalize(cross(axis, vec3(0.0, 0.0, 1.0)));
vec3 e2 = cross(axis, e1);
for (int i = 0; i < 110; i++) {
float r = length(pos);
if (r < rs) {
captured = true;
break;
}
if (r > infl * 1.02 && dot(pos, v) > 0.0) {
break;
}
float dt = clamp(r * 0.08, rs * 0.04, rs * 1.5);
vec3 prev = pos;
v += -1.5 * rs * h2 * pos / pow(r, 5.0) * lensK * dt;
v = normalize(v);
pos += v * dt;
travelled += dt;
if (travelled > sceneT && i < 6) {
blocked = true;
break;
}
float s0 = dot(prev, axis);
float s1 = dot(pos, axis);
if (diskK > 0.001 && s0 * s1 < 0.0) {
vec3 q = mix(prev, pos, s0 / (s0 - s1));
float rq = length(q);
if (rq > din && rq < dout && travelled < sceneT) {
float k = (rq - din) / (dout - din);
float a = atan(dot(q, e2), dot(q, e1));
float omega = 2.6 / pow(rq / din, 1.5);
float turn = a + omega * sec;
float streak = vnoise2(vec2(log(rq) * 9.0, turn * 3.0 / TAU * 8.0 + seed));
streak = streak * 0.7 + 0.3 * vnoise2(vec2(log(rq) * 31.0, turn * 3.0 / TAU * 23.0));
float dens = smoothstep(0.0, 0.06, k) * pow(1.0 - k, 1.4) * (0.35 + 1.1 * streak);
vec3 flow = normalize(cross(axis, q));
float dop = clamp(1.0 + 0.55 * dot(flow, -v), 0.45, 1.7);
vec3 em = diskColor(k) * dens * dop * dop * dop * diskK;
float op = saturate(dens * 1.4 * diskK);
acc += (1.0 - alpha) * em;
alpha += (1.0 - alpha) * op;
}
}
}
vec3 behind;
if (captured) {
behind = vec3(0.0);
} else if (blocked) {
behind = base;
} else {
// What the bent ray finds: the picture where its new way points.
vec3 dir = normalize(v);
float on = sky ? 600.0 : max(sceneT - travelled, 4.0);
vec3 pr = project(Center + pos + dir * on);
behind = pr.z > 0.0 ? sampleScene(pr.xy) : base * 0.2;
}
// The bending fades out toward the edge of its reach, that it has no seam.
float tc;
float b = rayPointDist(vec3(0.0), rd, Center, tc);
float w = 1.0 - smoothstep(infl * 0.55, infl, b);
vec3 bent = behind * (1.0 - alpha) + acc;
col = mix(col, bent, captured ? 1.0 : max(w, saturate(alpha * 2.0)));
// The photon ring and a glow round the shadow.
float ring = exp(-abs(b - rs * 2.6) / (rs * 0.12));
float outside = smoothstep(rs * 2.45, rs * 2.75, b);
col += vec3(1.0, 0.82, 0.6) * ring * 0.9 * max(diskK, 0.3) * step(tc, sceneT + rs);
col += vec3(1.0, 0.5, 0.2) * exp(-(b - rs * 2.6) / (rs * 3.0)) * 0.22 * diskK * outside * step(tc, sceneT + rs);
}
}

// The flash: a sphere of light thrown out, and the poles' two jets.
if (flash > 0.001) {
float tc;
float b = rayPointDist(vec3(0.0), rd, Center, tc);
float shell = flashR > 0.0 ? exp(-abs(b - flashR) / (flashR * 0.06 + 0.5)) * step(b, flashR * 1.2) : 0.0;
float inside = 1.0 - smoothstep(flashR * 0.6, flashR, b);
float seen = tc - flashR < sceneT ? 1.0 : 0.3;
col += (vec3(1.0, 0.95, 0.88) * shell * 2.2 + vec3(1.0, 0.75, 0.5) * inside * 0.8) * flash * seen;
}
if (jet > 0.001) {
for (int s = 0; s < 2; s++) {
vec3 tip = Center + axis * (s == 0 ? 1.0 : -1.0) * (40.0 + 400.0 * jet);
float tr;
float b = raySegmentDist(vec3(0.0), rd, Center, tip, tr);
if (tr < sceneT) {
col += vec3(0.85, 0.9, 1.0) * exp(-b / (1.2 + R * 0.06)) * 3.0 * (1.0 - jet * 0.7);
}
}
}
fragColor = vec4(col, 1.0);
}
"#;
