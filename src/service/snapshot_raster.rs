//! Turns what egui tessellated into pixels on the CPU, so a chart can be drawn at any scale with
//! no window, no GPU and no frame to wait for.
//!
//! It follows the shader egui's wgpu renderer runs, on the framebuffer eframe picks (not sRGB):
//! a vertex's colour times the texel it points at, blended over what is there as premultiplied
//! alpha in gamma space, each draw rounded to 8 bits. Triangles cover the pixels whose centres
//! they contain (with the top-left rule, so two triangles sharing an edge never blend twice) and
//! are cut to their clip rectangle. A texture is sampled bilinearly or at the nearest texel, as
//! its options ask. Paint callbacks (custom GPU drawing) cannot run here and are skipped; the
//! chart uses none.

use crate::model::chart::snapshot::MAX_PIXELS;
use eframe::egui::epaint::{ClippedPrimitive, ImageData, ImageDelta, Mesh, Primitive, Vertex};
use eframe::egui::{Color32, ColorImage, Pos2, TextureFilter, TextureId, TexturesDelta};
use std::collections::HashMap;

/// A tessellated frame with what it needs to be drawn.
pub struct Scene {
    primitives: Vec<ClippedPrimitive>,
    textures: HashMap<TextureId, Texture>,
    pixels_per_point: f32,
    width: u32,
    height: u32,
    background: Color32,
}

impl Scene {
    /// `primitives` as tessellated at `pixels_per_point`, with the textures the frame that made
    /// them reported (taken from `textures_delta`: they are applied here, once), to be drawn on a
    /// `width` × `height` image that starts out as `background`.
    pub fn new(
        primitives: Vec<ClippedPrimitive>,
        mut textures_delta: TexturesDelta,
        pixels_per_point: f32,
        (width, height): (u32, u32),
        background: Color32,
    ) -> Self {
        let textures = load_textures(std::mem::take(&mut textures_delta.set));
        textures_delta.free.clear();
        Self {
            primitives,
            textures,
            pixels_per_point,
            width,
            height,
            background,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterError {
    /// The image has no pixels.
    Empty,
    /// The image holds more pixels than a snapshot may.
    TooLarge,
}

struct Texture {
    size: [usize; 2],
    pixels: Vec<Color32>,
    magnification: TextureFilter,
    minification: TextureFilter,
}

fn load_textures<D: AsRef<[ImageDelta]>>(
    set: impl IntoIterator<Item = (TextureId, D)>,
) -> HashMap<TextureId, Texture> {
    let mut textures: HashMap<TextureId, Texture> = HashMap::new();
    for (id, deltas) in set {
        for change in deltas.as_ref() {
            let ImageData::Color(image) = &change.image;
            match change.pos {
                None => {
                    textures.insert(
                        id,
                        Texture {
                            size: image.size,
                            pixels: image.pixels.clone(),
                            magnification: change.options.magnification,
                            minification: change.options.minification,
                        },
                    );
                }
                Some([x, y]) => {
                    let Some(texture) = textures.get_mut(&id) else {
                        continue;
                    };
                    let [width, height] = image.size;
                    if x + width > texture.size[0] || y + height > texture.size[1] {
                        continue;
                    }
                    for row in 0..height {
                        let from = row * width;
                        let to = (y + row) * texture.size[0] + x;
                        texture.pixels[to..to + width]
                            .copy_from_slice(&image.pixels[from..from + width]);
                    }
                }
            }
        }
    }
    textures
}

impl Texture {
    /// The texel at (`x`, `y`) as premultiplied channels, the edge repeating outwards.
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, self.size[0] as i64 - 1) as usize;
        let y = y.clamp(0, self.size[1] as i64 - 1) as usize;
        let [r, g, b, a] = self.pixels[y * self.size[0] + x].to_array();
        [f32::from(r), f32::from(g), f32::from(b), f32::from(a)]
    }

    fn sample(&self, u: f32, v: f32, filter: TextureFilter) -> [f32; 4] {
        let (x, y) = (u * self.size[0] as f32, v * self.size[1] as f32);
        match filter {
            TextureFilter::Nearest => self.texel(x.floor() as i64, y.floor() as i64),
            TextureFilter::Linear => {
                let (x, y) = (x - 0.5, y - 0.5);
                let (x0, y0) = (x.floor(), y.floor());
                let (fx, fy) = (x - x0, y - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let (tl, tr) = (self.texel(x0, y0), self.texel(x0 + 1, y0));
                let (bl, br) = (self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1));
                let mut out = [0.0; 4];
                for c in 0..4 {
                    let top = tl[c] + (tr[c] - tl[c]) * fx;
                    let bottom = bl[c] + (br[c] - bl[c]) * fx;
                    out[c] = top + (bottom - top) * fy;
                }
                out
            }
        }
    }
}

/// The pixels of a clip rectangle: `min` inclusive, `max` exclusive.
#[derive(Clone, Copy)]
struct PixelRect {
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
}

struct Target {
    width: usize,
    pixels: Vec<[u8; 4]>,
}

/// Twice the signed area of the triangle `p`, `q`, `r`: positive when `r` lies to the right of
/// `p` → `q` on a screen whose y runs down.
fn edge(p: Pos2, q: Pos2, r: Pos2) -> f32 {
    (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
}

/// Whether a pixel centre exactly on the edge `p` → `q` belongs to the triangle. Of the two
/// triangles that share an edge, which walk it in opposite directions, exactly one takes it.
fn owns_edge(p: Pos2, q: Pos2) -> bool {
    let (dx, dy) = (q.x - p.x, q.y - p.y);
    dy < 0.0 || (dy == 0.0 && dx > 0.0)
}

fn point(vertex: &Vertex, scale: f32) -> Pos2 {
    Pos2::new(vertex.pos.x * scale, vertex.pos.y * scale)
}

fn draw_triangle(
    target: &mut Target,
    texture: &Texture,
    clip: PixelRect,
    corners: [&Vertex; 3],
    scale: f32,
) {
    let [a, mut b, mut c] = corners;
    let (pa, mut pb, mut pc) = (point(a, scale), point(b, scale), point(c, scale));
    let mut area = edge(pa, pb, pc);
    if !area.is_finite() || area == 0.0 {
        return;
    }
    if area < 0.0 {
        std::mem::swap(&mut b, &mut c);
        std::mem::swap(&mut pb, &mut pc);
        area = -area;
    }
    let min_x = (pa.x.min(pb.x).min(pc.x).floor() as i64).max(clip.min_x);
    let max_x = (pa.x.max(pb.x).max(pc.x).ceil() as i64).min(clip.max_x);
    let min_y = (pa.y.min(pb.y).min(pc.y).floor() as i64).max(clip.min_y);
    let max_y = (pa.y.max(pb.y).max(pc.y).ceil() as i64).min(clip.max_y);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    // Texels per pixel along the triangle decides between magnifying and minifying.
    let uv_area = {
        let size = [texture.size[0] as f32, texture.size[1] as f32];
        let ua = Pos2::new(a.uv.x * size[0], a.uv.y * size[1]);
        let ub = Pos2::new(b.uv.x * size[0], b.uv.y * size[1]);
        let uc = Pos2::new(c.uv.x * size[0], c.uv.y * size[1]);
        edge(ua, ub, uc).abs()
    };
    let filter = if uv_area > area {
        texture.minification
    } else {
        texture.magnification
    };
    let owns = [owns_edge(pb, pc), owns_edge(pc, pa), owns_edge(pa, pb)];
    let colors = [a.color, b.color, c.color].map(|color| {
        let [r, g, b, a] = color.to_array();
        [f32::from(r), f32::from(g), f32::from(b), f32::from(a)]
    });
    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
            let weights = [edge(pb, pc, p), edge(pc, pa, p), edge(pa, pb, p)];
            let inside = weights
                .iter()
                .zip(owns)
                .all(|(weight, owned)| *weight > 0.0 || (*weight == 0.0 && owned));
            if !inside {
                continue;
            }
            let l = weights.map(|weight| weight / area);
            let u = l[0] * a.uv.x + l[1] * b.uv.x + l[2] * c.uv.x;
            let v = l[0] * a.uv.y + l[1] * b.uv.y + l[2] * c.uv.y;
            let texel = texture.sample(u, v, filter);
            let mut source = [0.0f32; 4];
            for channel in 0..4 {
                let vertex = l[0] * colors[0][channel]
                    + l[1] * colors[1][channel]
                    + l[2] * colors[2][channel];
                // Both are 0–255 premultiplied values: their product, back on that scale.
                source[channel] = vertex * texel[channel] / 255.0;
            }
            let pixel = &mut target.pixels[y as usize * target.width + x as usize];
            let keep = 1.0 - source[3].clamp(0.0, 255.0) / 255.0;
            for channel in 0..4 {
                let blended = source[channel] + f32::from(pixel[channel]) * keep;
                pixel[channel] = blended.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

fn draw_mesh(
    target: &mut Target,
    textures: &HashMap<TextureId, Texture>,
    mesh: &Mesh,
    clip: PixelRect,
    scale: f32,
) {
    let Some(texture) = textures.get(&mesh.texture_id) else {
        return;
    };
    if texture.size[0] == 0 || texture.size[1] == 0 {
        return;
    }
    let (triangles, _) = mesh.indices.as_chunks::<3>();
    for [a, b, c] in triangles {
        let vertex = |index: &u32| mesh.vertices.get(*index as usize);
        if let (Some(a), Some(b), Some(c)) = (vertex(a), vertex(b), vertex(c)) {
            draw_triangle(target, texture, clip, [a, b, c], scale);
        }
    }
}

/// Draws `scene` and returns the image.
pub fn rasterize(scene: &Scene) -> Result<ColorImage, RasterError> {
    let (width, height) = (scene.width as usize, scene.height as usize);
    if width == 0 || height == 0 {
        return Err(RasterError::Empty);
    }
    if width as u64 * height as u64 > MAX_PIXELS {
        return Err(RasterError::TooLarge);
    }
    let textures = &scene.textures;
    let [r, g, b, a] = scene.background.to_array();
    let mut target = Target {
        width,
        pixels: vec![[r, g, b, a]; width * height],
    };
    let scale = scene.pixels_per_point;
    for clipped in &scene.primitives {
        let Primitive::Mesh(mesh) = &clipped.primitive else {
            continue;
        };
        let rect = clipped.clip_rect;
        // The clip rectangle snaps to whole pixels, as the GPU's scissor does.
        let clip = PixelRect {
            min_x: ((rect.min.x * scale).round() as i64).clamp(0, width as i64),
            min_y: ((rect.min.y * scale).round() as i64).clamp(0, height as i64),
            max_x: ((rect.max.x * scale).round() as i64).clamp(0, width as i64),
            max_y: ((rect.max.y * scale).round() as i64).clamp(0, height as i64),
        };
        draw_mesh(&mut target, textures, mesh, clip, scale);
    }
    let pixels = target
        .pixels
        .into_iter()
        .map(|[r, g, b, a]| Color32::from_rgba_premultiplied(r, g, b, a))
        .collect();
    Ok(ColorImage::new([width, height], pixels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::epaint::ImageDelta;
    use eframe::egui::{self, Rect, TextureOptions, Vec2, vec2};

    const WHITE: TextureId = TextureId::Managed(0);

    /// A texture whose first texel is white, as egui's own first texture always is.
    fn white_texture() -> TexturesDelta {
        let mut deltas = TexturesDelta::default();
        deltas.set.insert(
            WHITE,
            vec![ImageDelta::full(
                ColorImage::filled([2, 2], Color32::WHITE),
                TextureOptions::NEAREST,
            )]
            .into(),
        );
        deltas
    }

    fn vertex(x: f32, y: f32, color: Color32) -> Vertex {
        Vertex {
            pos: Pos2::new(x, y),
            uv: Pos2::new(0.25, 0.25),
            color,
        }
    }

    fn rect_mesh(rect: Rect, color: Color32) -> Mesh {
        let mut mesh = Mesh::with_texture(WHITE);
        mesh.vertices = vec![
            vertex(rect.min.x, rect.min.y, color),
            vertex(rect.max.x, rect.min.y, color),
            vertex(rect.max.x, rect.max.y, color),
            vertex(rect.min.x, rect.max.y, color),
        ];
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        mesh
    }

    fn scene(width: u32, height: u32, scale: f32, meshes: Vec<(Rect, Mesh)>) -> Scene {
        scene_with(white_texture(), width, height, scale, meshes)
    }

    fn scene_with(
        textures_delta: TexturesDelta,
        width: u32,
        height: u32,
        scale: f32,
        meshes: Vec<(Rect, Mesh)>,
    ) -> Scene {
        Scene::new(
            meshes
                .into_iter()
                .map(|(clip_rect, mesh)| ClippedPrimitive {
                    clip_rect,
                    primitive: Primitive::Mesh(mesh),
                })
                .collect(),
            textures_delta,
            scale,
            (width, height),
            Color32::BLACK,
        )
    }

    fn everywhere(width: f32, height: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(width, height))
    }

    fn at(image: &ColorImage, x: usize, y: usize) -> Color32 {
        image.pixels[y * image.size[0] + x]
    }

    #[test]
    fn a_rectangle_covers_the_pixels_whose_centres_it_holds() {
        let rect = Rect::from_min_max(Pos2::new(2.0, 1.0), Pos2::new(6.0, 4.0));
        let red = Color32::from_rgb(200, 10, 20);
        let image = rasterize(&scene(
            8,
            6,
            1.0,
            vec![(everywhere(8.0, 6.0), rect_mesh(rect, red))],
        ))
        .expect("drawn");
        for y in 0..6 {
            for x in 0..8 {
                let inside = (2..6).contains(&x) && (1..4).contains(&y);
                let expected = if inside { red } else { Color32::BLACK };
                assert_eq!(at(&image, x, y), expected, "({x}, {y})");
            }
        }
    }

    #[test]
    fn the_scale_draws_the_same_shapes_with_more_pixels() {
        let rect = Rect::from_min_max(Pos2::new(1.0, 1.0), Pos2::new(3.0, 2.0));
        let image = rasterize(&scene(
            8,
            6,
            2.0,
            vec![(everywhere(4.0, 3.0), rect_mesh(rect, Color32::WHITE))],
        ))
        .expect("drawn");
        let lit: Vec<(usize, usize)> = (0..6)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|(x, y)| at(&image, *x, *y) == Color32::WHITE)
            .collect();
        assert_eq!(lit.len(), 4 * 2, "a 2 × 1 point rectangle is 4 × 2 pixels");
        assert!(
            lit.iter()
                .all(|(x, y)| (2..6).contains(x) && (2..4).contains(y))
        );
    }

    #[test]
    fn a_shared_edge_is_drawn_once() {
        // Half-transparent white over black: a pixel drawn twice would come out brighter.
        let half = Color32::from_white_alpha(100);
        let rect = Rect::from_min_max(Pos2::new(0.5, 0.5), Pos2::new(7.5, 5.5));
        let image = rasterize(&scene(
            8,
            6,
            1.0,
            vec![(everywhere(8.0, 6.0), rect_mesh(rect, half))],
        ))
        .expect("drawn");
        let drawn: Vec<Color32> = (1..7).map(|x| at(&image, x, 3)).collect();
        assert!(drawn.iter().all(|pixel| *pixel == drawn[0]), "{drawn:?}");
        assert_eq!(drawn[0], Color32::from_gray(100));
        // The diagonal goes through pixel centres; none of them is blended twice.
        let diagonal = Rect::from_min_max(Pos2::ZERO, Pos2::new(6.0, 6.0));
        let square = rasterize(&scene(
            6,
            6,
            1.0,
            vec![(everywhere(6.0, 6.0), rect_mesh(diagonal, half))],
        ))
        .expect("drawn");
        for y in 0..6 {
            for x in 0..6 {
                assert_eq!(at(&square, x, y), Color32::from_gray(100), "({x}, {y})");
            }
        }
    }

    #[test]
    fn translucent_colour_blends_over_what_is_there_premultiplied() {
        let rect = everywhere(4.0, 4.0);
        let blue = Color32::from_rgb(0, 0, 255);
        let red_half = Color32::from_rgba_unmultiplied(255, 0, 0, 128);
        let scene = scene(
            4,
            4,
            1.0,
            vec![
                (rect, rect_mesh(rect, blue)),
                (rect, rect_mesh(rect, red_half)),
            ],
        );
        let image = rasterize(&scene).expect("drawn");
        let [r, g, b, a] = at(&image, 1, 1).to_array();
        // premultiplied red 128 + blue 255 × (1 − 128/255) = 127.
        assert_eq!((r, g, b, a), (128, 0, 127, 255));
    }

    #[test]
    fn a_clip_rectangle_cuts_what_is_drawn() {
        let image = rasterize(&scene(
            6,
            4,
            1.0,
            vec![(
                Rect::from_min_max(Pos2::new(1.0, 1.0), Pos2::new(3.0, 3.0)),
                rect_mesh(everywhere(6.0, 4.0), Color32::WHITE),
            )],
        ))
        .expect("drawn");
        for y in 0..4 {
            for x in 0..6 {
                let inside = (1..3).contains(&x) && (1..3).contains(&y);
                assert_eq!(at(&image, x, y) == Color32::WHITE, inside, "({x}, {y})");
            }
        }
    }

    #[test]
    fn shapes_off_the_image_are_cut_not_wrapped() {
        let rect = Rect::from_min_max(Pos2::new(-5.0, -5.0), Pos2::new(50.0, 2.0));
        let image = rasterize(&scene(
            4,
            4,
            1.0,
            vec![(everywhere(100.0, 100.0), rect_mesh(rect, Color32::WHITE))],
        ))
        .expect("drawn");
        assert_eq!(at(&image, 3, 1), Color32::WHITE);
        assert_eq!(at(&image, 0, 2), Color32::BLACK);
    }

    #[test]
    fn a_texture_patch_replaces_part_of_it() {
        // The 2×2 white texture, then its right column turned red.
        let mut textures = white_texture();
        textures
            .set
            .get_mut(&WHITE)
            .unwrap()
            .push(ImageDelta::partial(
                [1, 0],
                ColorImage::filled([1, 2], Color32::RED),
                TextureOptions::NEAREST,
            ));
        let mut meshes = Vec::new();
        for (x0, u) in [(0.0, 0.25), (1.0, 0.75)] {
            let mut mesh = Mesh::with_texture(WHITE);
            mesh.vertices = [(x0, 0.0), (x0 + 1.0, 0.0), (x0 + 1.0, 1.0), (x0, 1.0)]
                .map(|(x, y)| Vertex {
                    pos: Pos2::new(x, y),
                    uv: Pos2::new(u, 0.25),
                    color: Color32::WHITE,
                })
                .to_vec();
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            meshes.push((everywhere(2.0, 1.0), mesh));
        }
        let image = rasterize(&scene_with(textures, 2, 1, 1.0, meshes)).expect("drawn");
        assert_eq!(at(&image, 0, 0), Color32::WHITE);
        assert_eq!(at(&image, 1, 0), Color32::RED);
    }

    #[test]
    fn a_linear_texture_blends_its_neighbours_and_a_nearest_one_does_not() {
        // Two texels, white then black; the pixel looks at 45% across: inside the white texel,
        // but 0.4 of the way from its centre to the black one's.
        for (options, expected) in [
            (TextureOptions::NEAREST, 255u8),
            (TextureOptions::LINEAR, 153),
        ] {
            let mut texture = ColorImage::filled([2, 1], Color32::BLACK);
            texture.pixels[0] = Color32::WHITE;
            let mut textures = TexturesDelta::default();
            textures
                .set
                .insert(WHITE, vec![ImageDelta::full(texture, options)].into());
            let mut mesh = Mesh::with_texture(WHITE);
            mesh.vertices = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
                .map(|(x, y)| Vertex {
                    pos: Pos2::new(x, y),
                    uv: Pos2::new(0.45, 0.5),
                    color: Color32::WHITE,
                })
                .to_vec();
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            let image = rasterize(&scene_with(
                textures,
                1,
                1,
                1.0,
                vec![(everywhere(1.0, 1.0), mesh)],
            ))
            .expect("drawn");
            let got = i16::from(at(&image, 0, 0).r());
            assert!((got - i16::from(expected)).abs() <= 1, "{options:?}: {got}");
        }
    }

    #[test]
    fn an_empty_or_oversized_image_is_refused() {
        assert_eq!(
            rasterize(&scene(0, 10, 1.0, Vec::new())).unwrap_err(),
            RasterError::Empty
        );
        assert_eq!(
            rasterize(&scene(8192, 8192, 1.0, Vec::new())).unwrap_err(),
            RasterError::TooLarge
        );
    }

    /// What egui itself makes of some widgets, drawn at 1× and 2×.
    fn drawn_ui(scale: f32, size: Vec2) -> Scene {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(20, 30, 50));
            ui.painter().circle_filled(
                Pos2::new(40.0, 30.0),
                14.0,
                Color32::from_rgb(240, 200, 40),
            );
            ui.painter().text(
                Pos2::new(70.0, 20.0),
                egui::Align2::LEFT_TOP,
                "Hello 123",
                egui::FontId::proportional(16.0),
                Color32::WHITE,
            );
        });
        Scene::new(
            ctx.tessellate(output.shapes, output.pixels_per_point),
            output.textures_delta,
            output.pixels_per_point,
            ((size.x * scale) as u32, (size.y * scale) as u32),
            Color32::BLACK,
        )
    }

    #[test]
    fn egui_shapes_and_text_come_out_with_their_colours() {
        let image = rasterize(&drawn_ui(1.0, vec2(200.0, 60.0))).expect("drawn");
        assert_eq!(at(&image, 5, 5), Color32::from_rgb(20, 30, 50));
        assert_eq!(at(&image, 40, 30), Color32::from_rgb(240, 200, 40));
        // The text puts light pixels over the dark panel, antialiased: some in between too.
        let text_box = (70..160).flat_map(|x| (18..40).map(move |y| (x, y)));
        let (mut bright, mut partial) = (0, 0);
        for (x, y) in text_box {
            let pixel = at(&image, x, y);
            if pixel.r() > 200 {
                bright += 1;
            } else if pixel.r() > 40 {
                partial += 1;
            }
        }
        assert!(bright > 40, "the glyphs: {bright}");
        assert!(partial > 20, "their antialiased edges: {partial}");
    }

    #[test]
    fn drawing_at_twice_the_scale_gives_a_sharper_picture_of_the_same_thing() {
        let one = rasterize(&drawn_ui(1.0, vec2(200.0, 60.0))).expect("1×");
        let two = rasterize(&drawn_ui(2.0, vec2(200.0, 60.0))).expect("2×");
        assert_eq!(two.size, [400, 120]);
        assert_eq!(
            at(&two, 80, 60),
            Color32::from_rgb(240, 200, 40),
            "the disc's centre"
        );
        // The same amount of bright glyph, spread over four times the pixels.
        let bright = |image: &ColorImage| {
            image
                .pixels
                .iter()
                .filter(|pixel| pixel.r() > 200 && pixel.b() > 200)
                .count()
        };
        let ratio = bright(&two) as f32 / bright(&one) as f32;
        assert!((3.0..5.2).contains(&ratio), "{ratio}");
    }

    #[test]
    fn the_same_scene_is_the_same_image() {
        let a = rasterize(&drawn_ui(2.0, vec2(120.0, 40.0))).expect("drawn");
        let b = rasterize(&drawn_ui(2.0, vec2(120.0, 40.0))).expect("drawn");
        assert_eq!(a.pixels, b.pixels);
    }
}
