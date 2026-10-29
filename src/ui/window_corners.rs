//! Rounded corners for an undecorated window (Linux), which the window system does not round.
//!
//! The window's surface is transparent (`window_chrome::rounded_corners`), but egui can only blend
//! what it draws over the surface, and the tab strip, the chart and the toolbars are all opaque
//! right into the four corners. So the last thing a window draws is a pass that *erases* the area
//! outside a quarter circle at each corner: a full-screen triangle whose fragment shader keeps the
//! pixels inside the corner arcs and multiplies the ones outside by zero (the blend is
//! `dst * (1 - src.a)`, colour and alpha alike), antialiased over one pixel.
//!
//! [`corner_coverage`] is the same arithmetic on the CPU, which the tests check.

use eframe::egui::{self, Id, LayerId, Order};
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu;

/// The corner radius, in points. macOS windows are rounded about this much.
pub const WINDOW_CORNER_RADIUS: f32 = 10.0;

/// How much of the pixel at (`x`, `y`) (its centre) of a `width` × `height` window survives the
/// rounding: 1 inside, 0 outside a corner's arc, in between on its edge. `radius` is in pixels.
/// This is the shader's arithmetic, for the tests to check; the GPU does the real work.
#[cfg(test)]
fn corner_coverage(x: f32, y: f32, width: f32, height: f32, radius: f32) -> f32 {
    let (qx, qy) = (x.min(width - x), y.min(height - y));
    if qx >= radius || qy >= radius {
        return 1.0;
    }
    let distance = (radius - qx).hypot(radius - qy) - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Whether a window rounds its corners now: it does where the system composites (`rounded`), and
/// never maximized or fullscreen, where square corners meet the screen's edges.
pub const fn corners_wanted(rounded: bool, maximized: bool, fullscreen: bool) -> bool {
    rounded && !maximized && !fullscreen
}

/// Queues the erase pass for the window being drawn, over everything else in it.
pub fn erase(ctx: &egui::Context, radius_points: f32) {
    let painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::new("window-corners")));
    painter.add(egui_wgpu::Callback::new_paint_callback(
        ctx.content_rect(),
        CornerErase { radius_points },
    ));
}

/// Builds the pass once, for `erase` to find. Call it when the app starts, if windows round.
pub fn install(render_state: &RenderState) {
    let pass = ErasePass::new(&render_state.device, render_state.target_format);
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(pass);
}

const SHADER: &str = "
struct Params {
    size: vec2<f32>,
    radius: f32,
    unused: f32,
};
@group(0) @binding(0) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the whole screen.
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let r = params.radius;
    // Distance to the nearest edge on each axis: only a pixel near two edges is in a corner.
    let q = min(pixel.xy, params.size - pixel.xy);
    if (q.x >= r || q.y >= r) {
        discard;
    }
    let distance = length(vec2<f32>(r, r) - q) - r;
    let coverage = clamp(0.5 - distance, 0.0, 1.0);
    return vec4<f32>(0.0, 0.0, 0.0, 1.0 - coverage);
}
";

/// The pipeline and its parameters, kept in egui's callback resources.
struct ErasePass {
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl ErasePass {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("window_corners"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("window_corners_params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("window_corners_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("window_corners_bind_group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("window_corners_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // `dst * (1 - src.a)` for colour and alpha: what the shader's alpha covers goes clear.
        let erase = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("window_corners"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState {
                        color: erase,
                        alpha: erase,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            params,
            bind_group,
        }
    }
}

/// The parameters as the shader reads them: the size and the radius in pixels, and padding.
fn params_bytes(width: f32, height: f32, radius: f32) -> [u8; 16] {
    let mut bytes = [0; 16];
    for (index, value) in [width, height, radius, 0.0].into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

struct CornerErase {
    radius_points: f32,
}

impl CallbackTrait for CornerErase {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(pass) = resources.get::<ErasePass>() {
            let [width, height] = screen.size_in_pixels;
            let radius = self.radius_points * screen.pixels_per_point;
            queue.write_buffer(
                &pass.params,
                0,
                &params_bytes(width as f32, height as f32, radius),
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(pass) = resources.get::<ErasePass>() else {
            return;
        };
        render_pass.set_pipeline(&pass.pipeline);
        render_pass.set_bind_group(0, &pass.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f32, f32) = (200.0, 100.0);

    fn coverage(x: f32, y: f32, radius: f32) -> f32 {
        corner_coverage(x, y, SIZE.0, SIZE.1, radius)
    }

    #[test]
    fn the_corner_pixels_go_and_the_rest_stays() {
        // Every corner's outermost pixel is gone, at any distance from the other corners.
        for (x, y) in [(0.5, 0.5), (199.5, 0.5), (0.5, 99.5), (199.5, 99.5)] {
            assert_eq!(coverage(x, y, 10.0), 0.0, "({x}, {y})");
        }
        // The edges' middles, the middle, and everything at or past a radius from a corner stay.
        for (x, y) in [
            (100.0, 0.5),
            (0.5, 50.0),
            (100.0, 50.0),
            (10.5, 0.5),
            (0.5, 10.5),
        ] {
            assert_eq!(coverage(x, y, 10.0), 1.0, "({x}, {y})");
        }
    }

    #[test]
    fn the_arc_is_half_covered_on_its_edge_and_symmetric() {
        // On the 45 degree line the arc is r * (1 - 1/sqrt(2)) from each edge.
        let on_arc = 10.0 * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
        assert!((coverage(on_arc, on_arc, 10.0) - 0.5).abs() < 1e-4);
        assert!(coverage(on_arc + 1.0, on_arc + 1.0, 10.0) > 0.99);
        assert!(coverage(on_arc - 1.0, on_arc - 1.0, 10.0) < 0.01);
        // The four corners are mirror images.
        let (w, h) = SIZE;
        let top_left = coverage(3.5, 2.5, 10.0);
        assert_eq!(corner_coverage(w - 3.5, 2.5, w, h, 10.0), top_left);
        assert_eq!(corner_coverage(3.5, h - 2.5, w, h, 10.0), top_left);
        assert_eq!(corner_coverage(w - 3.5, h - 2.5, w, h, 10.0), top_left);
    }

    #[test]
    fn no_radius_erases_nothing() {
        for (x, y) in [(0.5, 0.5), (199.5, 99.5), (100.0, 50.0)] {
            assert_eq!(coverage(x, y, 0.0), 1.0);
        }
    }

    #[test]
    fn corners_round_only_where_composited_and_not_filling_the_screen() {
        assert!(corners_wanted(true, false, false));
        assert!(!corners_wanted(false, false, false), "no compositor");
        assert!(!corners_wanted(true, true, false), "maximized");
        assert!(!corners_wanted(true, false, true), "fullscreen");
    }

    #[test]
    fn the_parameters_are_the_size_the_radius_and_padding() {
        let bytes = params_bytes(1.0, 2.0, 3.0);
        let floats: [f32; 4] = std::array::from_fn(|index| {
            f32::from_ne_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
        });
        assert_eq!(floats, [1.0, 2.0, 3.0, 0.0]);
    }

    /// The shader parses and validates, so a mistake shows here and not as a blank window.
    #[test]
    fn the_shader_is_valid() {
        use wgpu::naga;
        let module = naga::front::wgsl::parse_str(SHADER).expect("the shader parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("the shader validates");
    }
}
