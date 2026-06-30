// UI overlay shader — SDF rounded rectangles with alpha + stroke.
//
// Draws anti-aliased rounded-corner rectangles for modern UI elements:
// tab bar backgrounds, pane borders, dialog panels, status bar.
//
// Vertex format (12 floats, 48 bytes stride):
//   location(0) position: vec2<f32>      — NDC clip-space position
//   location(1) color: vec4<f32>         — RGBA fill color [0,1]
//   location(2) local_pos: vec2<f32>     — pixel offset from rect center
//   location(3) half_size: vec2<f32>     — half width/height in pixels
//   location(4) params: vec2<f32>        — x=corner_radius_px, y=stroke_width_px

const PADDING: f32 = 1.0;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) local_pos: vec2<f32>,
    @location(3) half_size: vec2<f32>,
    @location(4) params: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local_pos: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) params: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    out.local_pos = in.local_pos;
    out.half_size = in.half_size;
    out.params = in.params;
    return out;
}

/// SDF for a rounded rectangle centered at origin.
/// Returns signed distance: negative inside, positive outside.
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let radius = in.params.x;
    let stroke_width = in.params.y;

    let dist = sd_rounded_box(in.local_pos, in.half_size, radius);

    // Anti-aliasing feather: 1.5px for crisp edges on all DPI levels.
    let feather = 1.5;

    if stroke_width > 0.5 {
        // Stroke mode: draw ring of width stroke_width centered on the boundary.
        let half_stroke = stroke_width * 0.5;
        // Distance from the stroke center line.
        let stroke_dist = abs(dist) - half_stroke + half_stroke; // = abs(dist)
        // Use abs(dist) - half_stroke as the actual boundary.
        let actual_dist = abs(dist) - half_stroke;
        let alpha = 1.0 - smoothstep(-feather, feather, actual_dist);
        if alpha < 0.001 {
            discard;
        }
        return vec4<f32>(in.color.rgb, in.color.a * alpha);
    } else {
        // Fill mode: fill the interior.
        let alpha = 1.0 - smoothstep(-feather, feather, dist);
        if alpha < 0.001 {
            discard;
        }
        return vec4<f32>(in.color.rgb, in.color.a * alpha);
    }
}
