//! Drawing Windows Media Player skins: a definition's view, painted the
//! way its author laid it out, with its controls working.
//!
//! The window's shape is every background layer's non-keyed pixels taken
//! together — the view's own background, and the backgrounds of the
//! visible subviews, positioned where they sit. Nothing paints outside
//! that shape, row span by row span, the way the Winamp window does.
//! Elements paint in z order, document order breaking ties, with each
//! child positioned in its parent's coordinates. Art decodes once and
//! uploads once per window; the [`Render`] holds both between frames.
//!
//! The controls answer the pointer: buttons wear their hover and pressed
//! bitmaps, a button group tells which of its buttons the pointer is on
//! through its mapping bitmap and paints the group's state bitmap only
//! through that button's own colour region, and sliders follow a drag
//! and hand back a [`SkinAction`] when it settles. The actions are the
//! skin's wishes; the caller decides what of them the player can do.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{
    Align2, Color32, ColorImage, Context, FontId, Id, Pos2, Rect, Sense, TextureHandle, TextureId,
    TextureOptions, Ui, Vec2,
};

use crate::app::NowPlaying;
use crate::skin::{Bitmap, Mask};
use crate::wmp::ir::{self, Background, Binding, Element, Value, View};
use crate::wmp::{Assets, SkinDocument};

/// What a skin control asks of the player, once a click or a drag has
/// settled. The window's own verbs come with them.
#[derive(Clone, Debug, PartialEq)]
pub enum SkinAction {
    TogglePlay,
    Stop,
    Next,
    Previous,
    /// Seek to a position, in seconds from the start.
    SeekTo(f64),
    /// Set the volume, 0 to 100.
    SetVolume(f64),
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,
    Minimize,
    Close,
    ReturnToMediaCenter,
}

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
    /// A mapping bitmap's region for one button colour, by file and
    /// colour: the pixels that are that button's to click.
    regions: HashMap<(String, ir::Color), Mask>,
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

    /// The pixels of a mapping bitmap that answer to one button colour,
    /// once per file and colour. A region that covers the whole bitmap
    /// is no region, and comes back as `None`.
    fn region(&mut self, assets: &Assets, file: &str, color: ir::Color) -> Option<Mask> {
        let name = file.to_ascii_lowercase();
        let key = (name.clone(), color);
        if !self.regions.contains_key(&key) {
            let bitmap = self.bitmap(assets, file);
            let mask = Mask::from_pixels(bitmap.width, bitmap.height, |x, y| {
                bitmap.pixel(x, y).is_some_and(|pixel| pixel[..3] == color)
            });
            self.regions.insert(key.clone(), mask);
        }
        let mask = &self.regions[&key];
        (!mask.is_everything()).then(|| mask.clone())
    }
}

/// Where a drag leaves a slider, as a share of its travel.
#[derive(Clone, Copy, PartialEq)]
enum SliderEvent {
    None,
    /// The thumb is under the pointer's thumb; the value follows it.
    Dragging(f64),
    /// The press ended, or a bare click landed: the value is settled.
    Committed(f64),
}

/// Draws the skin's main view, top-left at `origin`, with `unit` screen
/// pixels to the skin pixel, and answers with what its controls asked
/// of the player this frame.
pub fn show(
    ui: &mut Ui,
    document: &Arc<SkinDocument>,
    render: &mut Render,
    origin: Pos2,
    unit: f32,
    media: Option<&NowPlaying>,
) -> Vec<SkinAction> {
    let Some(view) = document.main_view() else {
        return Vec::new();
    };
    let ctx = ui.ctx().clone();
    let size = view_size(render, document, view);
    if size.0 == 0 || size.1 == 0 {
        return Vec::new();
    }
    let mask = window_mask(render, document, view, size);
    let mut actions = Vec::new();
    let mut next_id = 0usize;
    let mut skin = Skin {
        ui,
        origin,
        unit,
        mask: mask.as_ref(),
        media,
        actions: &mut actions,
        next_id: &mut next_id,
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
        paint_element(&mut skin, &mut art, element, (0, 0));
    }
    actions
}

/// The demo's look at a skin: it floats over the big window, small
/// margin, at one-to-one pixels, with the player it has. The screenshot
/// surface answers "does Toothy show its tooth" and, since the controls
/// landed, lets a skin drive the demo's player.
#[cfg(any(test, feature = "demo"))]
pub fn preview(app: &mut crate::app::App, ui: &mut Ui) {
    let media = app.now_playing();
    let Some((document, render)) = app.wmp_preview.as_mut() else {
        return;
    };
    let origin = ui.max_rect().min + Vec2::splat(16.0);
    for action in show(ui, document, render, origin, 1.0, media.as_ref()) {
        let action = match action {
            SkinAction::TogglePlay => crate::model::Action::TogglePlay,
            // Stopping where Winamp stops: a playing track pauses, a
            // paused one goes back to its start.
            SkinAction::Stop => {
                if media.as_ref().is_some_and(|now| now.playing) {
                    crate::model::Action::TogglePlay
                } else {
                    crate::model::Action::Seek(0)
                }
            }
            SkinAction::Next => crate::model::Action::Next,
            SkinAction::Previous => crate::model::Action::Previous,
            SkinAction::SeekTo(seconds) => {
                crate::model::Action::Seek((seconds.max(0.0) * 1000.0) as u32)
            }
            SkinAction::SetVolume(volume) => {
                crate::model::Action::SetVolume(volume.round().clamp(0.0, 100.0) as u8)
            }
            SkinAction::ToggleMute => crate::model::Action::ToggleMute,
            SkinAction::ToggleShuffle => crate::model::Action::ToggleShuffle,
            SkinAction::CycleRepeat => crate::model::Action::CycleRepeat,
            // The window verbs wait for the skin to be a window.
            SkinAction::Minimize | SkinAction::Close | SkinAction::ReturnToMediaCenter => continue,
        };
        app.actions.push(action);
    }
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
/// A shape in skin coordinates: a mask and where its own (0,0) sits.
struct Region<'a> {
    mask: &'a Mask,
    at: (i32, i32),
}

struct Skin<'a> {
    ui: &'a Ui,
    origin: Pos2,
    unit: f32,
    mask: Option<&'a Mask>,
    /// What the player is doing, for the controls that show it.
    media: Option<&'a NowPlaying>,
    /// What this frame's controls asked of the player.
    actions: &'a mut Vec<SkinAction>,
    /// The source of interaction ids: elements in a stable walk order.
    next_id: &'a mut usize,
}

impl Skin<'_> {
    /// An interaction id for the element being painted: the walk order
    /// is the same every frame, so the id is too.
    fn next_id(&mut self) -> Id {
        let id = Id::new(("wmp", *self.next_id));
        *self.next_id += 1;
        id
    }

    /// An element's rect as an interaction target, with a pointer hand.
    fn interact(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        sense: Sense,
    ) -> egui::Response {
        let id = self.next_id();
        let rect = self.rect(x, y, width, height);
        self.ui
            .interact(rect, id, sense)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    /// The pointer's position in skin coordinates, when it is over the
    /// window at all.
    fn pointer(&self, response: &egui::Response) -> Option<(f32, f32)> {
        let pos = response.interact_pointer_pos()?;
        Some((
            (pos.x - self.origin.x) / self.unit,
            (pos.y - self.origin.y) / self.unit,
        ))
    }

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

    /// A bitmap drawn only where two shapes agree: the window's, and a
    /// region's — a button's share of a group bitmap — placed at
    /// `region_at` in skin coordinates.
    fn blit_through(
        &self,
        painter: &egui::Painter,
        texture: TextureId,
        bitmap: (u32, u32),
        at: (i32, i32),
        alpha: u8,
        region: &Region,
    ) {
        let tint = Color32::from_white_alpha(alpha);
        let (width, height) = bitmap;
        let piece = |dx: i32, dy: u32, columns: i32| {
            let uv = Rect::from_min_max(
                Pos2::new(dx as f32 / width as f32, dy as f32 / height as f32),
                Pos2::new(
                    (dx + columns) as f32 / width as f32,
                    (dy + 1) as f32 / height as f32,
                ),
            );
            let dest = self.rect(at.0 + dx, at.1 + dy as i32, columns as u32, 1);
            painter.image(texture, dest, uv, tint);
        };
        for dy in 0..height {
            let y = at.1 + dy as i32;
            if y < 0 {
                continue;
            }
            let window = match self.mask {
                None => &[(0u32, u32::MAX)][..],
                Some(mask) => mask.spans(y as u32),
            };
            for (window_start, window_end) in window {
                for (region_start, region_end) in region.mask.spans((y - region.at.1).max(0) as u32)
                {
                    // The region's spans are its own; bring them to skin
                    // coordinates and keep what both shapes agree on.
                    let from = (*region_start as i32 + region.at.0)
                        .max(*window_start as i32)
                        .max(at.0);
                    let to = (*region_end as i32 + region.at.0)
                        .min(*window_end as i32)
                        .min(at.0 + width as i32);
                    if to > from {
                        piece(from - at.0, dy, to - from);
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
fn paint_element(skin: &mut Skin, art: &mut Art, element: &Element, at: (i32, i32)) {
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
        Element::Button(button) => paint_button(skin, art, button, (left, top), alpha),
        Element::ButtonGroup(group) => paint_group(skin, art, group, (left, top), alpha),
        Element::Slider(slider) => paint_slider(skin, art, slider, (left, top), alpha),
        Element::Text(text) => paint_text(skin, text, (left, top), alpha),
        Element::Other(_) => {}
    }
}

/// A button on its own: it wears the state image the pointer asks for,
/// and hands its action over on a click.
fn paint_button(skin: &mut Skin, art: &mut Art, button: &ir::Button, at: (i32, i32), alpha: u8) {
    let Some(file) = &button.states.image else {
        return;
    };
    let bitmap = art.render.bitmap(&art.document.assets, file);
    if bitmap.width == 0 {
        return;
    }
    let response = skin.interact(at.0, at.1, bitmap.width, bitmap.height, Sense::click());
    let state = if response.is_pointer_button_down_on() {
        button.states.down.as_ref().or(Some(file))
    } else if response.hovered() {
        button.states.hover.as_ref().or(Some(file))
    } else {
        Some(file)
    };
    let Some(state) = state else { return };
    paint_picture(
        skin,
        art,
        state,
        button.transparency_color,
        (at.0, at.1, 0, 0),
        button.tiled,
        alpha,
    );
    if response.clicked()
        && let Some(action) = button_action(&button.action)
    {
        skin.actions.push(action);
    }
}

/// A group of buttons sharing one bitmap per state, told apart by the
/// colour under the pointer in the mapping bitmap. The hover and
/// pressed state bitmaps are painted only through the hovered button's
/// own colour region, the way the player composited them.
fn paint_group(skin: &mut Skin, art: &mut Art, group: &ir::ButtonGroup, at: (i32, i32), alpha: u8) {
    let Some(file) = &group.states.image else {
        return;
    };
    let Some(mapping) = &group.mapping_image else {
        return;
    };
    let bitmap = art.render.bitmap(&art.document.assets, file);
    let map_bitmap = art.render.bitmap(&art.document.assets, mapping);
    if bitmap.width == 0 || map_bitmap.width == 0 {
        return;
    }
    let area = (
        group
            .common
            .width_i32()
            .map_or(bitmap.width, |w| w.max(0) as u32),
        group
            .common
            .height_i32()
            .map_or(bitmap.height, |h| h.max(0) as u32),
    );
    if !group.show_background {
        return;
    }
    let response = skin.interact(at.0, at.1, area.0, area.1, Sense::click());
    let pointer = skin.pointer(&response);
    let hovered = element_at_pointer(
        &map_bitmap,
        &group.buttons,
        pointer.map(|(x, y)| ((x - at.0 as f32) as i32, (y - at.1 as f32) as i32)),
    );
    // The resting bitmap always goes down first.
    paint_picture(
        skin,
        art,
        file,
        group.transparency_color,
        (at.0, at.1, area.0, area.1),
        false,
        alpha,
    );
    let pressed = response.is_pointer_button_down_on();
    let state_image = if pressed {
        group.states.down.as_ref()
    } else {
        group.states.hover.as_ref()
    };
    if let (Some(region_color), Some(state)) = (
        hovered.and_then(|index| group.buttons[index].mapping_color),
        state_image,
    ) {
        // The state bitmap, seen only through the hovered button's
        // region of the mapping bitmap.
        if let Some(region) = art
            .render
            .region(&art.document.assets, mapping, region_color)
            && let Some(texture) = art.render.texture(
                art.ctx,
                &art.document.assets,
                state,
                group.transparency_color,
            )
        {
            let region = Region { mask: &region, at };
            skin.blit_through(
                skin.ui.painter(),
                texture,
                (bitmap.width, bitmap.height),
                at,
                alpha,
                &region,
            );
        }
    }
    if response.clicked() {
        let Some(index) = element_at_pointer(
            &map_bitmap,
            &group.buttons,
            pointer.map(|(x, y)| ((x - at.0 as f32) as i32, (y - at.1 as f32) as i32)),
        ) else {
            return;
        };
        if let Some(action) = button_action(&group.buttons[index].action) {
            skin.actions.push(action);
        }
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
/// value's place along it, following a drag and handing its value over
/// when the drag settles.
fn paint_slider(skin: &mut Skin, art: &mut Art, slider: &ir::Slider, at: (i32, i32), alpha: u8) {
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
    if thumb_bitmap.width == 0 || thumb_bitmap.height == 0 {
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
    let border = slider.border_size.max(0);
    let area = track
        .map(|(_, area)| area)
        .unwrap_or((thumb_bitmap.width, thumb_bitmap.height));
    let travel = match slider.direction {
        ir::Direction::Horizontal => area.0 as i32 - 2 * border - thumb_bitmap.width as i32,
        ir::Direction::Vertical => area.1 as i32 - 2 * border - thumb_bitmap.height as i32,
    }
    .max(0);
    // Where the value sits along the track, as a share of the travel
    // the thumb has. What the skin binds to the player comes from the
    // player; what it wrote as a number comes from itself.
    let bound = slider
        .value
        .as_ref()
        .and_then(|value| bound_value(value, skin.media));
    let fraction = |value: f64| {
        let min = slider.min.as_ref().and_then(Value::as_f64).unwrap_or(0.0);
        let max = slider.max.as_ref().and_then(Value::as_f64).unwrap_or(100.0);
        if max > min {
            ((value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let response = match slider.direction {
        ir::Direction::Horizontal => {
            skin.interact(at.0, at.1, area.0, area.1, Sense::click_and_drag())
        }
        ir::Direction::Vertical => {
            skin.interact(at.0, at.1, area.0, area.1, Sense::click_and_drag())
        }
    };
    let pointer = skin.pointer(&response).map(|(x, y)| {
        match slider.direction {
            ir::Direction::Horizontal => {
                (x - at.0 as f32 - border as f32 - thumb_bitmap.width as f32 / 2.0)
                    / travel.max(1) as f32
            }
            ir::Direction::Vertical => {
                1.0 - (y - at.1 as f32 - border as f32 - thumb_bitmap.height as f32 / 2.0)
                    / travel.max(1) as f32
            }
        }
        .clamp(0.0, 1.0) as f64
    });
    // The thumb follows the drag from where the player had it, and the
    // value hands over when the drag settles or a bare click lands.
    let mut event = SliderEvent::None;
    if (response.drag_started() || response.dragged())
        && let Some(share) = pointer
    {
        event = SliderEvent::Dragging(share);
    }
    if (response.drag_stopped() || response.clicked())
        && let Some(share) = pointer
    {
        event = SliderEvent::Committed(share);
    }
    let shown = match event {
        SliderEvent::Dragging(share) | SliderEvent::Committed(share) => share,
        SliderEvent::None => bound.map_or(0.0, fraction),
    };
    if let SliderEvent::Committed(share) = event
        && let Some(action) = slider_action(slider, share)
    {
        skin.actions.push(action);
    }
    let position = match slider.direction {
        ir::Direction::Horizontal => (at.0 + border + (shown * travel as f64).round() as i32, at.1),
        ir::Direction::Vertical => (
            at.0,
            at.1 + border + ((1.0 - shown) * travel as f64).round() as i32,
        ),
    };
    skin.blit(
        skin.ui.painter(),
        texture,
        (thumb_bitmap.width, thumb_bitmap.height),
        position,
        alpha,
    );
}

/// A piece of text in its box, in a system font. A value bound to the
/// player reads from what the player is doing; a scrolling value stands
/// still for now.
fn paint_text(skin: &mut Skin, text: &ir::Text, at: (i32, i32), alpha: u8) {
    let string = match text
        .value
        .as_ref()
        .and_then(|value| bound_text(value, skin.media))
    {
        Some(string) => string,
        None => return,
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
    let galley = painter.layout_no_wrap(string.clone(), font.clone(), color);
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

/// The action a control's definition asks for. Handlers the skin has no
/// player verb for — an equalizer's reset, a script's named function —
/// ask for nothing.
fn button_action(action: &ir::Action) -> Option<SkinAction> {
    Some(match action {
        ir::Action::Play | ir::Action::Pause => SkinAction::TogglePlay,
        ir::Action::Stop => SkinAction::Stop,
        ir::Action::Next => SkinAction::Next,
        ir::Action::Previous => SkinAction::Previous,
        ir::Action::Mute => SkinAction::ToggleMute,
        ir::Action::Shuffle => SkinAction::ToggleShuffle,
        ir::Action::Repeat => SkinAction::CycleRepeat,
        ir::Action::Minimize => SkinAction::Minimize,
        ir::Action::Close => SkinAction::Close,
        ir::Action::ReturnToMediaCenter => SkinAction::ReturnToMediaCenter,
        ir::Action::None
        | ir::Action::FastForward
        | ir::Action::Rewind
        | ir::Action::OpenView(_)
        | ir::Action::CloseView(_)
        | ir::Action::ResetEq
        | ir::Action::EffectsNext
        | ir::Action::EffectsPrevious
        | ir::Action::Unhandled(_) => return None,
    })
}

/// The action a settled slider stands for: where the position goes, and
/// what the volume asks for outright.
fn slider_action(slider: &ir::Slider, share: f64) -> Option<SkinAction> {
    let min = slider.min.as_ref().and_then(Value::as_f64).unwrap_or(0.0);
    let max = slider.max.as_ref().and_then(Value::as_f64).unwrap_or(100.0);
    let value = min + share * (max - min);
    match slider.binding.as_ref()? {
        Binding::Position => Some(SkinAction::SeekTo(value)),
        Binding::Volume => Some(SkinAction::SetVolume(value)),
        // Balance, the equalizer's bands, and the rest wait for the
        // player to serve them.
        _ => None,
    }
}

/// What a bound value reads from the player, as a number. A value the
/// skin wrote as a number is its own answer.
fn bound_value(value: &Value, media: Option<&NowPlaying>) -> Option<f64> {
    match value.binding() {
        Some(Binding::Volume) => Some(f64::from(media?.volume_percent)),
        Some(Binding::Position) => Some(f64::from(media?.position_ms) / 1000.0),
        Some(Binding::Duration) => Some(f64::from(media?.duration_ms) / 1000.0),
        Some(Binding::Balance) => Some(0.0),
        _ => value.as_f64(),
    }
}

/// What a bound value reads from the player, as text. A literal is its
/// own answer.
fn bound_text(value: &Value, media: Option<&NowPlaying>) -> Option<String> {
    if let Some(text) = value.as_literal() {
        return Some(text.to_string());
    }
    let media = media?;
    match value.binding()? {
        Binding::TrackName => Some(media.title.clone()),
        Binding::PositionString => Some(clock(media.position_ms)),
        Binding::DurationString => Some(clock(media.duration_ms)),
        _ => None,
    }
}

/// A duration as the player writes it: `3:42`.
fn clock(ms: u32) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Which of a group's buttons the pointer rests on, from the colour the
/// mapping bitmap shows there.
fn element_at_pointer(
    mapping: &Bitmap,
    buttons: &[ir::ButtonElement],
    at: Option<(i32, i32)>,
) -> Option<usize> {
    let (x, y) = at?;
    if x < 0 || y < 0 || x >= mapping.width as i32 || y >= mapping.height as i32 {
        return None;
    }
    let pixel = mapping.pixel(x as u32, y as u32)?;
    buttons
        .iter()
        .position(|button| button.mapping_color == Some([pixel[0], pixel[1], pixel[2]]))
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
    fn a_group_button_is_found_by_the_colour_under_the_pointer() {
        let mut mapping = Bitmap {
            width: 4,
            height: 2,
            rgba: vec![0; 4 * 4 * 2],
        };
        let mut put = |x: u32, y: u32, color: [u8; 3]| {
            let at = 4 * (y * 4 + x) as usize;
            mapping.rgba[at..at + 3].copy_from_slice(&color);
            mapping.rgba[at + 3] = 255;
        };
        put(0, 0, [255, 0, 0]);
        put(1, 0, [0, 255, 0]);
        put(0, 1, [0, 0, 255]);
        let buttons = [
            ir::ButtonElement {
                mapping_color: Some([255, 0, 0]),
                ..Default::default()
            },
            ir::ButtonElement {
                mapping_color: Some([0, 255, 0]),
                ..Default::default()
            },
            ir::ButtonElement {
                mapping_color: Some([0, 0, 255]),
                ..Default::default()
            },
        ];
        let at = |x: i32, y: i32| Some((x, y));
        assert_eq!(element_at_pointer(&mapping, &buttons, at(0, 0)), Some(0));
        assert_eq!(element_at_pointer(&mapping, &buttons, at(1, 0)), Some(1));
        assert_eq!(element_at_pointer(&mapping, &buttons, at(0, 1)), Some(2));
        assert_eq!(element_at_pointer(&mapping, &buttons, at(3, 1)), None);
        assert_eq!(element_at_pointer(&mapping, &buttons, at(-1, 0)), None);
        assert_eq!(element_at_pointer(&mapping, &buttons, None), None);
    }

    #[test]
    fn a_regions_mask_covers_only_its_own_colour() {
        let (document, mut render) = document(
            br##"<theme><view width="10" height="8">
                <subview backgroundImage="plain.png"/>
            </view></theme>"##,
        );
        // plain.png is 4x4, green with magenta corners: the magenta
        // region is its two corners, the green one everything else.
        let magenta = render
            .region(&document.assets, "plain.png", [255, 0, 255])
            .unwrap();
        assert!(magenta.contains(0, 0));
        assert!(!magenta.contains(1, 1));
        let green = render
            .region(&document.assets, "plain.png", [0, 255, 0])
            .unwrap();
        assert!(green.contains(1, 1));
        assert!(!green.contains(0, 0));
        // Asked twice, the same mask comes back from the cache.
        let again = render
            .region(&document.assets, "PLAIN.PNG", [0, 255, 0])
            .unwrap();
        assert_eq!(again, green);
    }

    #[test]
    fn only_the_player_verbs_become_actions() {
        assert_eq!(
            button_action(&ir::Action::Play),
            Some(SkinAction::TogglePlay)
        );
        assert_eq!(
            button_action(&ir::Action::Pause),
            Some(SkinAction::TogglePlay)
        );
        assert_eq!(button_action(&ir::Action::Stop), Some(SkinAction::Stop));
        assert_eq!(button_action(&ir::Action::Next), Some(SkinAction::Next));
        assert_eq!(
            button_action(&ir::Action::Previous),
            Some(SkinAction::Previous)
        );
        assert_eq!(
            button_action(&ir::Action::Mute),
            Some(SkinAction::ToggleMute)
        );
        assert_eq!(
            button_action(&ir::Action::Shuffle),
            Some(SkinAction::ToggleShuffle)
        );
        assert_eq!(
            button_action(&ir::Action::Repeat),
            Some(SkinAction::CycleRepeat)
        );
        assert_eq!(
            button_action(&ir::Action::Minimize),
            Some(SkinAction::Minimize)
        );
        assert_eq!(button_action(&ir::Action::Close), Some(SkinAction::Close));
        assert_eq!(
            button_action(&ir::Action::ReturnToMediaCenter),
            Some(SkinAction::ReturnToMediaCenter)
        );
        // A script's named function asks for nothing at all.
        assert_eq!(
            button_action(&ir::Action::Unhandled("TogglePl();".into())),
            None
        );
        assert_eq!(button_action(&ir::Action::ResetEq), None);
    }

    #[test]
    fn a_settled_slider_stands_for_its_binding() {
        let seek = ir::Slider {
            binding: Some(Binding::Position),
            min: Some(Value::Literal("0".into())),
            max: Some(Value::Literal("300".into())),
            ..Default::default()
        };
        assert_eq!(slider_action(&seek, 0.0), Some(SkinAction::SeekTo(0.0)));
        assert_eq!(slider_action(&seek, 0.5), Some(SkinAction::SeekTo(150.0)));
        let volume = ir::Slider {
            binding: Some(Binding::Volume),
            ..Default::default()
        };
        assert_eq!(
            slider_action(&volume, 0.25),
            Some(SkinAction::SetVolume(25.0))
        );
        // A slider with no player behind it asks for nothing.
        let unbound = ir::Slider::default();
        assert_eq!(slider_action(&unbound, 0.5), None);
    }

    #[test]
    fn bound_values_read_the_player() {
        let media = NowPlaying {
            local: false,
            device_name: None,
            uri: "spotify:track:trk0".into(),
            id: None,
            title: "Rosewood".into(),
            artists: Vec::new(),
            subtitle: String::new(),
            album_name: String::new(),
            album_id: None,
            show_id: None,
            art_url: None,
            art_small: None,
            duration_ms: 180_000,
            position_ms: 85_000,
            playing: true,
            loading: false,
            shuffle: false,
            repeat: crate::player::RepeatMode::Off,
            volume_percent: 42,
            can_control: true,
            is_episode: false,
            resuming: false,
        };
        let volume = Value::WmpProp("player.settings.volume".into());
        assert_eq!(bound_value(&volume, Some(&media)), Some(42.0));
        let position = Value::WmpProp("player.Controls.currentPosition".into());
        assert_eq!(bound_value(&position, Some(&media)), Some(85.0));
        let duration = Value::WmpProp("player.currentmedia.duration".into());
        assert_eq!(bound_value(&duration, Some(&media)), Some(180.0));
        // A written number is its own answer.
        let literal = Value::Literal("7".into());
        assert_eq!(bound_value(&literal, None), Some(7.0));
        // And a binding with no player behind it reads as nothing.
        assert_eq!(bound_value(&volume, None), None);

        let name = Value::WmpProp("player.currentmedia.name".into());
        assert_eq!(bound_text(&name, Some(&media)).as_deref(), Some("Rosewood"));
        let position_string = Value::WmpProp("player.controls.currentpositionstring".into());
        assert_eq!(
            bound_text(&position_string, Some(&media)).as_deref(),
            Some("1:25")
        );
        let duration_string = Value::WmpProp("player.currentmedia.durationstring".into());
        assert_eq!(
            bound_text(&duration_string, Some(&media)).as_deref(),
            Some("3:00")
        );
        let literal = Value::Literal("Treble".into());
        assert_eq!(bound_text(&literal, None).as_deref(), Some("Treble"));
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
