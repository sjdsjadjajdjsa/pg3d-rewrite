//! A small re-implementation of NGUI, the Unity UI plugin the original game uses for its
//! menus. It draws a screen exported by ../pixel-gun-3d-re/scripts/export_ngui.py
//! (<data>/ui/<screen>/ui.json + atlases + fonts) with Bevy UI nodes.
//!
//! How NGUI places things (and what we copy):
//!   * The UIRoot makes the screen `manualHeight` (768) virtual pixels tall; the width
//!     follows the window's aspect ratio. Origin in the centre, y up.
//!   * Every object has a local position/scale relative to its parent.
//!   * A widget (sprite / label / texture) is a rectangle of width x height around its
//!     pivot point (one of 9: corners, edge middles, centre).
//!   * Anchors pin a widget's edges to another widget's edges or to the screen edges
//!     ("relative" 0..1 along the target + "absolute" pixel offset). This is what keeps
//!     buttons in the corners on any screen shape.
//!   * Draw order: panel depth first, then widget depth.

use std::collections::HashMap;

use bevy::{
    prelude::*,
    sprite::{BorderRect, SliceScaleMode, TextureSlicer},
    ui::widget::NodeImageMode,
    window::WindowResized,
};
use serde::Deserialize;

use crate::DataDir;

pub struct NguiPlugin;

impl Plugin for NguiPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NguiClick>()
            .add_message::<NguiSlider>()
            .add_systems(Update, (gamma_blend_fix, layout, (hover, clicks, cover_crop, drag_sliders)).chain());
    }
}

// ------------------------------------------------------------------ data

#[derive(Deserialize, Clone, Default)]
#[allow(dead_code)] // mirrors the exported JSON; not every field is used yet
pub struct NodeDef {
    pub name: String,
    pub parent: Option<usize>,
    pub active: bool,
    pub pos: [f32; 3],
    pub scale: [f32; 2],
    pub kind: Option<String>,
    pub widget: Option<WidgetDef>,
    pub sprite: Option<SpriteDef>,
    pub label: Option<LabelDef>,
    pub texture: Option<TextureDef>,
    pub panel: Option<PanelDef>,
    pub button: Option<ButtonDef>,
    pub root: Option<RootDef>,
    pub slider: Option<SliderDef>,
}

/// NGUI UISlider: the thumb moves along the slider's width with the value (0..1).
#[derive(Deserialize, Clone, Default)]
pub struct SliderDef {
    pub value: f32,
    pub thumb: Option<usize>,
}

#[derive(Deserialize, Clone, Default)]
pub struct WidgetDef {
    pub w: f32,
    pub h: f32,
    pub depth: i32,
    pub pivot: usize,
    pub color: [f32; 4],
    pub enabled: bool,
    #[serde(default)]
    pub anchors: HashMap<String, AnchorDef>,
}

#[derive(Deserialize, Clone, Copy, Default)]
pub struct AnchorDef {
    pub target: Option<usize>,
    pub rel: f32,
    pub abs: f32,
}

#[derive(Deserialize, Clone, Default)]
#[allow(dead_code)]
pub struct SpriteDef {
    pub atlas: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: i32,
    pub flip: i32,
    pub fill_center: bool,
    pub fill_amount: f32,
    pub fill_dir: i32,
}

#[derive(Deserialize, Clone, Default)]
pub struct LabelDef {
    pub text: String,
    pub font: Option<String>,
    pub size: f32,
    pub align: i32,
    pub overflow: i32,
    pub effect: i32,
    pub effect_color: [f32; 4],
    pub effect_dist: [f32; 2],
    pub spacing: [f32; 2],
    /// translation key (NGUI/I2 "term"); the text field is then just a placeholder
    #[serde(default)]
    pub key: String,
    /// NGUI "Gradient": each glyph's colour fades from `gradient_top` to `gradient_bottom`
    #[serde(default)]
    pub gradient: bool,
    #[serde(default = "white")]
    pub gradient_top: [f32; 4],
    #[serde(default = "white")]
    pub gradient_bottom: [f32; 4],
}

fn white() -> [f32; 4] {
    [1.0; 4]
}

#[derive(Deserialize, Clone, Default)]
pub struct TextureDef {
    pub png: Option<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct PanelDef {
    pub alpha: f32,
    pub depth: i32,
    /// 0 = none; otherwise the panel shows only the `clip` rectangle (x, y, width, height)
    #[serde(default)]
    pub clipping: i32,
    #[serde(default)]
    pub clip: [f32; 4],
}

#[derive(Deserialize, Clone, Default)]
#[allow(dead_code)]
pub struct ButtonDef {
    #[serde(default)]
    pub on_click: Vec<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct RootDef {
    pub manual_height: f32,
}

#[derive(Deserialize, Clone)]
struct AtlasDef {
    png: String,
    /// name -> [x, y, w, h, border l r t b, padding l r t b, centre transparent (0/1)]
    sprites: HashMap<String, Vec<f32>>,
}

#[derive(Deserialize, Clone)]
struct FontDef {
    png: String,
    size: f32,
    /// unicode code point -> [x, y, w, h, offset x, offset y, advance]
    glyphs: HashMap<String, [f32; 7]>,
}

// ------------------------------------------------------------------ screen

/// One loaded NGUI screen. Spawn with [`spawn_screen`].
#[derive(Component)]
pub struct NguiScreen {
    pub nodes: Vec<NodeDef>,
    atlases: HashMap<String, (Handle<Image>, AtlasDef)>,
    fonts: HashMap<String, (Handle<Image>, FontDef)>,
    textures: HashMap<String, Handle<Image>>,
    strings: HashMap<String, String>,
    /// UI entity drawing each node (widgets only)
    entities: Vec<Option<Entity>>,
    /// clickable area for each button node
    hits: Vec<Option<Entity>>,
    /// overrides set by game code: node index -> active
    pub active_override: HashMap<usize, bool>,
    /// node index -> replacement text
    pub text_override: HashMap<usize, String>,
    /// node index -> replacement picture for a texture widget (cropped to fill it)
    pub texture_override: HashMap<usize, Handle<Image>>,
    /// node index -> replacement widget size (virtual pixels)
    pub size_override: HashMap<usize, Vec2>,
    /// node index -> replacement local position (virtual pixels)
    pub pos_override: HashMap<usize, Vec2>,
    /// slider node -> current value 0..1
    pub slider_values: HashMap<usize, f32>,
    dirty: bool,
    last_size: Vec2,
    /// label glyph cache key (node -> (text, size)) to avoid rebuilding glyphs
    glyph_key: HashMap<usize, (String, i32, i32)>,
}

impl NguiScreen {
    /// Index of the first node with this name.
    #[allow(dead_code)]
    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }
    /// Index of a node by slash path suffix, e.g. "RightAnchor/BtnBattle_A".
    #[allow(dead_code)]
    pub fn find_path(&self, suffix: &str) -> Option<usize> {
        (0..self.nodes.len()).find(|&i| self.path(i).ends_with(suffix))
    }
    pub fn path(&self, mut i: usize) -> String {
        let mut parts = vec![];
        loop {
            parts.push(self.nodes[i].name.as_str());
            match self.nodes[i].parent {
                Some(p) => i = p,
                None => break,
            }
        }
        parts.reverse();
        parts.join("/")
    }
    pub fn set_active(&mut self, i: usize, on: bool) {
        self.active_override.insert(i, on);
        self.dirty = true;
    }
    /// Set a slider's value (0..1) and move its thumb.
    pub fn set_slider(&mut self, i: usize, value: f32) {
        let v = value.clamp(0.0, 1.0);
        self.slider_values.insert(i, v);
        let (Some(sl), Some(w)) = (self.nodes[i].slider.clone(), self.nodes[i].widget.clone()) else { return };
        if let Some(t) = sl.thumb {
            let pv = pivot_offset(w.pivot);
            let y = self.nodes[t].pos[1];
            self.pos_override.insert(t, Vec2::new(-pv.x * w.w + v * w.w, y));
        }
        self.dirty = true;
    }
    pub fn slider(&self, i: usize) -> f32 {
        self.slider_values.get(&i).copied().or(self.nodes[i].slider.as_ref().map(|s| s.value)).unwrap_or(0.0)
    }
    pub fn set_size(&mut self, i: usize, size: Vec2) {
        self.size_override.insert(i, size);
        self.dirty = true;
    }
    /// Show `image` in texture widget `i`, cropped to fill it (like CSS object-fit: cover).
    pub fn set_texture(&mut self, i: usize, image: Handle<Image>) {
        self.texture_override.insert(i, image);
        self.dirty = true;
    }
    pub fn set_pos(&mut self, i: usize, pos: Vec2) {
        self.pos_override.insert(i, pos);
        self.dirty = true;
    }
    /// Make a node clickable (for widgets the game drives through other scripts).
    pub fn make_button(&mut self, i: usize) {
        self.nodes[i].button.get_or_insert_with(ButtonDef::default);
        self.dirty = true;
    }
    /// `i` and every node below it, in list order.
    pub fn subtree(&self, i: usize) -> Vec<usize> {
        (i..self.nodes.len()).filter(|&j| j == i || is_descendant(self, j, i)).collect()
    }
    /// First node named `name` inside the subtree of `i`.
    pub fn find_in(&self, i: usize, name: &str) -> Option<usize> {
        self.subtree(i).into_iter().find(|&j| self.nodes[j].name == name)
    }
    /// Copy the subtree of `i` (same parent) like Unity's Instantiate; returns the copy's
    /// root. Anchors and slider thumbs inside the subtree point at the copies.
    pub fn clone_subtree(&mut self, i: usize) -> usize {
        let src = self.subtree(i);
        let base = self.nodes.len();
        let map: HashMap<usize, usize> = src.iter().enumerate().map(|(k, &j)| (j, base + k)).collect();
        for &j in &src {
            let mut n = self.nodes[j].clone();
            if j != i {
                n.parent = n.parent.map(|p| map.get(&p).copied().unwrap_or(p));
            }
            if let Some(w) = &mut n.widget {
                for a in w.anchors.values_mut() {
                    a.target = a.target.map(|t| map.get(&t).copied().unwrap_or(t));
                }
            }
            if let Some(sl) = &mut n.slider {
                sl.thumb = sl.thumb.map(|t| map.get(&t).copied().unwrap_or(t));
            }
            if let Some(&on) = self.active_override.get(&j) {
                self.active_override.insert(map[&j], on);
            }
            self.nodes.push(n);
            self.entities.push(None);
            self.hits.push(None);
        }
        self.dirty = true;
        base
    }
    pub fn set_text(&mut self, i: usize, text: impl Into<String>) {
        self.text_override.insert(i, text.into());
        self.dirty = true;
    }
    fn active(&self, i: usize) -> bool {
        *self.active_override.get(&i).unwrap_or(&self.nodes[i].active)
    }
    pub fn visible(&self, i: usize) -> bool {
        let mut j = Some(i);
        while let Some(k) = j {
            if !self.active(k) {
                return false;
            }
            j = self.nodes[k].parent;
        }
        true
    }
    pub fn text(&self, i: usize) -> String {
        if let Some(t) = self.text_override.get(&i) {
            return t.clone();
        }
        let Some(l) = &self.nodes[i].label else { return String::new() };
        // labels store translation keys ("Key_5072" -> "To Battle"), either as their
        // term or in place of the text
        self.strings
            .get(&l.key)
            .or_else(|| self.strings.get(&l.text))
            .cloned()
            .unwrap_or_else(|| l.text.clone())
    }
}

/// "The player clicked the button at this node" (index + its name).
#[derive(Message, Clone, Debug)]
#[allow(dead_code)]
pub struct NguiClick {
    pub screen: Entity,
    pub node: usize,
    pub name: String,
}

/// "The player moved this slider" (index, new value 0..1).
#[derive(Message, Clone, Debug)]
pub struct NguiSlider {
    pub screen: Entity,
    pub node: usize,
    pub value: f32,
}

#[derive(Component)]
struct Hit {
    screen: Entity,
    node: usize,
}

#[derive(Component)]
struct Glyphs;

pub fn spawn_screen(commands: &mut Commands, assets: &AssetServer, data: &DataDir, name: &str) -> Option<Entity> {
    let dir = data.0.join("ui").join(name);
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).ok();
    let nodes: Vec<NodeDef> = serde_json::from_str(&read("ui.json")?).ok()?;
    let atlas_defs: HashMap<String, AtlasDef> = serde_json::from_str(&read("atlases.json")?).ok()?;
    let font_defs: HashMap<String, FontDef> = serde_json::from_str(&read("fonts.json")?).ok()?;
    let strings: HashMap<String, String> = std::fs::read_to_string(data.0.join("ui/strings_en-gb.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let base = format!("ui/{name}");
    let atlases = atlas_defs
        .into_iter()
        .map(|(k, a)| (k, (assets.load(format!("{base}/{}", a.png)), a)))
        .collect();
    let fonts = font_defs.into_iter().map(|(k, f)| (k, (assets.load(format!("{base}/{}", f.png)), f))).collect();
    let textures = nodes
        .iter()
        .filter_map(|n| n.texture.as_ref()?.png.clone())
        .map(|p| (p.clone(), assets.load(format!("{base}/{p}"))))
        .collect();
    let n = nodes.len();
    // things the game switches off right after loading
    let active_override: HashMap<usize, bool> =
        nodes.iter().enumerate().filter(|(_, d)| d.name == "InputBlocker").map(|(i, _)| (i, false)).collect();
    let screen = NguiScreen {
        nodes,
        atlases,
        fonts,
        textures,
        strings,
        entities: vec![None; n],
        hits: vec![None; n],
        active_override,
        text_override: HashMap::new(),
        texture_override: HashMap::new(),
        size_override: HashMap::new(),
        pos_override: HashMap::new(),
        slider_values: HashMap::new(),
        dirty: true,
        last_size: Vec2::ZERO,
        glyph_key: HashMap::new(),
    };
    let root = commands
        .spawn((
            Name::new(format!("ngui {name}")),
            screen,
            Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), ..default() },
            Pickable::IGNORE,
            // Unity draws NGUI quads where the maths puts them (no snapping to whole screen
            // pixels); Bevy would round every glyph on its own and shift some by a pixel
            LayoutConfig { use_rounding: false },
        ))
        .id();
    info!("ngui screen {name}: {n} nodes");
    Some(root)
}

// ------------------------------------------------------------------ layout

fn pivot_offset(p: usize) -> Vec2 {
    // NGUI pivots: 0 TopLeft 1 Top 2 TopRight 3 Left 4 Center 5 Right 6 BottomLeft 7 Bottom 8 BottomRight
    let x = [0.0, 0.5, 1.0][p % 3];
    let y = [1.0, 0.5, 0.0][(p / 3).min(2)];
    Vec2::new(x, y)
}

#[derive(Clone, Copy, Default)]
struct Placed {
    pos: Vec2,
    scale: Vec2,
    /// widget rectangle in virtual screen units (y up), if this node is a widget
    rect: Option<Rect>,
}

fn compute(screen: &NguiScreen, vw: f32, vh: f32) -> Vec<Placed> {
    let n = screen.nodes.len();
    let full = Rect::new(-vw / 2.0, -vh / 2.0, vw / 2.0, vh / 2.0);
    let mut out = vec![Placed::default(); n];
    // two passes so anchors to nodes later in the list resolve too
    for _ in 0..2 {
        for i in 0..n {
            let nd = &screen.nodes[i];
            let (ppos, pscale) = match nd.parent {
                Some(p) => (out[p].pos, out[p].scale),
                None => (Vec2::ZERO, Vec2::ONE),
            };
            let local = screen.pos_override.get(&i).copied().unwrap_or(Vec2::new(nd.pos[0], nd.pos[1]));
            let lscale = Vec2::new(nd.scale[0], nd.scale[1]);
            // the root's (and a nested UIRoot's) transform is set by NGUI at run time to fit the camera
            let (mut pos, scale) =
                if nd.parent.is_none() || nd.root.is_some() { (Vec2::ZERO, Vec2::ONE) } else { (ppos + pscale * local, pscale * lscale) };
            let mut rect = None;
            if let Some(w) = &nd.widget {
                let base_size = screen.size_override.get(&i).copied().unwrap_or(Vec2::new(w.w, w.h));
                let size = base_size * scale;
                let pv = pivot_offset(w.pivot);
                let mut r = Rect::from_corners(pos - pv * size, pos - pv * size + size);
                let side = |name: &str| w.anchors.get(name).copied();
                let target_rect = |a: AnchorDef| -> Rect {
                    match a.target {
                        Some(t) => match out[t].rect {
                            Some(tr) => tr,
                            // a clipping panel's sides are its clip rectangle
                            None if screen.nodes[t].panel.as_ref().is_some_and(|p| p.clipping != 0) => {
                                let c = screen.nodes[t].panel.as_ref().unwrap().clip;
                                let centre = out[t].pos + Vec2::new(c[0], c[1]) * out[t].scale;
                                let half = Vec2::new(c[2], c[3]) * out[t].scale / 2.0;
                                Rect::from_corners(centre - half, centre + half)
                            }
                            None if screen.nodes[t].panel.is_some() || screen.nodes[t].root.is_some() => full,
                            None => Rect::from_corners(out[t].pos, out[t].pos),
                        },
                        None => full,
                    }
                };
                if let Some(a) = side("left") {
                    let t = target_rect(a);
                    r.min.x = t.min.x + (t.max.x - t.min.x) * a.rel + a.abs * pscale.x;
                }
                if let Some(a) = side("right") {
                    let t = target_rect(a);
                    r.max.x = t.min.x + (t.max.x - t.min.x) * a.rel + a.abs * pscale.x;
                }
                if let Some(a) = side("bottom") {
                    let t = target_rect(a);
                    r.min.y = t.min.y + (t.max.y - t.min.y) * a.rel + a.abs * pscale.y;
                }
                if let Some(a) = side("top") {
                    let t = target_rect(a);
                    r.max.y = t.min.y + (t.max.y - t.min.y) * a.rel + a.abs * pscale.y;
                }
                // a side anchored without its opposite side keeps the widget's size (it moves)
                let (has_l, has_r, has_b, has_t) = (side("left").is_some(), side("right").is_some(), side("bottom").is_some(), side("top").is_some());
                if has_l && !has_r {
                    r.max.x = r.min.x + size.x;
                } else if has_r && !has_l {
                    r.min.x = r.max.x - size.x;
                }
                if has_b && !has_t {
                    r.max.y = r.min.y + size.y;
                } else if has_t && !has_b {
                    r.min.y = r.max.y - size.y;
                }
                if !w.anchors.is_empty() {
                    // anchored: the widget moves so its pivot sits at the same spot of the new rect
                    pos = r.min + pv * r.size();
                }
                rect = Some(r);
            }
            out[i] = Placed { pos, scale, rect };
        }
    }
    out
}

/// Nearest panel depth above (or on) a node: NGUI draws panel by panel.
fn panel_of(screen: &NguiScreen, mut i: usize) -> (i32, f32) {
    let mut alpha = 1.0;
    let mut depth = None;
    loop {
        if let Some(p) = &screen.nodes[i].panel {
            alpha *= p.alpha;
            depth.get_or_insert(p.depth);
        }
        match screen.nodes[i].parent {
            Some(p) => i = p,
            None => break,
        }
    }
    (depth.unwrap_or(0), alpha)
}

/// Strip NGUI markup ([ff0000] colour, [-] end colour, [b], [i], ...) into coloured runs.
fn parse_markup(text: &str, base: Color) -> Vec<(char, Color)> {
    let mut out = vec![];
    let mut stack = vec![base];
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(end) = chars[i..].iter().position(|c| *c == ']') {
                let tag: String = chars[i + 1..i + end].iter().collect();
                let is_hex = tag.len() == 6 && tag.chars().all(|c| c.is_ascii_hexdigit());
                if is_hex {
                    let v = u32::from_str_radix(&tag, 16).unwrap_or(0xffffff);
                    let c = Color::srgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8);
                    stack.push(c);
                    i += end + 1;
                    continue;
                }
                if tag == "-" {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                    i += end + 1;
                    continue;
                }
                let t = tag.trim_start_matches('/');
                if ["b", "i", "u", "s", "sub", "sup", "c"].contains(&t) || t.starts_with("url") {
                    i += end + 1;
                    continue;
                }
            }
        }
        out.push((chars[i], *stack.last().unwrap()));
        i += 1;
    }
    out
}

fn srgba(c: [f32; 4]) -> Color {
    Color::srgba(c[0], c[1], c[2], c[3])
}

fn tint(c: Color, alpha: f32) -> Color {
    let mut c = c;
    c.set_alpha(c.alpha() * alpha);
    c
}

#[allow(clippy::too_many_arguments)]
fn layout(
    mut commands: Commands,
    mut screens: Query<(Entity, &mut NguiScreen)>,
    window: Single<&Window>,
    mut resized: MessageReader<WindowResized>,
    mut nodes_q: Query<(&mut Node, &mut Visibility, &mut GlobalZIndex), Without<NguiScreen>>,
) {
    let any_resize = resized.read().count() > 0;
    let wsize = Vec2::new(window.width(), window.height());
    for (screen_e, mut screen) in &mut screens {
        if !screen.dirty && !any_resize && screen.last_size == wsize {
            continue;
        }
        let screen = &mut *screen;
        screen.dirty = false;
        screen.last_size = wsize;
        let vh = screen.nodes.iter().find_map(|n| n.root.as_ref().map(|r| r.manual_height)).unwrap_or(768.0);
        let k = wsize.y / vh;
        let vw = wsize.x / k;
        let placed = compute(screen, vw, vh);
        let mut spawned_any = false;
        let to_screen = |r: Rect| -> Rect {
            Rect::new((r.min.x + vw / 2.0) * k, (vh / 2.0 - r.max.y) * k, (r.max.x + vw / 2.0) * k, (vh / 2.0 - r.min.y) * k)
        };
        for i in 0..screen.nodes.len() {
            let nd = screen.nodes[i].clone();
            let Some(w) = &nd.widget else { continue };
            let Some(vr) = placed[i].rect else { continue };
            let visible = screen.visible(i) && w.enabled && w.color[3] > 0.0;
            let sr = to_screen(vr);
            let (pdepth, palpha) = panel_of(screen, i);
            let z = pdepth * 1000 + w.depth;
            // create the entity the first time the node becomes visible
            if screen.entities[i].is_none() {
                if !visible {
                    continue;
                }
                let Some((bundle, image)) = make_widget(screen, i, palpha, k, sr) else { continue };
                // new entities only exist once commands are applied: give them their
                // draw order now, and lay out once more next frame
                let e = commands.spawn((bundle, GlobalZIndex(z))).id();
                spawned_any = true;
                if let Some(image) = image {
                    commands.entity(e).insert(image);
                }
                commands.entity(screen_e).add_child(e);
                screen.entities[i] = Some(e);
            }
            let e = screen.entities[i].unwrap();
            if let Ok((mut node, mut vis, mut gz)) = nodes_q.get_mut(e) {
                let dr = sprite_draw_rect(screen, &nd, sr);
                node.left = px(dr.min.x);
                node.top = px(dr.min.y);
                node.width = px(dr.width().max(0.0));
                node.height = px(dr.height().max(0.0));
                *vis = if visible { Visibility::Inherited } else { Visibility::Hidden };
                *gz = GlobalZIndex(z);
            }
            if nd.label.is_some() && visible {
                build_glyphs(&mut commands, screen, i, e, sr, k, palpha);
            }
            if visible {
                build_frame(&mut commands, screen, i, e, sr, k, palpha);
            }
            // a picture set by game code after the widget was first drawn
            if let Some(h) = screen.texture_override.get(&i) {
                let mut im = ImageNode::new(h.clone()).with_color(tint(srgba(w.color), palpha));
                im.image_mode = NodeImageMode::Stretch;
                let h = h.clone();
                commands.entity(e).entry::<ImageNode>().and_modify(move |mut img| {
                    if img.image != h {
                        img.image = h.clone();
                        img.rect = None;
                    }
                }).or_insert(im);
            }
            if let Some(ImageNodeUpdate { slicer }) = slice_update(screen, &nd, k) {
                commands.entity(e).entry::<ImageNode>().and_modify(move |mut img| {
                    img.image_mode = NodeImageMode::Sliced(slicer);
                });
            }
        }
        if std::env::var("PG3D_TRACE").is_ok() {
            let made = screen.entities.iter().filter(|e| e.is_some()).count();
            info!("ngui layout {vw:.0}x{vh:.0} k={k:.2}: {made} widget entities");
            for i in 0..screen.nodes.len() {
                let watch = std::env::var("PG3D_TRACE_NGUI").unwrap_or_default();
                if screen.entities[i].is_some() && !watch.is_empty() && screen.path(i).contains(&watch) && screen.visible(i) {
                    let (pd, _) = panel_of(screen, i);
                    let d = screen.nodes[i].widget.as_ref().map_or(0, |w| w.depth);
                    info!("  {} z={} rect {:?}", screen.path(i), pd * 1000 + d, placed[i].rect.map(to_screen));
                }
            }
        }
        if spawned_any {
            screen.dirty = true;
        }
        // clickable areas: the button's own widget, or the union of its visible children
        for i in 0..screen.nodes.len() {
            if screen.nodes[i].button.is_none() && screen.nodes[i].slider.is_none() {
                continue;
            }
            // a slider's thumb is dragged through the slider itself
            if screen.nodes.iter().any(|n| n.slider.as_ref().is_some_and(|s| s.thumb == Some(i))) {
                continue;
            }
            let visible = screen.visible(i);
            let mut area: Option<Rect> = placed[i].rect;
            if area.is_none() {
                for j in i + 1..screen.nodes.len() {
                    if !is_descendant(screen, j, i) {
                        continue;
                    }
                    if let (Some(r), true) = (placed[j].rect, screen.visible(j)) {
                        area = Some(area.map_or(r, |a| a.union(r)));
                    }
                }
            }
            let Some(area) = area else { continue };
            let sr = to_screen(area);
            let z = panel_of(screen, i).0 * 1000 + 900;
            let e = *screen.hits[i].get_or_insert_with(|| {
                let e = commands
                    .spawn((
                        Hit { screen: screen_e, node: i },
                        Button,
                        Node::default(),
                        BackgroundColor(Color::NONE),
                        bevy::ui::RelativeCursorPosition::default(),
                    ))
                    .id();
                commands.entity(screen_e).add_child(e);
                e
            });
            commands.entity(e).insert((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(sr.min.x),
                    top: px(sr.min.y),
                    width: px(sr.width()),
                    height: px(sr.height()),
                    ..default()
                },
                GlobalZIndex(z),
                if visible { Visibility::Inherited } else { Visibility::Hidden },
            ));
        }
    }
}

fn is_descendant(screen: &NguiScreen, mut j: usize, ancestor: usize) -> bool {
    while let Some(p) = screen.nodes[j].parent {
        if p == ancestor {
            return true;
        }
        j = p;
    }
    false
}

/// A 9-slice sprite whose middle is not drawn ("Fill Center" off in NGUI): a frame.
fn is_hollow(s: &SpriteDef, r: &[f32]) -> bool {
    let clear_centre = r.get(12).is_some_and(|v| *v > 0.5);
    s.kind == 1 && (!s.fill_center || clear_centre) && r[4] + r[5] + r[6] + r[7] > 0.0
}

/// Draw a hollow 9-slice sprite as its 8 border pieces (Bevy's slicer always fills the centre).
fn build_frame(commands: &mut Commands, screen: &mut NguiScreen, i: usize, e: Entity, sr: Rect, k: f32, alpha: f32) {
    let nd = &screen.nodes[i];
    let (Some(s), Some(w)) = (&nd.sprite, &nd.widget) else { return };
    let Some((img, a)) = s.atlas.as_ref().and_then(|a| screen.atlases.get(a)) else { return };
    let Some(r) = a.sprites.get(&s.name).cloned() else { return };
    if !is_hollow(s, &r) {
        return;
    }
    let key = (format!("frame:{}", s.name), sr.width() as i32, sr.height() as i32);
    if screen.glyph_key.get(&i) == Some(&key) {
        return;
    }
    screen.glyph_key.insert(i, key);
    commands.entity(e).despawn_related::<Children>();
    let color = tint(srgba(w.color), alpha);
    let (x, y, sw, sh) = (r[0], r[1], r[2], r[3]);
    let (bl, br, bt, bb) = (r[4], r[5], r[6], r[7]);
    let (tw, th) = (sr.width(), sr.height());
    // source columns/rows in the atlas and target columns/rows on screen
    let sx = [x, x + bl, x + sw - br, x + sw];
    let sy = [y, y + bt, y + sh - bb, y + sh];
    let tx = [0.0, bl * k, (tw - br * k).max(bl * k), tw];
    let ty = [0.0, bt * k, (th - bb * k).max(bt * k), th];
    let mut kids = vec![];
    for row in 0..3 {
        for col in 0..3 {
            if row == 1 && col == 1 {
                continue;
            }
            let (w_, h_) = (tx[col + 1] - tx[col], ty[row + 1] - ty[row]);
            if w_ <= 0.0 || h_ <= 0.0 || sx[col + 1] <= sx[col] || sy[row + 1] <= sy[row] {
                continue;
            }
            let mut im = ImageNode::new(img.clone()).with_color(color);
            im.rect = Some(Rect::new(sx[col], sy[row], sx[col + 1], sy[row + 1]));
            im.image_mode = NodeImageMode::Stretch;
            let node = Node {
                position_type: PositionType::Absolute,
                left: px(tx[col]),
                top: px(ty[row]),
                width: px(w_),
                height: px(h_),
                ..default()
            };
            kids.push(commands.spawn((node, im, Pickable::IGNORE, Glyphs)).id());
        }
    }
    commands.entity(e).add_children(&kids);
}

struct ImageNodeUpdate {
    slicer: TextureSlicer,
}

/// Sliced sprites keep their corners at 1 virtual pixel = `k` screen pixels.
fn slice_update(screen: &NguiScreen, nd: &NodeDef, k: f32) -> Option<ImageNodeUpdate> {
    let s = nd.sprite.as_ref()?;
    if s.kind != 1 {
        return None;
    }
    let (_, a) = screen.atlases.get(s.atlas.as_ref()?)?;
    let r = a.sprites.get(&s.name)?;
    if is_hollow(s, r) {
        return None;
    }
    if r[4] + r[5] + r[6] + r[7] <= 0.0 {
        return None;
    }
    Some(ImageNodeUpdate {
        slicer: TextureSlicer {
            border: BorderRect { min_inset: Vec2::new(r[4], r[6]), max_inset: Vec2::new(r[5], r[7]) },
            center_scale_mode: SliceScaleMode::Stretch,
            sides_scale_mode: SliceScaleMode::Stretch,
            max_corner_scale: k,
        },
    })
}

fn abs_node(r: Rect) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: px(r.min.x),
        top: px(r.min.y),
        width: px(r.width()),
        height: px(r.height()),
        ..default()
    }
}

/// Where a Simple or Filled sprite is actually drawn inside its widget rectangle (screen
/// pixels): NGUI's UIBasicSprite.drawingDimensions insets the quad by the sprite's
/// padding, and gives a sprite with an odd width / height one extra pixel of padding on
/// the right / top, measured in sprite pixels stretched to the widget size. E.g. the
/// Arena box's 19x29 zombie icon in an 18x27 widget is drawn ~17x26 and nudged left/down.
fn sprite_draw_rect(screen: &NguiScreen, nd: &NodeDef, sr: Rect) -> Rect {
    let Some(s) = &nd.sprite else { return sr };
    if s.kind != 0 && s.kind != 3 {
        return sr;
    }
    let Some(r) = s.atlas.as_ref().and_then(|a| screen.atlases.get(a)).and_then(|(_, a)| a.sprites.get(&s.name)) else {
        return sr;
    };
    let pad = |j: usize| r.get(j).copied().unwrap_or(0.0);
    let (mut l, mut rt, mut t, b) = (pad(8), pad(9), pad(10), pad(11));
    let (w, h) = (r[2] + l + rt, r[3] + t + b);
    if w <= 0.0 || h <= 0.0 {
        return sr;
    }
    if (w as i32) & 1 != 0 {
        rt += 1.0;
    }
    if (h as i32) & 1 != 0 {
        t += 1.0;
    }
    let (px_, py_) = (sr.width() / w, sr.height() / h);
    let (fx, fy) = (s.flip == 1 || s.flip == 3, s.flip == 2 || s.flip == 3);
    if fx {
        std::mem::swap(&mut l, &mut rt);
    }
    let (t, b) = if fy { (b, t) } else { (t, b) };
    Rect::new(sr.min.x + l * px_, sr.min.y + t * py_, sr.max.x - rt * px_, sr.max.y - b * py_)
}

/// Bevy components for one widget (sprite / texture / label container).
fn make_widget(screen: &NguiScreen, i: usize, alpha: f32, k: f32, sr: Rect) -> Option<(impl Bundle, Option<ImageNode>)> {
    let nd = &screen.nodes[i];
    let w = nd.widget.as_ref()?;
    let color = tint(srgba(w.color), alpha);
    let base = (Name::new(nd.name.clone()), abs_node(sr), Visibility::Inherited, Pickable::IGNORE);
    if let Some(s) = &nd.sprite {
        let (img, a) = screen.atlases.get(s.atlas.as_ref()?)?;
        let r = a.sprites.get(&s.name)?;
        if is_hollow(s, r) {
            return Some((base, None)); // pieces added by build_frame
        }
        let mut image = ImageNode::new(img.clone()).with_color(color);
        image.rect = Some(Rect::new(r[0], r[1], r[0] + r[2], r[1] + r[3]));
        image.flip_x = s.flip == 1 || s.flip == 3;
        image.flip_y = s.flip == 2 || s.flip == 3;
        image.image_mode = match s.kind {
            1 if r[4] + r[5] + r[6] + r[7] > 0.0 => NodeImageMode::Sliced(TextureSlicer {
                border: BorderRect { min_inset: Vec2::new(r[4], r[6]), max_inset: Vec2::new(r[5], r[7]) },
                center_scale_mode: SliceScaleMode::Stretch,
                sides_scale_mode: SliceScaleMode::Stretch,
                max_corner_scale: k,
            }),
            2 => NodeImageMode::Tiled { tile_x: true, tile_y: true, stretch_value: k },
            _ => NodeImageMode::Stretch,
        };
        if s.kind == 3 && s.fill_amount < 0.999 {
            // filled sprite (progress bars): crop horizontally / vertically
            let f = s.fill_amount.max(0.0);
            let mut rc = image.rect.unwrap();
            if s.fill_dir == 0 {
                rc.max.x = rc.min.x + rc.width() * f;
            } else if s.fill_dir == 1 {
                rc.min.y = rc.max.y - rc.height() * f;
            }
            image.rect = Some(rc);
        }
        return Some((base, Some(image)));
    }
    if let Some(h) = screen.texture_override.get(&i) {
        // cover_crop fits the picture once it has loaded
        let mut im = ImageNode::new(h.clone()).with_color(color);
        im.image_mode = NodeImageMode::Stretch;
        return Some((base, Some(im)));
    }
    if let Some(t) = &nd.texture {
        let h = screen.textures.get(t.png.as_ref()?)?;
        let mut im = ImageNode::new(h.clone()).with_color(color);
        im.image_mode = NodeImageMode::Stretch;
        return Some((base, Some(im)));
    }
    // labels and plain widgets: an empty container
    Some((base, None))
}

/// Lay out a bitmap-font label as one image node per glyph (plus outline/shadow copies).
fn build_glyphs(commands: &mut Commands, screen: &mut NguiScreen, i: usize, e: Entity, sr: Rect, k: f32, alpha: f32) {
    let nd = &screen.nodes[i];
    let (Some(l), Some(w)) = (&nd.label, &nd.widget) else { return };
    let text = screen.text(i);
    let key = (text.clone(), sr.width() as i32, sr.height() as i32);
    if screen.glyph_key.get(&i) == Some(&key) {
        return;
    }
    screen.glyph_key.insert(i, key);
    commands.entity(e).despawn_related::<Children>();
    // labels with a TrueType font (not exported) use the screen's bitmap font instead
    let Some((img, font)) = l.font.as_ref().and_then(|f| screen.fonts.get(f)).or_else(|| screen.fonts.values().next())
    else {
        return;
    };
    let runs = parse_markup(&text, tint(srgba(w.color), alpha));
    // lines of (glyph rect in font page, x, advance-adjusted) in virtual units at scale 1
    let mut lines: Vec<Vec<(char, Color, [f32; 7], f32)>> = vec![vec![]];
    let mut x = 0.0;
    let mut widths = vec![0.0f32];
    for (ch, col) in runs {
        if ch == '\n' {
            lines.push(vec![]);
            widths.push(0.0);
            x = 0.0;
            continue;
        }
        let g = font.glyphs.get(&(ch as u32).to_string()).copied();
        let g = g.or_else(|| font.glyphs.get(&(ch.to_ascii_uppercase() as u32).to_string()).copied());
        let Some(g) = g else { continue };
        lines.last_mut().unwrap().push((ch, col, g, x));
        x += g[6] + l.spacing[0] * font.size / l.size.max(1.0);
        *widths.last_mut().unwrap() = x;
    }
    let mut s = l.size / font.size; // virtual units per font pixel
    let line_h = font.size + l.spacing[1] * font.size / l.size.max(1.0);
    let box_w = sr.width() / k;
    let box_h = sr.height() / k;
    let max_w = widths.iter().cloned().fold(0.0, f32::max) * s;
    let total_h = line_h * lines.len() as f32 * s;
    if l.overflow == 0 && (max_w > box_w || total_h > box_h) && max_w > 0.0 {
        s *= (box_w / max_w).min(box_h / total_h).min(1.0);
    }
    let pv = pivot_offset(w.pivot);
    // NGUI "Automatic" alignment follows the pivot
    let align = match l.align {
        1 => 0.0,
        2 => 0.5,
        3 => 1.0,
        _ => pv.x,
    };
    let total_h = line_h * lines.len() as f32 * s;
    // top / middle / bottom; NGUI rounds the label's text offset (UILabel.ApplyOffset)
    let y0 = ((1.0 - pv.y) * (box_h - total_h)).round();
    let effect_offsets: Vec<Vec2> = {
        let d = Vec2::new(l.effect_dist[0], l.effect_dist[1]);
        match l.effect {
            1 => vec![Vec2::new(d.x, d.y)],
            2 => vec![Vec2::new(d.x, d.y), Vec2::new(-d.x, d.y), Vec2::new(d.x, -d.y), Vec2::new(-d.x, -d.y)],
            3 => (0..8)
                .map(|a| {
                    let t = a as f32 * std::f32::consts::FRAC_PI_4;
                    Vec2::new(t.cos(), t.sin()) * d
                })
                .collect(),
            _ => vec![],
        }
    };
    let effect_color = tint(srgba(l.effect_color), alpha);
    let mut kids = vec![];
    for (li, line) in lines.iter().enumerate() {
        // NGUIText.Align: right = rect - printed; centre = half of that, plus half a font
        // pixel when the rounded leftover and the rounded width differ in parity ("keep it
        // pixel-perfect"); UILabel.ApplyOffset then rounds the pivot offset
        let printed = widths[li] * s;
        let mut lx = align * (box_w - printed);
        if align == 0.5 && box_w > printed {
            let diff = (box_w - printed).round() as i64;
            let int_w = box_w.round() as i64;
            if (diff & 1 == 1) != (int_w & 1 == 1) {
                lx += 0.5 * s;
            }
        }
        lx += pv.x * box_w - (pv.x * box_w).round();
        for (_, col, g, gx) in line {
            if g[2] <= 0.0 || g[3] <= 0.0 {
                continue;
            }
            let x = lx + (gx + g[4]) * s;
            let y = y0 + (li as f32 * line_h + g[5]) * s;
            let rect = Rect::new(g[0], g[1], g[0] + g[2], g[1] + g[3]);
            let place = |off: Vec2| Node {
                position_type: PositionType::Absolute,
                left: px((x + off.x) * k),
                top: px((y + off.y) * k),
                width: px(g[2] * s * k),
                height: px(g[3] * s * k),
                ..default()
            };
            for off in &effect_offsets {
                let mut im = ImageNode::new(img.clone()).with_color(effect_color);
                im.rect = Some(rect);
                im.image_mode = NodeImageMode::Stretch;
                kids.push((place(*off), im, Pickable::IGNORE, ZIndex(0)));
            }
            if !l.gradient {
                let mut im = ImageNode::new(img.clone()).with_color(*col);
                im.rect = Some(rect);
                im.image_mode = NodeImageMode::Stretch;
                kids.push((place(Vec2::ZERO), im, Pickable::IGNORE, ZIndex(1)));
                continue;
            }
            // vertical gradient: draw the glyph as horizontal strips, each tinted by the
            // gradient at its middle (NGUI interpolates between the quad's top and bottom)
            const STRIPS: usize = 4;
            let c = col.to_srgba();
            for j in 0..STRIPS {
                let (a, b) = (j as f32 / STRIPS as f32, (j + 1) as f32 / STRIPS as f32);
                let t = (a + b) / 2.0;
                let gc: Vec<f32> = (0..4).map(|i| l.gradient_top[i] + (l.gradient_bottom[i] - l.gradient_top[i]) * t).collect();
                let tint_c = Color::srgba(c.red * gc[0], c.green * gc[1], c.blue * gc[2], c.alpha * gc[3]);
                let mut im = ImageNode::new(img.clone()).with_color(tint_c);
                im.rect = Some(Rect::new(g[0], g[1] + g[3] * a, g[0] + g[2], g[1] + g[3] * b));
                im.image_mode = NodeImageMode::Stretch;
                let mut n = place(Vec2::ZERO);
                n.top = px((y + g[3] * a * s) * k);
                n.height = px(g[3] * (b - a) * s * k);
                kids.push((n, im, Pickable::IGNORE, ZIndex(1)));
            }
        }
    }
    let ids: Vec<Entity> = kids.into_iter().map(|b| commands.spawn((b, Glyphs)).id()).collect();
    commands.entity(e).add_children(&ids);
}

fn hover(mut q: Query<(&Interaction, &mut BackgroundColor), (With<Hit>, Changed<Interaction>)>) {
    for (i, mut bg) in &mut q {
        bg.0 = match i {
            Interaction::Pressed => Color::srgba(0.0, 0.0, 0.0, 0.15),
            Interaction::Hovered => Color::srgba(1.0, 1.0, 1.0, 0.08),
            Interaction::None => Color::NONE,
        };
    }
}

fn clicks(
    q: Query<(&Interaction, &Hit), Changed<Interaction>>,
    screens: Query<&NguiScreen>,
    mut out: MessageWriter<NguiClick>,
) {
    for (i, h) in &q {
        if *i == Interaction::Pressed {
            if let Ok(s) = screens.get(h.screen) {
                let name = s.nodes[h.node].name.clone();
                info!("ngui click: {}", s.path(h.node));
                out.write(NguiClick { screen: h.screen, node: h.node, name });
            }
        }
    }
}

/// Override pictures fill their widget: crop the image to the widget's aspect ratio.
fn cover_crop(
    screens: Query<&NguiScreen>,
    mut q: Query<(&mut ImageNode, &ComputedNode)>,
    images: Res<Assets<Image>>,
) {
    for s in &screens {
        for (&i, handle) in &s.texture_override {
            let Some(e) = s.entities.get(i).copied().flatten() else { continue };
            let Ok((mut img, node)) = q.get_mut(e) else { continue };
            let Some(image) = images.get(handle) else { continue };
            let size = image.size_f32();
            let target = node.size();
            if target.x <= 0.0 || target.y <= 0.0 {
                continue;
            }
            let want = target.x / target.y;
            let have = size.x / size.y;
            let rect = if have > want {
                let w = size.y * want;
                Rect::new((size.x - w) / 2.0, 0.0, (size.x + w) / 2.0, size.y)
            } else {
                let h = size.x / want;
                Rect::new(0.0, (size.y - h) / 2.0, size.x, (size.y + h) / 2.0)
            };
            if img.rect != Some(rect) {
                img.rect = Some(rect);
            }
        }
    }
}

fn drag_sliders(
    q: Query<(&Interaction, &Hit, &bevy::ui::RelativeCursorPosition)>,
    mut screens: Query<&mut NguiScreen>,
    mut out: MessageWriter<NguiSlider>,
) {
    for (i, h, rel) in &q {
        if *i != Interaction::Pressed {
            continue;
        }
        let Ok(mut s) = screens.get_mut(h.screen) else { continue };
        if s.nodes[h.node].slider.is_none() {
            continue;
        }
        let Some(p) = rel.normalized else { continue };
        let v = (p.x + 0.5).clamp(0.0, 1.0);
        if (s.slider(h.node) - v).abs() > 0.001 {
            s.set_slider(h.node, v);
            out.write(NguiSlider { screen: h.screen, node: h.node, value: v });
        }
    }
}

// ------------------------------------------------------------------ gamma-space blending

/// NGUI draws in Unity's gamma colour space, so a half-transparent pixel mixes the *sRGB*
/// values: dark boxes (the Arena score box's 71 % black middle) hide the scene well and
/// white text edges stay thin. Bevy blends in linear light, which lets far more of the
/// scene through dark boxes and fattens bright edges. We can't change the blend, so each
/// UI picture's alpha is adjusted once after loading so that blending in linear light gives
/// the colour gamma blending would give over a typical background (sRGB 0.2). For black
/// pixels that is exactly a' = 1 - (1 - a)^2.2 whatever the background.
fn gamma_blend_fix(
    screens: Query<&NguiScreen>,
    mut images: ResMut<Assets<Image>>,
    mut done: Local<std::collections::HashSet<AssetId<Image>>>,
    mut lut: Local<Vec<u8>>,
) {
    if lut.is_empty() {
        *lut = gamma_alpha_lut();
    }
    for s in &screens {
        let handles = s.atlases.values().map(|a| &a.0).chain(s.fonts.values().map(|f| &f.0)).chain(s.textures.values());
        for h in handles {
            if done.contains(&h.id()) {
                continue;
            }
            let Some(mut img) = images.get_mut(h) else { continue };
            done.insert(h.id());
            use bevy::render::render_resource::TextureFormat as F;
            if !matches!(img.texture_descriptor.format, F::Rgba8UnormSrgb | F::Rgba8Unorm) {
                continue;
            }
            let Some(data) = img.data.as_mut() else { continue };
            for px in data.chunks_exact_mut(4) {
                let a = px[3];
                if a == 0 || a == 255 {
                    continue;
                }
                let l = ((px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8) as usize;
                px[3] = lut[(a as usize) << 8 | l];
            }
        }
    }
}

/// [alpha << 8 | luminance] -> alpha that gives the gamma-space blend in linear light.
fn gamma_alpha_lut() -> Vec<u8> {
    let lin = |v: f32| v.powf(2.2);
    let b = 0.2_f32;
    let mut t = vec![0u8; 256 * 256];
    for a in 0..256 {
        for l in 0..256 {
            let (af, c) = (a as f32 / 255.0, l as f32 / 255.0);
            let d = lin(c) - lin(b);
            let out = if d.abs() < 0.02 { af } else { ((lin(af * c + (1.0 - af) * b) - lin(b)) / d).clamp(0.0, 1.0) };
            t[a << 8 | l] = (out * 255.0 + 0.5) as u8;
        }
    }
    t
}
