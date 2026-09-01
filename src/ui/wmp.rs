//! Drawing Windows Media Player skins: a definition's view, painted the
//! way its author laid it out.
//!
//! The window's shape is every background layer's non-keyed pixels taken
//! together — the view's own background, and the backgrounds of the
//! visible subviews, positioned where they sit. Nothing paints outside
//! that shape, row span by row span, the way the Winamp window does.
//! Elements paint in z order, document order breaking ties, with each
//! child positioned in its parent's coordinates. Art decodes once and
//! uploads once per window; the [`Render`] holds both between frames.
//!
//! This pass only draws. What the skin's controls *do* — hover states,
//! dragging, the player behind the bindings — arrives with the
//! interaction pass.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{
    Align2, Color32, ColorImage, Context, FontId, Pos2, Rect, TextureHandle, TextureId,
    TextureOptions, Ui, Vec2,
};

use crate::skin::{Bitmap, Mask};
use crate::wmp::ir::{self, Background, Element, Value, View};
use crate::wmp::{Assets, SkinDocument};

/// The art of one skin, decoded and on the graphics card, kept between
/// frames so neither work happens twice.
#[derive(Default)]
pub struct Render {
    /// Decoded bitmaps by lower-case file name. One that would not
    /// decode is kept empty, so it is not read again every frame.
    bitmaps: HashMap<String, Bitmap>,
    /// Keyed copies, by file and the colour cut out of it.
    keyed: HashMap<(String, Option<ir::Color>), Bitmap>,
    /// Uploaded textures, by file and the colour keyed out of it. These
    /// belong to a window's context and go with it.
    textures: HashMap<(String, Option<ir::Color>), TextureHandle>,
}

/// A bitmap that would not decode, kept so the attempt is not repeated.
const EMPTY: Bitmap = Bitmap {
    width: 0,
    height: 0,
    rgba: Vec::new(),
};

impl Render {
    /// The window is gone (or about to be), and with it the textures.
    pub fn forget_textures(&mut self) {
        self.textures.clear();
    }

    /// A file decoded, once. The name is matched the way skins name
    /// their art: without regard to case.
    fn bitmap(&mut self, assets: &Assets, file: &str) -> Bitmap {
        let name = file.to_ascii_lowercase();
        if !self.bitmaps.contains_key(&name) {
            let decoded = assets.bitmap(&name).unwrap_or(EMPTY);
            self.bitmaps.insert(name.clone(), decoded);
        }
        self.bitmaps[&name].clone()
    }

    /// A file's pixels with its key colour cut out, once per file and
    /// colour.
    fn keyed_bitmap(
        &mut self,
        assets: &Assets,
        file: &str,
        key: Option<ir::Color>,
    ) -> Option<Bitmap> {
        let name = file.to_ascii_lowercase();
        let entry = (name.clone(), key);
        if !self.keyed.contains_key(&entry) {
            let raw = self.bitmap(assets, file);
            let keyed = match key {
                Some(key) => raw.keyed(key),
                None => raw,
            };
            self.keyed.insert(entry.clone(), keyed);
        }
        let bitmap = &self.keyed[&entry];
        (bitmap.width > 0).then(|| bitmap.clone())
    }

    /// A keyed bitmap as a texture, uploaded once per window.
    fn texture(
        &mut self,
        ctx: &Context,
        assets: &Assets,
        file: &str,
        key: Option<ir::Color>,
    ) -> Option<TextureId> {
        let name = file.to_ascii_lowercase();
        let entry = (name.clone(), key);
        if let Some(handle) = self.textures.get(&entry) {
            return Some(handle.id());
        }
        let bitmap = self.keyed_bitmap(assets, file, key)?;
        let image = ColorImage::from_rgba_unmultiplied(
            [bitmap.width as usize, bitmap.height as usize],
            &bitmap.rgba,
        );
        let handle = ctx.load_texture(format!("wmp-{name}"), image, TextureOptions::NEAREST);
        let id = handle.id();
        self.textures.insert(entry, handle);
        Some(id)
    }
}

/// Draws the skin's main view, top-left at `origin`, with `unit` screen
/// pixels to the skin pixel.
pub fn show(
    ui: &mut Ui,
    document: &Arc<SkinDocument>,
    render: &mut Render,
    origin: Pos2,
    unit: f32,
) {
    let Some(view) = document.main_view() else {
        return;
    };
    let ctx = ui.ctx().clone();
    let size = view_size(render, document, view);
    if size.0 == 0 || size.1 == 0 {
        return;
    }
    let mask = window_mask(render, document, view, size);
    let skin = Skin {
        ui,
        origin,
        unit,
        mask: mask.as_ref(),
    };
    if let Some(color) = view.background.color {
        skin.fill(0, 0, size.0, size.1, color);
    }
    let mut ordered: Vec<(i32, &Element)> = view
        .children
        .iter()
        .map(|element| (element.common().z_index.unwrap_or(0), element))
        .collect();
    // A stable sort keeps document order inside one layer.
    ordered.sort_by_key(|(z, _)| *z);
    let mut art = Art {
        document,
        render,
        ctx: &ctx,
    };
    for (_, element) in ordered {
        paint_element(&skin, &mut art, element, (0, 0));
    }
}

/// The demo's read-only look at a skin: it floats over the big window,
/// small margin, at one-to-one pixels. The screenshot surface answers
/// "does Toothy show its tooth" without any of the app behind it.
#[cfg(any(test, feature = "demo"))]
pub fn preview(app: &mut crate::app::App, ui: &mut Ui) {
    let Some((document, render)) = app.wmp_preview.as_mut() else {
        return;
    };
    let origin = ui.max_rect().min + Vec2::splat(16.0);
    show(ui, document, render, origin, 1.0);
}

/// The view's size in skin pixels: what its attributes say, or failing
/// that, what its background layers cover.
fn view_size(render: &mut Render, document: &SkinDocument, view: &View) -> (u32, u32) {
    if let (Some(width), Some(height)) = (view.width, view.height) {
        return (width.max(0) as u32, height.max(0) as u32);
    }
    let mut widest = 0u32;
    let mut tallest = 0u32;
    for (x, y, file, _) in background_layers(view) {
        let bitmap = render.bitmap(&document.assets, &file);
        widest = widest.max(x.max(0) as u32 + bitmap.width);
        tallest = tallest.max(y.max(0) as u32 + bitmap.height);
    }
    (widest, tallest)
}

/// The window's shape: every background layer's non-keyed pixels,
/// positioned where the layer sits. Hidden layers shape nothing. When
/// the shape covers the whole view, there is no mask to be had.
fn window_mask(
    render: &mut Render,
    document: &SkinDocument,
    view: &View,
    (width, height): (u32, u32),
) -> Option<Mask> {
    let mut inside = vec![false; (width * height) as usize];
    let mut any = false;
    for (x, y, file, key) in background_layers(view) {
        let bitmap = render.keyed_bitmap(&document.assets, &file, key);
        let Some(bitmap) = bitmap else { continue };
        for dy in 0..bitmap.height {
            let y = y + dy as i32;
            if y < 0 || y >= height as i32 {
                continue;
            }
            for dx in 0..bitmap.width {
                let x = x + dx as i32;
                if x < 0 || x >= width as i32 {
                    continue;
                }
                if bitmap.pixel(dx, dy).is_some_and(|pixel| pixel[3] > 0) {
                    inside[y as usize * width as usize + x as usize] = true;
                    any = true;
                }
            }
        }
    }
    any.then(|| Mask::from_pixels(width, height, |x, y| inside[(y * width + x) as usize]))
}

/// The skin's background layers: the view's own, then every visible
/// subview's, each with its position and the colour it keys out.
fn background_layers(view: &View) -> Vec<(i32, i32, String, Option<ir::Color>)> {
    let mut layers = Vec::new();
    if let Some(file) = &view.background.image {
        layers.push((0, 0, file.clone(), view.background.transparency_color));
    }
    for child in &view.children {
        collect_layers(child, (0, 0), &mut layers);
    }
    layers
}

fn collect_layers(
    element: &Element,
    at: (i32, i32),
    layers: &mut Vec<(i32, i32, String, Option<ir::Color>)>,
) {
    let common = element.common();
    if common.visible_bool() == Some(false) {
        return;
    }
    let left = common.left_i32().unwrap_or(0) + at.0;
    let top = common.top_i32().unwrap_or(0) + at.1;
    if let Element::Subview(subview) = element {
        if let Some(file) = &subview.background.image {
            layers.push((
                left,
                top,
                file.clone(),
                subview.background.transparency_color,
            ));
        }
        for child in &subview.children {
            collect_layers(child, (left, top), layers);
        }
    }
}

/// The drawing surface of one view: where it sits on screen, how large
/// a skin pixel is, and the shape nothing paints outside.
struct Skin<'a> {
    ui: &'a Ui,
    origin: Pos2,
    unit: f32,
    mask: Option<&'a Mask>,
}

impl Skin<'_> {
    fn screen(&self, x: i32, y: i32) -> Pos2 {
        self.origin + Vec2::new(x as f32, y as f32) * self.unit
    }

    fn rect(&self, x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect::from_min_size(
            self.screen(x, y),
            Vec2::new(width as f32, height as f32) * self.unit,
        )
    }

    /// A block of skin pixels in one colour.
    fn fill(&self, x: i32, y: i32, width: u32, height: u32, color: ir::Color) {
        let color = Color32::from_rgb(color[0], color[1], color[2]);
        let block = |x: i32, y: i32, width: u32, height: u32| {
            self.ui
                .painter()
                .rect_filled(self.rect(x, y, width, height), 0.0, color);
        };
        match self.mask {
            None => block(x, y, width, height),
            Some(mask) => {
                for row in 0..height as i32 {
                    for (start, end) in mask.spans((y + row).max(0) as u32) {
                        let from = (*start as i32).max(x);
                        let to = (*end as i32).min(x + width as i32);
                        if to > from {
                            block(from, y + row, (to - from) as u32, 1);
                        }
                    }
                }
            }
        }
    }

    /// A bitmap at a skin position, in `alpha`, clipped to the window's
    /// shape. Rows the shape leaves out are left unpainted, span by
    /// span, through the same painter calls.
    fn blit(
        &self,
        painter: &egui::Painter,
        texture: TextureId,
        bitmap: (u32, u32),
        at: (i32, i32),
        alpha: u8,
    ) {
        let tint = Color32::from_white_alpha(alpha);
        let (width, height) = bitmap;
        let piece = |dx: u32, dy: u32, columns: u32, rows: u32| {
            let uv = Rect::from_min_max(
                Pos2::new(dx as f32 / width as f32, dy as f32 / height as f32),
                Pos2::new(
                    (dx + columns) as f32 / width as f32,
                    (dy + rows) as f32 / height as f32,
                ),
            );
            let dest = self.rect(at.0 + dx as i32, at.1 + dy as i32, columns, rows);
            painter.image(texture, dest, uv, tint);
        };
        match self.mask {
            None => piece(0, 0, width, height),
            Some(mask) => {
                for dy in 0..height {
                    let y = at.1 + dy as i32;
                    if y < 0 {
                        continue;
                    }
                    for (start, end) in mask.spans(y as u32) {
                        let from = (*start as i32 - at.0).clamp(0, width as i32) as u32;
                        let to = (*end as i32 - at.0).clamp(0, width as i32) as u32;
                        if to > from {
                            piece(from, dy, to - from, 1);
                        }
                    }
                }
            }
        }
    }

    /// A bitmap tiled across an area. Each tile is drawn whole through a
    /// painter clipped to the area, and the window's shape still decides
    /// what shows of it.
    fn tiled(&self, texture: TextureId, bitmap: (u32, u32), area: (i32, i32, u32, u32), alpha: u8) {
        let (tile_width, tile_height) = (bitmap.0.max(1), bitmap.1.max(1));
        let clip = self
            .rect(area.0, area.1, area.2, area.3)
            .intersect(self.ui.clip_rect());
        let painter = self.ui.painter().with_clip_rect(clip);
        for ty in (0..area.3).step_by(tile_height as usize) {
            for tx in (0..area.2).step_by(tile_width as usize) {
                self.blit(
                    &painter,
                    texture,
                    bitmap,
                    (area.0 + tx as i32, area.1 + ty as i32),
                    alpha,
                );
            }
        }
    }
}

/// What the painters reach for together: the skin's definition, its
/// decoded art, and the frame's context.
struct Art<'a> {
    document: &'a SkinDocument,
    render: &'a mut Render,
    ctx: &'a Context,
}

/// One element of the view, at its parent's position.
fn paint_element(skin: &Skin, art: &mut Art, element: &Element, at: (i32, i32)) {
    let common = element.common();
    if common.visible_bool() == Some(false) {
        return;
    }
    let left = common.left_i32().unwrap_or(0) + at.0;
    let top = common.top_i32().unwrap_or(0) + at.1;
    let alpha = common.alpha_blend.unwrap_or(255);
    match element {
        Element::Subview(subview) => {
            paint_background(skin, art, &subview.background, common, (left, top), alpha);
            for child in &subview.children {
                paint_element(skin, art, child, (left, top));
            }
        }
        Element::Image(image) => {
            let Some(file) = &image.image else { return };
            let area = element_area(art.render, art.document, common, file);
            paint_picture(
                skin,
                art,
                file,
                image.transparency_color,
                (left, top, area.0, area.1),
                image.tiled,
                alpha,
            );
        }
        Element::Button(button) => {
            let Some(file) = &button.states.image else {
                return;
            };
            paint_picture(
                skin,
                art,
                file,
                button.transparency_color,
                (left, top, 0, 0),
                button.tiled,
                alpha,
            );
        }
        Element::ButtonGroup(group) => {
            let Some(file) = &group.states.image else {
                return;
            };
            if group.show_background {
                let area = element_area(art.render, art.document, common, file);
                paint_picture(
                    skin,
                    art,
                    file,
                    group.transparency_color,
                    (left, top, area.0, area.1),
                    false,
                    alpha,
                );
            }
        }
        Element::Slider(slider) => paint_slider(skin, art, slider, (left, top), alpha),
        Element::Text(text) => paint_text(skin, text, (left, top), alpha),
        Element::Other(_) => {}
    }
}

/// The area of an element that carries art: what its attributes say, or
/// failing that, the art's own size.
fn element_area(
    render: &mut Render,
    document: &SkinDocument,
    common: &ir::Common,
    file: &str,
) -> (u32, u32) {
    let bitmap = render.bitmap(&document.assets, file);
    let width = common.width_i32().map_or(bitmap.width, |w| w.max(0) as u32);
    let height = common
        .height_i32()
        .map_or(bitmap.height, |h| h.max(0) as u32);
    (width, height)
}

/// A background layer: the colour behind it, then the art on it.
fn paint_background(
    skin: &Skin,
    art: &mut Art,
    background: &Background,
    common: &ir::Common,
    at: (i32, i32),
    alpha: u8,
) {
    let Some(file) = &background.image else {
        // A colour alone: the element's own size, or nothing to fill.
        if let Some(color) = background.color {
            let width = common.width_i32().unwrap_or(0).max(0) as u32;
            let height = common.height_i32().unwrap_or(0).max(0) as u32;
            if width > 0 && height > 0 {
                skin.fill(at.0, at.1, width, height, color);
            }
        }
        return;
    };
    let area = element_area(art.render, art.document, common, file);
    if let Some(color) = background.color {
        skin.fill(at.0, at.1, area.0, area.1, color);
    }
    paint_picture(
        skin,
        art,
        file,
        background.transparency_color,
        (at.0, at.1, area.0, area.1),
        background.tiled,
        alpha,
    );
}

/// A picture at a position: one draw, or a grid of them when tiled.
fn paint_picture(
    skin: &Skin,
    art: &mut Art,
    file: &str,
    key: Option<ir::Color>,
    at: (i32, i32, u32, u32),
    tiled: bool,
    alpha: u8,
) {
    let Some(texture) = art.render.texture(art.ctx, &art.document.assets, file, key) else {
        return;
    };
    let bitmap = art.render.bitmap(&art.document.assets, file);
    let size = (bitmap.width, bitmap.height);
    if size.0 == 0 || size.1 == 0 {
        return;
    }
    if tiled && (at.2 > size.0 || at.3 > size.1) {
        skin.tiled(texture, size, (at.0, at.1, at.2, at.3), alpha);
    } else {
        skin.blit(skin.ui.painter(), texture, size, (at.0, at.1), alpha);
    }
}

/// A slider: the track tiled across its area, the thumb drawn at the
/// value's place along it.
fn paint_slider(skin: &Skin, art: &mut Art, slider: &ir::Slider, at: (i32, i32), alpha: u8) {
    let track = slider.background_image.as_deref().map(|file| {
        (
            file.to_string(),
            element_area(art.render, art.document, &slider.common, file),
        )
    });
    let texture = track.as_ref().and_then(|(file, _)| {
        art.render.texture(
            art.ctx,
            &art.document.assets,
            file,
            slider.transparency_color,
        )
    });
    if let (Some((file, area)), Some(texture)) = (&track, texture) {
        let bitmap = art.render.bitmap(&art.document.assets, file);
        skin.tiled(
            texture,
            (bitmap.width, bitmap.height),
            (at.0, at.1, area.0, area.1),
            alpha,
        );
    }
    let Some(thumb) = &slider.thumb_image else {
        return;
    };
    let thumb_bitmap = art.render.bitmap(&art.document.assets, thumb);
    if thumb_bitmap.width == 0 {
        return;
    }
    let Some(texture) = art.render.texture(
        art.ctx,
        &art.document.assets,
        thumb,
        slider.transparency_color,
    ) else {
        return;
    };
    // Where the value sits along the track, as a share of the travel
    // the thumb has: the area less the border on each end and the thumb
    // itself. A value the skin binds to the player reads as the minimum
    // until the bindings arrive.
    let fraction = slider
        .value
        .as_ref()
        .and_then(Value::as_f64)
        .map(|value| {
            let min = slider.min.as_ref().and_then(Value::as_f64).unwrap_or(0.0);
            let max = slider.max.as_ref().and_then(Value::as_f64).unwrap_or(100.0);
            ((value - min) / (max - min)).clamp(0.0, 1.0)
        })
        .unwrap_or(0.0);
    let border = slider.border_size.max(0);
    let area = track
        .map(|(_, area)| area)
        .unwrap_or((thumb_bitmap.width, thumb_bitmap.height));
    let position = match slider.direction {
        ir::Direction::Horizontal => {
            let travel = area.0 as i32 - 2 * border - thumb_bitmap.width as i32;
            let x = at.0 + border + (fraction * travel as f64).round() as i32;
            (x, at.1)
        }
        ir::Direction::Vertical => {
            // The minimum sits at the bottom, so the thumb rises.
            let travel = area.1 as i32 - 2 * border - thumb_bitmap.height as i32;
            let y = at.1 + border + ((1.0 - fraction) * travel as f64).round() as i32;
            (at.0, y)
        }
    };
    skin.blit(
        skin.ui.painter(),
        texture,
        (thumb_bitmap.width, thumb_bitmap.height),
        position,
        alpha,
    );
}

/// A piece of text in its box, in a system font. Values bound to the
/// player wait for the bindings pass; a scrolling value stands still
/// for now.
fn paint_text(skin: &Skin, text: &ir::Text, at: (i32, i32), alpha: u8) {
    let Some(string) = text.value.as_ref().and_then(Value::as_literal) else {
        return;
    };
    if string.is_empty() {
        return;
    }
    let size = text
        .font_size
        .as_ref()
        .and_then(Value::as_f64)
        .unwrap_or(8.0) as f32
        * skin.unit;
    let mut color = text
        .foreground_color
        .map_or(Color32::WHITE, |c| Color32::from_rgb(c[0], c[1], c[2]));
    if alpha < 255 {
        color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
    }
    let font = FontId::proportional(size.max(1.0));
    let painter = match (text.common.width_i32(), text.common.height_i32()) {
        (Some(width), Some(height)) => {
            let clip = skin
                .rect(at.0, at.1, width.max(0) as u32, height.max(0) as u32)
                .intersect(skin.ui.clip_rect());
            skin.ui.painter().with_clip_rect(clip)
        }
        _ => skin.ui.painter().clone(),
    };
    let galley = painter.layout_no_wrap(string.to_string(), font.clone(), color);
    let width = text.common.width_i32().map(|w| w as f32 * skin.unit);
    let x = match text.justification {
        ir::Justification::Left => 0.0,
        ir::Justification::Center => {
            ((width.unwrap_or(galley.size().x) - galley.size().x) / 2.0).max(0.0)
        }
        ir::Justification::Right => (width.unwrap_or(galley.size().x) - galley.size().x).max(0.0),
    };
    if let Some(background) = text.background_color {
        let fill_color = Color32::from_rgb(background[0], background[1], background[2]);
        let rect = Rect::from_min_size(
            skin.screen(at.0, at.1),
            Vec2::new(width.unwrap_or(galley.size().x), galley.size().y),
        );
        painter.rect_filled(rect, 0.0, fill_color);
    }
    painter.text(
        skin.screen(at.0, at.1) + Vec2::new(x, 0.0),
        Align2::LEFT_TOP,
        string,
        font,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wmp::ir::theme;
    use crate::wmp::xml;

    /// A PNG of the given size, magenta in the corners, the rest green.
    fn toothy_png(width: u32, height: u32) -> Vec<u8> {
        let mut image = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 255, 0, 255]));
        for corner in [
            (0, 0),
            (width - 1, 0),
            (0, height - 1),
            (width - 1, height - 1),
        ] {
            image.put_pixel(corner.0, corner.1, image::Rgba([255, 0, 255, 255]));
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn document(definition: &[u8]) -> (SkinDocument, Render) {
        let (theme, views) = theme(&xml::parse(definition).unwrap()).unwrap();
        let document = SkinDocument {
            name: "test".into(),
            theme,
            views,
            scripts: Vec::new(),
            assets: Assets::from_files([
                ("base.bmp", toothy_png(10, 8)),
                ("plain.png", toothy_png(4, 4)),
            ]),
        };
        (document, Render::default())
    }

    #[test]
    fn a_view_is_the_size_its_attributes_say() {
        let (document, mut render) = document(
            br#"<theme><view width="320" height="240" backgroundImage="base.bmp"/></theme>"#,
        );
        let view = document.main_view().unwrap();
        assert_eq!(view_size(&mut render, &document, view), (320, 240));
    }

    #[test]
    fn a_view_without_a_size_is_its_background() {
        let (document, mut render) =
            document(br#"<theme><view backgroundImage="base.bmp"/></theme>"#);
        let view = document.main_view().unwrap();
        assert_eq!(view_size(&mut render, &document, view), (10, 8));
    }

    #[test]
    fn the_window_shape_is_the_background_layers_taken_together() {
        let (document, mut render) = document(
            br##"<theme><view width="10" height="8" backgroundImage="base.bmp"
                transparencyColor="#FF00FF"/></theme>"##,
        );
        let view = document.main_view().unwrap();
        let mask = window_mask(&mut render, &document, view, (10, 8)).unwrap();
        // The magenta corners are outside the window; the rest is in.
        assert!(!mask.contains(0, 0));
        assert!(!mask.contains(9, 7));
        assert!(mask.contains(1, 1));
        assert!(mask.contains(5, 4));
        assert_eq!((mask.width, mask.height), (10, 8));
    }

    #[test]
    fn a_subview_shaped_window_keeps_hidden_layers_out_of_the_shape() {
        let (document, mut render) = document(
            br##"<theme><view width="10" height="8">
                <subview left="2" top="1" backgroundImage="plain.png"
                    transparencyColor="#FF00FF"/>
                <subview left="0" top="0" visible="false"
                    backgroundImage="base.bmp" transparencyColor="#FF00FF"/>
            </view></theme>"##,
        );
        let view = document.main_view().unwrap();
        let mask = window_mask(&mut render, &document, view, (10, 8)).unwrap();
        // Only the visible 4x4 subview, magenta corners keyed, shapes
        // the window: (0,0) belongs to the hidden layer and stays out.
        assert!(!mask.contains(0, 0));
        assert!(!mask.contains(2, 1), "a keyed corner is not the window");
        assert!(!mask.contains(5, 4), "the far corner is keyed too");
        assert!(mask.contains(3, 2));
        assert!(mask.contains(4, 3));
        assert!(!mask.contains(8, 6));
    }

    #[test]
    fn a_background_without_a_key_shapes_every_pixel() {
        let (document, mut render) = document(
            br##"<theme><view width="10" height="8" backgroundImage="plain.png"/></theme>"##,
        );
        let view = document.main_view().unwrap();
        // A 4x4 layer in a 10x8 view: only those pixels are the window,
        // so the mask is worth having even without a key colour.
        let mask = window_mask(&mut render, &document, view, (10, 8)).unwrap();
        assert!(mask.contains(0, 0));
        assert!(mask.contains(3, 3));
        assert!(!mask.contains(4, 4));
    }

    #[test]
    fn a_keyed_texture_has_its_colour_cut_out() {
        let (document, mut render) = document(
            br##"<theme><view width="10" height="8" backgroundImage="base.bmp"
                transparencyColor="#FF00FF"/></theme>"##,
        );
        let view = document.main_view().unwrap();
        let mask = window_mask(&mut render, &document, view, (10, 8)).unwrap();
        assert!(
            render
                .keyed_bitmap(&document.assets, "BASE.BMP", Some([255, 0, 255]))
                .is_some()
        );
        assert!(
            render
                .keyed_bitmap(&document.assets, "missing.bmp", None)
                .is_none()
        );
        // The keyed copy is shared with the mask's; the raw one is not
        // keyed at all.
        let raw = render.bitmap(&document.assets, "base.bmp");
        assert_eq!(raw.pixel(0, 0), Some([255, 0, 255, 255]));
        let _ = mask;
    }
}
