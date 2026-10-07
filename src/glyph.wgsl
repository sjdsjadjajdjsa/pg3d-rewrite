// Bitmap-font glyphs, blended the way NGUI does in Unity's gamma colour space.
//
// The texture is filtered first (bilinear, like the original) and only then is the
// coverage turned into the alpha that, blended in linear light, gives the colour that
// mixing the sRGB values would give over a typical dark background (sRGB 0.2). Doing this
// per texel before filtering (as for the crisp atlas sprites) makes filtered glyph edges
// too bright: a faint extra line under letters such as "a" and "5".
#import bevy_ui::ui_vertex_output::UiVertexOutput



const GLYPH_BG: f32 = 0.02;
struct Glyph {
    // uv rectangle in the font page: min.xy, size.zw
    rect: vec4<f32>,
    // tint, sRGB
    color: vec4<f32>,
};

@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var<uniform> glyph: Glyph;

fn to_lin(v: f32) -> f32 { return pow(max(v, 0.0), 2.2); }
fn to_gamma(v: vec3<f32>) -> vec3<f32> { return pow(max(v, vec3(0.0)), vec3(1.0 / 2.2)); }

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let s = textureSample(tex, samp, glyph.rect.xy + in.uv * glyph.rect.zw);
    let c = to_gamma(s.rgb) * glyph.color.rgb;
    let a = s.a * glyph.color.a;
    let l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    let b = GLYPH_BG;
    let d = to_lin(l) - to_lin(b);
    var out_a = a;
    if abs(d) >= 0.02 {
        out_a = clamp((to_lin(a * l + (1.0 - a) * b) - to_lin(b)) / d, 0.0, 1.0);
    }
    return vec4(pow(c, vec3(2.2)), out_a);
}
