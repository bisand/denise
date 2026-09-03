// Every shape Denise draws, as one pipeline.
//
// A vertex carries the pixel it sits on, the clip it must not leave, a
// premultiplied colour, two vec4s of shape parameters and a kind. The fragment
// shader turns the kind into a signed distance, the distance into coverage, and
// the coverage into how much of the colour reaches the target. Rectangles and
// polygons are kind 0 and skip all of that: their edges are the triangle's own.

struct Globals {
    // Target size in pixels, for the NDC transform.
    size: vec2<f32>,
    // 1 if the target is an sRGB format, so what we hand it must be linear.
    srgb: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) a: vec4<f32>,
    @location(4) b: vec4<f32>,
    @location(5) kind: u32,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) clip: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) a: vec4<f32>,
    @location(3) b: vec4<f32>,
    @location(4) @interpolate(flat) kind: u32,
};

@vertex
fn vs(in: VsIn) -> VsOut {
    var out: VsOut;
    let ndc = vec2<f32>(
        in.pos.x / globals.size.x * 2.0 - 1.0,
        1.0 - in.pos.y / globals.size.y * 2.0,
    );
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.clip = in.clip;
    out.color = in.color;
    out.a = in.a;
    out.b = in.b;
    out.kind = in.kind;
    return out;
}

// Anti-aliasing is one pixel wide: a fragment centre on the edge is half
// covered, one inside is fully covered, one outside not at all.
fn coverage(d: f32) -> f32 {
    return clamp(0.5 - d, 0.0, 1.0);
}

// Signed distance to a box of half-extents `half` with corner radius `r`,
// centred on the origin.
fn sd_round_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

// The angle of `v` in turns: 0 at twelve o'clock, clockwise positive, y down.
// The same convention as `denise::angle::TURN`, scaled to 0..1.
fn turns(v: vec2<f32>) -> f32 {
    let t = atan2(v.x, -v.y) / 6.283185307;
    return fract(t + 1.0);
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let p = in.position.xy;
    if p.x < in.clip.x || p.y < in.clip.y || p.x >= in.clip.z || p.y >= in.clip.w {
        discard;
    }

    // Sampled unconditionally: `textureSample` needs uniform control flow, and
    // a switch on a per-primitive kind does not count as uniform to naga.
    let texel = textureSample(tex, samp, in.a.xy);

    var color = in.color;
    var cov = 1.0;
    let a = in.a;
    let b = in.b;

    switch in.kind {
        // 0: solid triangle — rectangles and polygons.
        case 0u: {}
        // 1: rounded rectangle, filled.
        case 1u: {
            cov = coverage(sd_round_box(p - a.xy, a.zw, b.x));
        }
        // 2: rounded rectangle, stroked inside its bounds by b.y.
        case 2u: {
            let outer = sd_round_box(p - a.xy, a.zw, b.x);
            let inner = sd_round_box(p - a.xy, a.zw - vec2<f32>(b.y, b.y), max(b.x - b.y, 0.0));
            cov = coverage(outer) - coverage(inner);
        }
        // 3: circle, filled.
        case 3u: {
            cov = coverage(length(p - a.xy) - a.z);
        }
        // 4: circle, stroked inside its radius by a.w.
        case 4u: {
            let d = length(p - a.xy);
            cov = coverage(d - a.z) - coverage(d - (a.z - a.w));
        }
        // 5: arc — the ring of kind 4 cut to b.x..b.x+b.y turns.
        case 5u: {
            let d = length(p - a.xy);
            cov = coverage(d - a.z) - coverage(d - (a.z - a.w));
            let t = fract(turns(p - a.xy) - b.x + 1.0);
            if t >= b.y {
                cov = 0.0;
            }
        }
        // 6: a line from a.xy to a.zw, half-width b.x.
        case 6u: {
            cov = coverage(sd_segment(p, a.xy, a.zw) - b.x);
        }
        // 7: a premultiplied texture, sampled at a.xy.
        case 7u: {
            color = texel;
        }
        // 8: a coverage mask in the texture's red channel, in the vertex colour.
        case 8u: {
            cov = texel.r;
        }
        // 9: a texture masked to a rounded box: box centre b.xy, half b.zw, radius a.z.
        case 9u: {
            color = texel;
            cov = coverage(sd_round_box(p - b.xy, b.zw, a.z));
        }
        default: {}
    }

    var out = color * cov;
    if globals.srgb == 1u {
        out = vec4<f32>(pow(out.rgb, vec3<f32>(2.2, 2.2, 2.2)), out.a);
    }
    return out;
}
