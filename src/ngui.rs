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
            .add_systems(Update, (layout, (hover, clicks)).chain());
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
}

#[derive(Deserialize, Clone, Default)]
pub struct TextureDef {
    pub png: Option<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct PanelDef {
    pub alpha: f32,
    pub depth: i32,
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
    /// name -> [x, y, w, h, border l r t b, padding l r t b]
    sprites: HashMap<String, [f32; 12]>,
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
    pub fn set_text(&mut self, i: usize, text: impl Into<String>) {
        self.text_override.insert(i, text.into());
        self.dirty = true;
    }
    fn active(&self, i: usize) -> bool {
        *self.active_override.get(&i).unwrap_or(&self.nodes[i].active)
    }
    fn visible(&self, i: usize) -> bool {
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
        // labels store translation keys ("Key_5072" -> "To Battle")
        self.strings.get(&l.text).cloned().unwrap_or_else(|| l.text.clone())
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
            let local = Vec2::new(nd.pos[0], nd.pos[1]);
            let lscale = Vec2::new(nd.scale[0], nd.scale[1]);
            // the root's own transform is set by NGUI at run time to fit the camera
            let (mut pos, scale) =
                if nd.parent.is_none() { (Vec2::ZERO, Vec2::ONE) } else { (ppos + pscale * local, pscale * lscale) };
            let mut rect = None;
            if let Some(w) = &nd.widget {
                let size = Vec2::new(w.w, w.h) * scale;
                let pv = pivot_offset(w.pivot);
                let mut r = Rect::from_corners(pos - pv * size, pos - pv * size + size);
                let side = |name: &str| w.anchors.get(name).copied();
                let target_rect = |a: AnchorDef| -> Rect {
                    match a.target {
                        Some(t) => match out[t].rect {
                            Some(tr) => tr,
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
                let Some((bundle, image)) = make_widget(screen, &nd, palpha, k, sr) else { continue };
                let e = commands.spawn(bundle).id();
                if let Some(image) = image {
                    commands.entity(e).insert(image);
                }
                commands.entity(screen_e).add_child(e);
                screen.entities[i] = Some(e);
            }
            let e = screen.entities[i].unwrap();
            if let Ok((mut node, mut vis, mut gz)) = nodes_q.get_mut(e) {
                node.left = px(sr.min.x);
                node.top = px(sr.min.y);
                node.width = px(sr.width().max(0.0));
                node.height = px(sr.height().max(0.0));
                *vis = if visible { Visibility::Inherited } else { Visibility::Hidden };
                *gz = GlobalZIndex(z);
            }
            if nd.label.is_some() && visible {
                build_glyphs(&mut commands, screen, i, e, sr, k, palpha);
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
                if screen.entities[i].is_some() && screen.nodes[i].name.contains("Battle") {
                    info!("  {} rect {:?}", screen.path(i), placed[i].rect.map(to_screen));
                }
            }
        }
        // clickable areas: the button's own widget, or the union of its visible children
        for i in 0..screen.nodes.len() {
            if screen.nodes[i].button.is_none() {
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
                    .spawn((Hit { screen: screen_e, node: i }, Button, Node::default(), BackgroundColor(Color::NONE)))
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

/// Bevy components for one widget (sprite / texture / label container).
fn make_widget(screen: &NguiScreen, nd: &NodeDef, alpha: f32, k: f32, sr: Rect) -> Option<(impl Bundle, Option<ImageNode>)> {
    let w = nd.widget.as_ref()?;
    let color = tint(srgba(w.color), alpha);
    let base = (Name::new(nd.name.clone()), abs_node(sr), Visibility::Inherited, GlobalZIndex(0), Pickable::IGNORE);
    if let Some(s) = &nd.sprite {
        let (img, a) = screen.atlases.get(s.atlas.as_ref()?)?;
        let r = a.sprites.get(&s.name)?;
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
    if let Some(t) = &nd.texture {
        let h = screen.textures.get(t.png.as_ref()?)?;
        return Some((base, Some(ImageNode::new(h.clone()).with_color(color))));
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
    let Some((img, font)) = l.font.as_ref().and_then(|f| screen.fonts.get(f)) else { return };
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
    let y0 = (1.0 - pv.y) * (box_h - total_h); // top / middle / bottom
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
        let lx = align * (box_w - widths[li] * s);
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
            let mut im = ImageNode::new(img.clone()).with_color(*col);
            im.rect = Some(rect);
            im.image_mode = NodeImageMode::Stretch;
            kids.push((place(Vec2::ZERO), im, Pickable::IGNORE, ZIndex(1)));
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
