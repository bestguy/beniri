//! BeOS-style title tab drawn above the top-left corner of a tile.
//!
//! The tab lives in a strip reserved at the top of the tile (see `Tile::tab_height`). Its
//! background is drawn with the border shader so it shares the frame's color, gradient and corner
//! radius, and it overlaps the top edge of the focus ring so the two blend into one shape.

use std::cell::RefCell;
use std::cmp::min;

use anyhow::ensure;
use niri_config::{
    Color, CornerRadius, Gradient, GradientInterpolation, GradientRelativeTo, TitleTabStyle,
};
use pango::FontDescription;
use pangocairo::cairo::{self, ImageSurface};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};

use crate::layout::TitleTabHit;
use crate::niri_render_elements;
use crate::render_helpers::border::BorderRenderElement;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::to_physical_precise_round;

/// Largest texture dimension we'll ask the GPU for.
const MAX_TEXTURE_SIZE: i32 = 16383;

/// Towns style defaults: a white tab on a light blue bar, and a white bar on inactive windows.
/// The lines come from the frame, which the config defaults to black for this style.
const DEFAULT_TOWNS_TAB_COLOR: [u8; 3] = [255, 255, 255];
const DEFAULT_BAR_COLOR: [u8; 3] = [140, 189, 239];
const DEFAULT_INACTIVE_BAR_COLOR: [u8; 3] = [255, 255, 255];

/// Mac style defaults: a white bar, and five stripes across it while the window is active.
const DEFAULT_MAC_COLOR: [u8; 3] = [255, 255, 255];
const MAC_STRIPES: i32 = 5;

/// Placement of a tab within a row of tabs (tabbed columns).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabRow {
    /// Tile-relative X where this tab starts (ignored for the first tab, which is flush left).
    pub x: f64,
    pub index: usize,
    pub count: usize,
    /// Index of the tile whose window and frame are shown under the row.
    pub shown: usize,
    /// How far the shown tile's frame extends outside the tile, and its outer top-left radius.
    /// A hidden first tab reaches out to that edge and fills in around the rounded corner.
    pub frame_outset: f64,
    pub frame_radius: f64,
}

#[derive(Debug)]
pub struct TitleTab {
    config: niri_config::TitleTab,

    // Everything below is computed in update_render_elements().
    visible: bool,
    /// Whether the maximize button is shown: on in the config and the window can be maximized.
    show_maximize: bool,
    /// Tab background rectangle, relative to the tile origin.
    rect: Rectangle<f64, Logical>,
    /// Radius of the tab's (and bar's) outer top corners.
    outer_radius: f64,
    background: BorderRenderElement,
    /// Outline drawn in the frame color when a fill color is configured.
    outline: BorderRenderElement,
    /// Concave fillet where the tab's right edge meets the frame's top edge.
    fillet: BorderRenderElement,
    /// Fill-colored piece behind the inside of the bend.
    fillet_fill: BorderRenderElement,
    fillet_loc: Point<f64, Logical>,
    fillet_radius: f64,
    /// Same for the left edge, used when the tab is not the first in a row.
    fillet_l: BorderRenderElement,
    fillet_fill_l: BorderRenderElement,
    fillet_l_loc: Point<f64, Logical>,
    /// Fills the gap between the squared-off frame corner and the rounded window corner.
    corner: BorderRenderElement,
    corner_loc: Point<f64, Logical>,
    has_corner: bool,
    /// Hidden first tab in a row: fills the gap outside the shown frame's rounded top-left
    /// corner, so the tab meets the frame.
    skirt: BorderRenderElement,
    skirt_loc: Point<f64, Logical>,
    has_skirt: bool,
    /// Filled style: whether the tab's right edge bends into the frame (reuses `fillet_fill`).
    has_filled_join: bool,
    has_left_bend: bool,
    /// Whether the tab sits on the frame's top-left corner (squaring it off).
    attached_left: bool,
    /// Horizontal gap to the next tab in a row.
    row_gap: f64,
    /// The outline element is cut short so its right edge stops where the bend begins; this stub
    /// continues the left edge down to the frame.
    stub: BorderRenderElement,
    stub_loc: Point<f64, Logical>,
    outline_width: f64,
    has_fill: bool,
    /// Whether to draw the outline (only when the tile has a frame to join).
    draw_outline: bool,
    /// Towns and Mac styles: the tab's top and sides are stroked in the frame color (reuses
    /// `outline`).
    has_bar_outline: bool,
    solid: SolidColorBuffer,
    use_shader: bool,
    alpha: f32,
    scale: f64,
    text_color: Color,
    /// Top-left of the text, relative to the tile origin.
    text_loc: Point<f64, Logical>,
    /// Close button square, relative to the tile origin.
    close_rect: Rectangle<f64, Logical>,
    close_button: BorderRenderElement,
    /// Maximize button box, relative to the tile origin, and its two overlapping squares.
    max_rect: Rectangle<f64, Logical>,
    max_back: BorderRenderElement,
    max_front: BorderRenderElement,
    max_front_fill: BorderRenderElement,
    max_back_loc: Point<f64, Logical>,
    max_front_loc: Point<f64, Logical>,
    /// Towns style: grid lines inside the close button, relative to the tile origin.
    close_grid: Vec<(Point<f64, Logical>, SolidColorBuffer)>,
    /// Mac style: stripe segments across the active bar, relative to the tile origin.
    stripes: Vec<(Point<f64, Logical>, SolidColorBuffer)>,
    /// Mac style: the boxes only show, and only hit, on the active window.
    mac: bool,
    is_active: bool,
    /// Maximum text width in physical pixels; longer titles get ellipsized.
    text_max_width: i32,
    /// Measured natural text size in physical pixels.
    text_natural_size: Size<i32, Physical>,
    title: String,

    /// Towns style: what the title bar right of the tab needs, kept so a tabbed column can move its
    /// start to the end of the row after all tabs are laid out. `None` when this tab has no bar.
    bar_params: Option<BarParams>,
    has_bar: bool,
    bar_rect: Rectangle<f64, Logical>,
    bar_fill: BorderRenderElement,
    /// Frame-colored top and right edges of the bar.
    bar_line: BorderRenderElement,
    /// Fills the gap between the squared-off frame corner and the rounded window corner on the
    /// top right, like `corner` does on the top left.
    corner_r: BorderRenderElement,
    corner_r_loc: Point<f64, Logical>,
    has_corner_r: bool,
    /// Maximize button at the bar's right end, relative to the tile origin.
    bar_max_rect: Rectangle<f64, Logical>,
    has_bar_max: bool,
    bar_max_icon: RefCell<Option<IconTexture>>,

    texture: RefCell<Option<TextTexture>>,
}

#[derive(Debug, Clone, Copy)]
struct BarParams {
    /// Tile-relative top, bottom and right edges of the bar.
    top: f64,
    bottom: f64,
    right: f64,
    line_width: f64,
    draw_line: bool,
    fill: Color,
    gradient: Gradient,
    gradient_area: Rectangle<f64, Logical>,
    radius: f64,
    scale: f64,
    alpha: f32,
    /// Maximize button at the bar's right end: side length, inset from the right, tile-relative
    /// top, and icon line width. `None` when the window can't be maximized.
    max_button: Option<(f64, f64, f64, f64)>,
    icon_color: Color,
}

#[derive(Debug)]
struct IconTexture {
    size: i32,
    line: i32,
    color: Color,
    buffer: TextureBuffer<GlesTexture>,
}

#[derive(Debug)]
struct TextTexture {
    title: String,
    scale: f64,
    color: Color,
    /// `Some(width)` if the text was ellipsized to fit `width` physical pixels.
    ellipsized_to: Option<i32>,
    buffer: TextureBuffer<GlesTexture>,
}

niri_render_elements! {
    TitleTabRenderElement => {
        Background = BorderRenderElement,
        SolidColor = SolidColorRenderElement,
        Text = PrimaryGpuTextureRenderElement,
    }
}

/// Everything the tab needs to know about the frame it attaches to.
#[derive(Debug, Clone, Copy)]
pub struct FrameInfo {
    /// How far outside the tile the frame extends (focus ring width, or 0).
    pub outset: f64,
    /// Outer corner radius of the frame's top-left corner, before the tab squares it off.
    pub top_left_radius: f32,
    /// Width of the frame line, used for the outline style.
    pub line_width: f64,
    /// Whether a frame (focus ring or border) is actually drawn around this tile.
    pub frame_drawn: bool,
    /// Full frame rectangle relative to the tile origin, for window-relative gradients.
    pub frame_rect: Rectangle<f64, Logical>,
    /// Workspace view rectangle relative to the tile origin, for view-relative gradients.
    pub view_rect: Rectangle<f64, Logical>,
    /// Gradient (or solid color) of the frame, used when the tab has no color of its own.
    pub gradient: Gradient,
    /// Top-left of the window relative to the tile origin.
    pub window_loc: Point<f64, Logical>,
    /// The window's own top-left corner radius. The frame's corner is squared off under the tab,
    /// and the gap between it and the rounded window corner is filled in the tab's color.
    pub window_radius: f64,
}

impl TitleTab {
    pub fn new(config: niri_config::TitleTab) -> Self {
        Self {
            config,
            visible: false,
            rect: Rectangle::default(),
            outer_radius: 0.,
            background: BorderRenderElement::default(),
            outline: BorderRenderElement::default(),
            fillet: BorderRenderElement::default(),
            fillet_fill: BorderRenderElement::default(),
            fillet_loc: Point::from((0., 0.)),
            fillet_radius: 0.,
            fillet_l: BorderRenderElement::default(),
            fillet_fill_l: BorderRenderElement::default(),
            fillet_l_loc: Point::from((0., 0.)),
            corner: BorderRenderElement::default(),
            corner_loc: Point::from((0., 0.)),
            has_corner: false,
            skirt: BorderRenderElement::default(),
            skirt_loc: Point::from((0., 0.)),
            has_skirt: false,
            has_filled_join: false,
            has_left_bend: false,
            attached_left: true,
            row_gap: 0.,
            stub: BorderRenderElement::default(),
            stub_loc: Point::from((0., 0.)),
            outline_width: 0.,
            has_fill: false,
            show_maximize: false,
            draw_outline: false,
            has_bar_outline: false,
            solid: SolidColorBuffer::new((0., 0.), [0., 0., 0., 0.]),
            use_shader: false,
            alpha: 1.,
            scale: 1.,
            text_color: Color::new_unpremul(0., 0., 0., 1.),
            text_loc: Point::from((0., 0.)),
            close_rect: Rectangle::default(),
            close_button: BorderRenderElement::default(),
            close_grid: Vec::new(),
            stripes: Vec::new(),
            mac: false,
            is_active: false,
            max_rect: Rectangle::default(),
            max_back: BorderRenderElement::default(),
            max_front: BorderRenderElement::default(),
            max_front_fill: BorderRenderElement::default(),
            max_back_loc: Point::from((0., 0.)),
            max_front_loc: Point::from((0., 0.)),
            text_max_width: 0,
            text_natural_size: Size::from((0, 0)),
            title: String::new(),
            bar_params: None,
            has_bar: false,
            bar_rect: Rectangle::default(),
            bar_fill: BorderRenderElement::default(),
            bar_line: BorderRenderElement::default(),
            corner_r: BorderRenderElement::default(),
            corner_r_loc: Point::from((0., 0.)),
            has_corner_r: false,
            bar_max_rect: Rectangle::default(),
            has_bar_max: false,
            bar_max_icon: RefCell::new(None),
            texture: RefCell::new(None),
        }
    }

    pub fn update_config(&mut self, config: niri_config::TitleTab) {
        if self.config.font != config.font {
            // Font changed, remeasure and regenerate.
            self.title.clear();
            self.texture.borrow_mut().take();
        }
        self.config = config;
    }

    pub fn update_shaders(&mut self) {
        self.background.damage_all();
        self.outline.damage_all();
        self.fillet.damage_all();
        self.fillet_fill.damage_all();
        self.fillet_l.damage_all();
        self.fillet_fill_l.damage_all();
        self.corner.damage_all();
        self.skirt.damage_all();
        self.stub.damage_all();
        self.close_button.damage_all();
        self.max_back.damage_all();
        self.max_front.damage_all();
        self.max_front_fill.damage_all();
        self.bar_fill.damage_all();
        self.bar_line.damage_all();
        self.corner_r.damage_all();
    }

    /// Tab background rectangle relative to the tile origin, valid after update_render_elements.
    pub fn rect(&self) -> Rectangle<f64, Logical> {
        self.rect
    }

    /// Radius of the outer top corners, valid after update_render_elements.
    pub fn outer_radius(&self) -> f64 {
        self.outer_radius
    }

    /// Returns whether the tab is drawn as an outline that should cut into the frame's top edge.
    pub fn is_outline(&self) -> bool {
        self.visible && self.draw_outline
    }

    pub fn outline_width(&self) -> f64 {
        self.outline_width
    }

    /// Tile-relative X range of the frame's top edge that the tab replaces in outline style.
    pub fn frame_top_edge_gap(&self) -> (f64, f64) {
        let start = if self.has_left_bend {
            self.rect.loc.x - self.fillet_radius
        } else {
            f64::NEG_INFINITY
        };
        (
            start,
            self.rect.loc.x + self.rect.size.w + self.fillet_radius,
        )
    }

    /// Whether the tab squares off the frame's top-left corner.
    pub fn is_attached_left(&self) -> bool {
        self.visible && self.attached_left
    }

    /// Tile-relative X where the next tab in a row should start.
    pub fn row_next_x(&self) -> f64 {
        self.rect.loc.x + self.rect.size.w + self.row_gap
    }

    /// Tile-relative X of this tab's right edge.
    pub fn right_x(&self) -> f64 {
        self.rect.loc.x + self.rect.size.w
    }

    /// Starts the bar at tile-relative `x` instead of this tab's right edge, for a tabbed column
    /// whose shown tile draws the bar after the whole row.
    pub fn set_bar_start(&mut self, x: f64) {
        if self.visible {
            self.update_bar(x);
        }
    }

    pub fn is_off(&self) -> bool {
        self.config.off
    }

    pub fn height(&self) -> f64 {
        self.config.height
    }

    pub fn config(&self) -> &niri_config::TitleTab {
        &self.config
    }

    /// Returns what part of the tab `point` (relative to the tile origin) hits, if any.
    pub fn hit(&self, point: Point<f64, Logical>) -> Option<TitleTabHit> {
        if !self.visible {
            return None;
        }

        // Buttons get a bit of slack around them so they are easy to hit.
        let slack = self.config.padding / 2.;
        let with_slack = |mut rect: Rectangle<f64, Logical>| {
            rect.loc -= Point::from((slack, slack));
            rect.size += Size::from((slack * 2., slack * 2.));
            rect
        };

        if !self.rect.contains(point) {
            if !self.has_bar || !self.bar_rect.contains(point) {
                return None;
            }
            if self.has_bar_max && with_slack(self.bar_max_rect).contains(point) {
                return Some(TitleTabHit::Maximize);
            }
            return Some(TitleTabHit::Body);
        }

        let boxes = !self.mac || self.is_active;
        if boxes && self.config.close_button && with_slack(self.close_rect).contains(point) {
            Some(TitleTabHit::Close)
        } else if self.show_maximize && with_slack(self.max_rect).contains(point) {
            Some(TitleTabHit::Maximize)
        } else {
            Some(TitleTabHit::Body)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_render_elements(
        &mut self,
        visible: bool,
        can_maximize: bool,
        tile_width: f64,
        title: &str,
        is_active: bool,
        is_urgent: bool,
        frame: FrameInfo,
        row: Option<TabRow>,
        scale: f64,
        alpha: f32,
    ) {
        self.visible = visible;
        let is_towns = self.config.style == TitleTabStyle::Towns;
        let is_mac = self.config.style == TitleTabStyle::Mac;
        self.mac = is_mac;
        self.is_active = is_active;
        // Mac style: the boxes only show on the active window.
        self.show_maximize =
            self.config.maximize_button && can_maximize && !is_towns && (!is_mac || is_active);
        self.alpha = alpha;
        if !visible {
            return;
        }

        // Remeasure the title when it (or the scale) changes.
        if self.title != title || self.scale != scale {
            self.title.clear();
            self.title.push_str(title);
            self.scale = scale;
            self.text_natural_size = measure_text(&self.config.font, title, scale)
                .unwrap_or_else(|_| Size::from((0, 0)));
        }

        // Colors.
        let inactive_bar_color = self.config.inactive_bar_color.unwrap_or_else(|| {
            let [r, g, b] = DEFAULT_INACTIVE_BAR_COLOR;
            Color::from_rgba8_unpremul(r, g, b, 255)
        });
        let (color, gradient) = if is_urgent {
            (self.config.urgent_color, self.config.urgent_gradient)
        } else if is_active {
            (self.config.active_color, self.config.active_gradient)
        } else if is_towns {
            // An inactive bar is one color, tab included.
            (
                Some(self.config.inactive_color.unwrap_or(inactive_bar_color)),
                self.config.inactive_gradient,
            )
        } else {
            (self.config.inactive_color, self.config.inactive_gradient)
        };
        let gradient = gradient
            .or_else(|| color.map(Gradient::from))
            .unwrap_or_else(|| {
                if is_mac {
                    let [r, g, b] = DEFAULT_MAC_COLOR;
                    Gradient::from(Color::from_rgba8_unpremul(r, g, b, 255))
                } else if is_towns && !is_urgent {
                    // The active Towns tab is white by default rather than the frame color.
                    let [r, g, b] = DEFAULT_TOWNS_TAB_COLOR;
                    Gradient::from(Color::from_rgba8_unpremul(r, g, b, 255))
                } else {
                    frame.gradient
                }
            });

        let text_color = if is_urgent {
            self.config.urgent_text_color
        } else if is_active {
            self.config.active_text_color
        } else {
            self.config.inactive_text_color
        };
        let fill = match self.config.style {
            TitleTabStyle::Outline => Some(
                self.config
                    .fill_color
                    .unwrap_or_else(|| Color::from_rgba8_unpremul(24, 24, 24, 255)),
            ),
            TitleTabStyle::Filled | TitleTabStyle::Towns | TitleTabStyle::Mac => None,
        };
        self.has_fill = fill.is_some();
        // Like the focus ring, the outline only appears on tiles that have a frame.
        self.draw_outline = fill.is_some() && frame.frame_drawn;
        self.text_color = text_color.unwrap_or_else(|| match fill {
            // Outlined tab: the title matches the frame line.
            Some(_) if self.draw_outline => gradient.from,
            // Solid enough fill: contrast against it. Transparent fill: use the frame color.
            Some(fill) if fill.a >= 0.5 => contrasting_text_color(fill),
            Some(_) => gradient.from,
            // Mac bar: title, stripes and boxes are all drawn in the frame line's color.
            None if is_mac && frame.frame_drawn => frame.gradient.from,
            None => contrasting_text_color(gradient.from),
        });

        // Geometry. The tab spans from the frame's outer edge (outset above and to the left of the
        // tile) down to the bottom of the reserved strip, which is the window's top edge.
        let round = |x: f64| (x * scale).round() / scale;
        let outset = frame.outset;
        let height = self.config.height;
        let padding = round(self.config.padding);

        let natural_w = self.text_natural_size.w as f64 / scale;
        let radius = round(f64::from(frame.top_left_radius) * self.config.corner_radius_factor);
        self.outer_radius = radius;
        // Close button: a square before the title, with a gap to the text.
        let button_w = if self.config.close_button {
            round(self.config.close_button_size)
        } else {
            0.
        };
        let button_gap = if self.config.close_button {
            round(padding * 0.6)
        } else {
            0.
        };
        // With a button in front, inset it from the left as much as from the top, so the square
        // sits in the corner evenly.
        let button_inset = round((height + outset - button_w) / 2.);
        let left_inset = if self.config.close_button {
            button_inset
        } else {
            padding
        };
        let lead = left_inset + button_w + button_gap;

        // Maximize button box after the title, same size as the close button.
        let max_w_btn = if self.show_maximize {
            round(self.config.close_button_size)
        } else {
            0.
        };
        let trail = if self.show_maximize {
            button_gap + max_w_btn + button_inset
        } else {
            padding
        };

        // Bend and gap sizes are fixed per tab so a row lines up regardless of focus.
        let join_r = round(f64::from(frame.top_left_radius) * self.config.join_radius_factor);
        // Towns- and Mac-style tabs sit side by side like segments of one bar, overlapping by the
        // outline width so neighbors share one line.
        self.has_bar_outline = (is_towns || is_mac) && frame.frame_drawn;
        let bar_outline_w = if self.has_bar_outline {
            round(frame.line_width)
        } else {
            0.
        };
        let default_gap = if is_towns || is_mac {
            -bar_outline_w
        } else {
            join_r
        };
        self.row_gap = round(self.config.row_gap.unwrap_or(default_gap));

        // The first tab in a row sits flush on the frame's top-left corner like a lone tab does,
        // squaring it off; the other tabs are free-standing with a bend on both sides and leave
        // the frame's corner rounded. When the first tab's own tile is hidden, it reaches out to
        // the shown frame's outer edge instead of its own.
        let first_in_row = row.is_none_or(|row| row.index == 0);
        let skirt = row.filter(|row| first_in_row && row.shown != row.index);
        self.attached_left = first_in_row;
        self.has_left_bend = !first_in_row;
        let left_outset = skirt.map_or(outset, |row| round(row.frame_outset));
        let x0 = match row {
            Some(row) if !first_in_row => row.x,
            _ => -left_outset,
        };

        let min_w = f64::max(radius * 2., lead + trail);
        let mut max_w = round(tile_width * self.config.max_width_proportion) + left_outset;
        if let Some(row) = row {
            let count = row.count.max(1) as f64;
            let share = (tile_width - self.row_gap * (count - 1.)) / count;
            max_w = f64::min(max_w, round(share));
        }
        let max_w = f64::max(min_w, max_w);
        let frame_right = frame.frame_rect.loc.x + frame.frame_rect.size.w;
        let width = if is_mac {
            // The bar runs to the frame's right edge. In a row the tabs share that width.
            let count = row.map_or(1., |row| row.count.max(1) as f64);
            round((frame_right + left_outset - self.row_gap * (count - 1.)) / count)
        } else {
            (natural_w + lead + trail + left_outset).clamp(min_w, max_w)
        };
        let width = round(width);

        self.rect = Rectangle::new(
            Point::from((x0, -outset)),
            Size::from((width, height + outset)),
        );

        let avail_w = width - left_outset - lead - trail;
        self.text_max_width = (avail_w * scale).floor().max(0.) as i32;

        let text_h = self.text_natural_size.h as f64 / scale;
        self.text_loc = Point::from((
            self.rect.loc.x + left_outset + lead,
            self.rect.loc.y + round((self.rect.size.h - text_h) / 2.),
        ))
        .to_physical_precise_round(scale)
        .to_logical(scale);

        self.close_rect = Rectangle::new(
            Point::from((
                self.rect.loc.x + left_outset + left_inset,
                self.rect.loc.y + button_inset,
            ))
            .to_physical_precise_round(scale)
            .to_logical(scale),
            Size::from((button_w, button_w)),
        );

        self.max_rect = Rectangle::new(
            Point::from((
                self.rect.loc.x + width - button_inset - max_w_btn,
                self.rect.loc.y + round((self.rect.size.h - max_w_btn) / 2.),
            ))
            .to_physical_precise_round(scale)
            .to_logical(scale),
            Size::from((max_w_btn, max_w_btn)),
        );

        // Mac style: close box and zoom box are centered below the top line, one box width in
        // from the sides; the title is centered between them (the box room is reserved either
        // way so it doesn't move with focus); stripes as thick as the gaps between them fill the
        // rest while active, sized so a margin of one stripe fits above and below the block.
        let box_w = round(self.config.close_button_size);
        let inner_h = self.rect.size.h - bar_outline_w;
        let box_top = round((inner_h - box_w) / 2.);
        let box_inset = bar_outline_w + box_top;
        let box_side = bar_outline_w + box_w;
        let stripe = f64::max(
            1.,
            ((inner_h * scale) / (MAC_STRIPES as f64 * 2. + 1.)).floor(),
        ) / scale;
        let mut text_w = 0.;
        if is_mac {
            let has_box = self.config.close_button || (self.config.maximize_button && can_maximize);
            let side = if has_box {
                box_side + box_w + box_top
            } else {
                padding
            };
            let avail_w = f64::max(0., width - (side + padding) * 2.);
            self.text_max_width = (avail_w * scale).floor().max(0.) as i32;
            text_w = f64::min(natural_w, avail_w);
            self.text_loc = Point::from((
                self.rect.loc.x + round((width - text_w) / 2.),
                self.text_loc.y,
            ))
            .to_physical_precise_round(scale)
            .to_logical(scale);

            self.close_rect = Rectangle::new(
                Point::from((
                    self.rect.loc.x + left_outset + box_side,
                    self.rect.loc.y + box_inset,
                ))
                .to_physical_precise_round(scale)
                .to_logical(scale),
                Size::from((button_w, button_w)),
            );
            self.max_rect = Rectangle::new(
                Point::from((
                    self.rect.loc.x + width - box_side - max_w_btn,
                    self.rect.loc.y + box_inset,
                ))
                .to_physical_precise_round(scale)
                .to_logical(scale),
                Size::from((max_w_btn, max_w_btn)),
            );
        }

        self.stripes.clear();
        if is_mac && is_active {
            // The stripes keep the same distance from the sides as the first one keeps from
            // the top.
            let block_h = stripe * (MAC_STRIPES as f64 * 2. - 1.);
            let margin = round((inner_h - block_h) / 2.);
            let inner_left = self.rect.loc.x + bar_outline_w + margin;
            let inner_right = self.rect.loc.x + width - bar_outline_w - margin;
            // Spans the stripes stay out of: a stripe's width around each box, the padding
            // around the title.
            let mut holes: Vec<(f64, f64)> = Vec::new();
            if self.config.close_button {
                let r = self.close_rect;
                holes.push((r.loc.x - stripe, r.loc.x + r.size.w + stripe));
            }
            if text_w > 0. && !self.title.is_empty() {
                let x = self.text_loc.x;
                holes.push((x - padding, x + text_w + padding));
            }
            if self.show_maximize {
                let r = self.max_rect;
                holes.push((r.loc.x - stripe, r.loc.x + r.size.w + stripe));
            }
            holes.sort_by(|a, b| a.0.total_cmp(&b.0));

            let top = self.rect.loc.y + bar_outline_w + margin;
            let color = self.text_color.to_array_premul();
            let mut x = inner_left;
            for (start, end) in holes.into_iter().chain([(inner_right, inner_right)]) {
                let start = round(f64::min(start, inner_right));
                if start - x >= stripe {
                    let len = start - x;
                    for i in 0..MAC_STRIPES {
                        let y = top + f64::from(i) * stripe * 2.;
                        let buffer = SolidColorBuffer::new((len, stripe), color);
                        self.stripes.push((Point::from((x, y)), buffer));
                    }
                }
                x = f64::max(x, round(end));
            }
        }

        // Close button: an outlined square in the text color.
        // The Towns style uses a thinner stroke, matching its maximize icon.
        let close_line_div = if is_towns { 10. } else { 8. };
        if self.config.close_button {
            let line = if is_mac {
                stripe
            } else {
                f64::max(1. / scale, round(button_w / close_line_div))
            };
            self.close_button.update(
                self.close_rect.size,
                Rectangle::from_size(self.close_rect.size),
                GradientInterpolation::default(),
                self.text_color,
                self.text_color,
                0.,
                Rectangle::from_size(self.close_rect.size),
                line as f32,
                CornerRadius::default(),
                scale as f32,
                alpha,
            );
        }

        // Towns style: a 5x5 grid inside the close button, or 3x3 when the cells
        // would get too small. Inner lines match the outline; cells need two physical pixels,
        // and a grid whose cells come out even wins over one that doesn't.
        self.close_grid.clear();
        if self.config.close_button && is_towns {
            let size = (button_w * scale).round();
            let line = f64::max(1., (round(button_w / close_line_div) * scale).round());
            let inner = size - line * 2.;
            let space = |n: f64| inner - (n - 1.) * line;
            let fits = |n: &f64| space(*n) >= n * 2.;
            let cells = [5., 3.]
                .into_iter()
                .filter(fits)
                .find(|&n| space(n) % n == 0.)
                .or_else(|| [5., 3.].into_iter().find(fits));
            if let Some(n) = cells {
                let cell = (inner - (n - 1.) * line) / n;
                let origin = self.close_rect.loc;
                for i in 1..n as i32 {
                    let i = f64::from(i);
                    let pos = (line + i * cell + (i - 1.) * line).round() / scale;
                    let start = line / scale;
                    let len = inner / scale;
                    let thin = line / scale;
                    let color = self.text_color.to_array_premul();
                    let vertical = SolidColorBuffer::new((thin, len), color);
                    let horizontal = SolidColorBuffer::new((len, thin), color);
                    self.close_grid
                        .push((origin + Point::from((pos, start)), vertical));
                    self.close_grid
                        .push((origin + Point::from((start, pos)), horizontal));
                }
            }
        }

        // Maximize button: two overlapping outlined squares, BeOS zoom style. The front square
        // is filled so it covers the back one.
        if self.show_maximize {
            let s = self.max_rect.size.w;
            let line = f64::max(1. / scale, round(s / 8.));
            let back = round(s * 0.72);
            let front = round(s * 0.58);
            // Big square at the bottom-right, small one in front at the top-left, as on BeOS.
            // Mac zoom box: the full square with a smaller one sharing its top-left corner.
            let (line, back, front) = if is_mac {
                (stripe, s, round(s * 0.64))
            } else {
                (line, back, front)
            };
            self.max_back_loc = self.max_rect.loc + Point::from((s - back, s - back));
            self.max_front_loc = self.max_rect.loc;
            let square = |elem: &mut BorderRenderElement, size: f64, color: Color, bw: f64| {
                let size = Size::from((size, size));
                elem.update(
                    size,
                    Rectangle::from_size(size),
                    GradientInterpolation::default(),
                    color,
                    color,
                    0.,
                    Rectangle::from_size(size),
                    bw as f32,
                    CornerRadius::default(),
                    scale as f32,
                    alpha,
                );
            };
            square(&mut self.max_back, back, self.text_color, line);
            square(&mut self.max_front, front, self.text_color, line);
            let fill_color = fill.unwrap_or(gradient.from);
            square(&mut self.max_front_fill, front, fill_color, 0.);
        }

        // Background. Only the top corners are rounded: the bottom merges into the frame.
        // Towns- and Mac-style tabs are segments of one bar, so only its outer corners are rounded:
        // the first tab's top-left, and the last tab's top-right when no bar is left after it.
        let (round_left, round_right) = if is_towns || is_mac {
            let last_in_row = row.is_none_or(|row| row.index + 1 == row.count);
            let frame_right = frame.frame_rect.loc.x + frame.frame_rect.size.w;
            (first_in_row, last_in_row && x0 + width >= frame_right)
        } else {
            (true, true)
        };
        let corner_radius = CornerRadius {
            top_left: if round_left { radius as f32 } else { 0. },
            top_right: if round_right { radius as f32 } else { 0. },
            bottom_right: 0.,
            bottom_left: 0.,
        }
        .fit_to(width as f32, (height + outset) as f32);
        self.use_shader = corner_radius != CornerRadius::default() || gradient.from != gradient.to;

        let gradient_area = match gradient.relative_to {
            GradientRelativeTo::Window => frame.frame_rect,
            GradientRelativeTo::WorkspaceView => frame.view_rect,
        };

        // Window corner: the frame's top-left is squared off under the tab, but the window keeps
        // its rounded corner. Fill the gap in the tab's color so the tab flows around it. The
        // element is the window's corner square; the geometry is a rounded rect whose inner edge
        // is the window's arc, wide enough that its outer edge clears the square's far corner.
        let r = round(frame.window_radius);
        self.has_corner = self.attached_left && r > 0.;
        if self.has_corner {
            self.corner_loc = frame.window_loc;
            let w = (r * (std::f64::consts::SQRT_2 - 1.)).ceil() + 1.;
            let (from, to) = match fill {
                Some(fill) => (fill, fill),
                // Under a bar the window sits below the frame line, so match that.
                None if is_towns && frame.frame_drawn => (frame.gradient.from, frame.gradient.to),
                None => (gradient.from, gradient.to),
            };
            self.corner.update(
                Size::from((r, r)),
                Rectangle::new(gradient_area.loc - self.corner_loc, gradient_area.size),
                gradient.in_,
                from,
                to,
                ((gradient.angle as f32) - 90.).to_radians(),
                Rectangle::new(
                    Point::from((-w, -w)),
                    Size::from(((r + w) * 2., (r + w) * 2.)),
                ),
                w as f32,
                CornerRadius {
                    top_left: (r + w) as f32,
                    top_right: 0.,
                    bottom_right: 0.,
                    bottom_left: 0.,
                },
                scale as f32,
                alpha,
            );
        }

        // Skirt: same shape as the window corner fill, with its inner arc on the shown frame's
        // outer corner. It goes behind that frame, so only the part outside the arc shows.
        let r = skirt.map_or(0., |row| round(row.frame_radius));
        self.has_skirt = r > 0.;
        if let Some(row) = skirt.filter(|_| self.has_skirt) {
            let o = round(row.frame_outset);
            self.skirt_loc = Point::from((-o, height - o));
            let w = (r * (std::f64::consts::SQRT_2 - 1.)).ceil() + 1.;
            let (from, to) = match fill {
                Some(fill) => (fill, fill),
                None => (gradient.from, gradient.to),
            };
            self.skirt.update(
                Size::from((r, r)),
                Rectangle::new(gradient_area.loc - self.skirt_loc, gradient_area.size),
                gradient.in_,
                from,
                to,
                ((gradient.angle as f32) - 90.).to_radians(),
                Rectangle::new(
                    Point::from((-w, -w)),
                    Size::from(((r + w) * 2., (r + w) * 2.)),
                ),
                w as f32,
                CornerRadius {
                    top_left: (r + w) as f32,
                    top_right: 0.,
                    bottom_right: 0.,
                    bottom_left: 0.,
                },
                scale as f32,
                alpha,
            );
        }

        if let Some(fill) = fill {
            // Outline style: the tab is a frame-colored line around a filled interior. The
            // geometry extends below the element so the bottom edge is clipped off and the tab
            // opens into the window frame.
            // Without an outline the stroke width is zero and only the fill pieces matter.
            self.outline_width = if self.draw_outline {
                round(frame.line_width)
            } else {
                0.
            };
            let w = self.outline_width;
            // Bend radius at the inside join; the stroke keeps its width around it.
            let r = round(f64::from(frame.top_left_radius) * self.config.join_radius_factor);
            self.fillet_radius = r;
            let rc = r + w;

            let open_geometry = Rectangle::from_size(Size::from((width, height + outset + w * 2.)));
            // The outline stops where the bend starts, so its right edge doesn't poke into it.
            let outline_size = Size::from((width, f64::max(w, self.rect.size.h - rc)));
            self.outline.update(
                outline_size,
                Rectangle::new(gradient_area.loc - self.rect.loc, gradient_area.size),
                gradient.in_,
                gradient.from,
                gradient.to,
                ((gradient.angle as f32) - 90.).to_radians(),
                open_geometry,
                w as f32,
                corner_radius,
                scale as f32,
                alpha,
            );

            // Left edge continues down to the frame (first tab only; others get a left bend).
            self.stub_loc = Point::from((self.rect.loc.x, self.rect.loc.y + outline_size.h));
            let stub_size = if self.has_left_bend {
                Size::from((w, 0.))
            } else {
                Size::from((w, self.rect.size.h - outline_size.h))
            };
            self.stub.update(
                stub_size,
                Rectangle::new(gradient_area.loc - self.stub_loc, gradient_area.size),
                gradient.in_,
                gradient.from,
                gradient.to,
                ((gradient.angle as f32) - 90.).to_radians(),
                Rectangle::from_size(stub_size),
                0.,
                CornerRadius::default(),
                scale as f32,
                alpha,
            );
            self.background.update(
                self.rect.size,
                Rectangle::from_size(self.rect.size),
                GradientInterpolation::default(),
                fill,
                fill,
                0.,
                open_geometry,
                0.,
                corner_radius,
                scale as f32,
                alpha,
            );
            self.use_shader = true;
            self.solid.update(self.rect.size, fill.to_array_premul());

            // Bend: the corner zone of a rounded rect whose arc is centered at the bend center,
            // so the border band is an annulus sector between radius r (wallpaper side) and r + w.
            self.fillet_loc = Point::from((
                self.rect.loc.x + width - w,
                self.rect.loc.y + self.rect.size.h - rc,
            ));
            let fillet_size = Size::from((rc, rc));
            let corner = |radius: f64| CornerRadius {
                top_left: 0.,
                top_right: 0.,
                bottom_right: 0.,
                bottom_left: radius as f32,
            };
            self.fillet.update(
                fillet_size,
                Rectangle::new(gradient_area.loc - self.fillet_loc, gradient_area.size),
                gradient.in_,
                gradient.from,
                gradient.to,
                ((gradient.angle as f32) - 90.).to_radians(),
                Rectangle::new(Point::from((0., -rc)), Size::from((rc * 2., rc * 2.))),
                w as f32,
                corner(rc),
                scale as f32,
                alpha,
            );

            // Inside of the bend: fill color beyond radius r + w, out to the zone's far corner.
            // Without an outline this alone rounds the join between the tab and the window.
            let rc2 = (rc * std::f64::consts::SQRT_2).ceil() + 1.;
            self.fillet_fill.update(
                fillet_size,
                Rectangle::from_size(fillet_size),
                GradientInterpolation::default(),
                fill,
                fill,
                0.,
                Rectangle::new(
                    Point::from((rc - rc2, -rc2)),
                    Size::from((rc2 * 2., rc2 * 2.)),
                ),
                (rc2 - rc) as f32,
                corner(rc2),
                scale as f32,
                alpha,
            );

            // Mirrored bend on the left for tabs that are not first in a row. The arc center is
            // the element's top-left corner, so the geometry's bottom-right corner is rounded.
            self.fillet_l_loc =
                Point::from((self.rect.loc.x - r, self.rect.loc.y + self.rect.size.h - rc));
            let corner_r = |radius: f64| CornerRadius {
                top_left: 0.,
                top_right: 0.,
                bottom_right: radius as f32,
                bottom_left: 0.,
            };
            self.fillet_l.update(
                fillet_size,
                Rectangle::new(gradient_area.loc - self.fillet_l_loc, gradient_area.size),
                gradient.in_,
                gradient.from,
                gradient.to,
                ((gradient.angle as f32) - 90.).to_radians(),
                Rectangle::new(Point::from((-rc, -rc)), Size::from((rc * 2., rc * 2.))),
                w as f32,
                corner_r(rc),
                scale as f32,
                alpha,
            );
            self.fillet_fill_l.update(
                fillet_size,
                Rectangle::from_size(fillet_size),
                GradientInterpolation::default(),
                fill,
                fill,
                0.,
                Rectangle::new(Point::from((-rc2, -rc2)), Size::from((rc2 * 2., rc2 * 2.))),
                (rc2 - rc) as f32,
                corner_r(rc2),
                scale as f32,
                alpha,
            );
        } else {
            // Bend where the tab's right edge meets the frame, filled in the tab's color. Left
            // out when another tab follows right away, where the two would overlap.
            let r = round(f64::from(frame.top_left_radius) * self.config.join_radius_factor);
            let tab_follows = row.is_some_and(|row| row.index + 1 < row.count);
            let fits =
                self.rect.loc.x + width + r <= frame.frame_rect.loc.x + frame.frame_rect.size.w;
            self.has_filled_join = r > 0. && !tab_follows && fits && !is_towns && !is_mac;
            self.fillet_radius = 0.;
            if self.has_filled_join {
                self.use_shader = true;
                // The bend sits on the frame's outer top edge, above the focus ring if any.
                self.fillet_loc =
                    Point::from((self.rect.loc.x + width, frame.frame_rect.loc.y - r));
                let fillet_size = Size::from((r, r));
                let rc2 = (r * std::f64::consts::SQRT_2).ceil() + 1.;
                self.fillet_fill.update(
                    fillet_size,
                    Rectangle::new(gradient_area.loc - self.fillet_loc, gradient_area.size),
                    gradient.in_,
                    gradient.from,
                    gradient.to,
                    ((gradient.angle as f32) - 90.).to_radians(),
                    Rectangle::new(
                        Point::from((r - rc2, -rc2)),
                        Size::from((rc2 * 2., rc2 * 2.)),
                    ),
                    (rc2 - r) as f32,
                    CornerRadius {
                        top_left: 0.,
                        top_right: 0.,
                        bottom_right: 0.,
                        bottom_left: rc2 as f32,
                    },
                    scale as f32,
                    alpha,
                );
            }

            self.background.update(
                self.rect.size,
                Rectangle::new(gradient_area.loc - self.rect.loc, gradient_area.size),
                gradient.in_,
                gradient.from,
                gradient.to,
                ((gradient.angle as f32) - 90.).to_radians(),
                Rectangle::from_size(self.rect.size),
                0.,
                corner_radius,
                scale as f32,
                alpha,
            );
            self.solid
                .update(self.rect.size, gradient.from.to_array_premul());

            // Towns and Mac styles: stroke the top and sides in the frame color. The geometry
            // reaches below the tab so the bottom edge is left to the frame.
            if self.has_bar_outline {
                self.use_shader = true;
                let g = frame.gradient;
                let w = bar_outline_w;
                self.outline.update(
                    self.rect.size,
                    Rectangle::new(gradient_area.loc - self.rect.loc, gradient_area.size),
                    g.in_,
                    g.from,
                    g.to,
                    ((g.angle as f32) - 90.).to_radians(),
                    Rectangle::from_size(self.rect.size + Size::from((0., w))),
                    w as f32,
                    corner_radius,
                    scale as f32,
                    alpha,
                );
            }
        }

        // Towns style: the bar runs from the tab to the frame's right edge, down to the frame's top
        // edge. In a tabbed column only the shown tile draws it, and the column moves its start
        // past the last tab afterwards.
        let has_bar = is_towns && row.is_none_or(|row| row.index == row.shown);
        self.bar_params = has_bar.then(|| {
            let fill = if is_active {
                self.config.bar_color.unwrap_or_else(|| {
                    let [r, g, b] = DEFAULT_BAR_COLOR;
                    Color::from_rgba8_unpremul(r, g, b, 255)
                })
            } else {
                inactive_bar_color
            };
            BarParams {
                top: -outset,
                bottom: frame.frame_rect.loc.y,
                right: frame.frame_rect.loc.x + frame.frame_rect.size.w,
                line_width: round(frame.line_width),
                draw_line: frame.frame_drawn,
                fill,
                gradient: frame.gradient,
                gradient_area,
                radius,
                scale,
                alpha,
                // Same size and inset as the close button, mirrored to the right end.
                max_button: (self.config.maximize_button && can_maximize).then(|| {
                    let size = round(self.config.close_button_size);
                    let inset = round((height + outset - size) / 2.);
                    // Thinner than the close button's outline so the arrowheads stay apart.
                    let line = f64::max(1. / scale, round(size / 10.));
                    (size, inset, -outset + inset, line)
                }),
                icon_color: contrasting_text_color(fill),
            }
        });
        self.update_bar(self.rect.loc.x + width);

        // The frame's top-right corner is squared off under the bar; fill the gap to the rounded
        // window corner in the frame color, or the bar color when there is no frame.
        let r = round(frame.window_radius);
        self.has_corner_r = is_towns && r > 0.;
        if self.has_corner_r {
            let w = (r * (std::f64::consts::SQRT_2 - 1.)).ceil() + 1.;
            let right = tile_width - frame.window_loc.x;
            self.corner_r_loc = Point::from((right - r, frame.window_loc.y));
            let g = if frame.frame_drawn {
                frame.gradient
            } else {
                gradient
            };
            self.corner_r.update(
                Size::from((r, r)),
                Rectangle::new(gradient_area.loc - self.corner_r_loc, gradient_area.size),
                g.in_,
                g.from,
                g.to,
                ((g.angle as f32) - 90.).to_radians(),
                Rectangle::new(
                    Point::from((-(r + w), -w)),
                    Size::from(((r + w) * 2., (r + w) * 2.)),
                ),
                w as f32,
                CornerRadius {
                    top_left: 0.,
                    top_right: (r + w) as f32,
                    bottom_right: 0.,
                    bottom_left: 0.,
                },
                scale as f32,
                alpha,
            );
        }
    }

    fn update_bar(&mut self, start_x: f64) {
        self.has_bar = false;
        let Some(bar) = self.bar_params else {
            return;
        };
        let size = Size::from((bar.right - start_x, bar.bottom - bar.top));
        if size.w <= 0. || size.h <= 0. {
            return;
        }
        self.has_bar = true;
        self.bar_rect = Rectangle::new(Point::from((start_x, bar.top)), size);

        // Maximize button, only when the bar has room for it with the inset on both sides.
        self.has_bar_max = false;
        if let Some((btn, inset, top, _)) = bar.max_button {
            if size.w >= btn + inset * 2. {
                self.has_bar_max = true;
                self.bar_max_rect = Rectangle::new(
                    Point::from((bar.right - inset - btn, top))
                        .to_physical_precise_round(bar.scale)
                        .to_logical(bar.scale),
                    Size::from((btn, btn)),
                );
            }
        }

        // The left edge meets the tab and the bottom edge is the frame's top edge, so the geometry
        // reaches past both and only the top and right edges get stroked.
        let w = if bar.draw_line { bar.line_width } else { 0. };
        let geometry = Rectangle::new(Point::from((-w, 0.)), Size::from((size.w + w, size.h + w)));
        let corner_radius = CornerRadius {
            top_left: 0.,
            top_right: bar.radius as f32,
            bottom_right: 0.,
            bottom_left: 0.,
        }
        .fit_to(geometry.size.w as f32, geometry.size.h as f32);

        let g = bar.gradient;
        self.bar_line.update(
            size,
            Rectangle::new(
                bar.gradient_area.loc - self.bar_rect.loc,
                bar.gradient_area.size,
            ),
            g.in_,
            g.from,
            g.to,
            ((g.angle as f32) - 90.).to_radians(),
            geometry,
            w as f32,
            corner_radius,
            bar.scale as f32,
            bar.alpha,
        );
        self.bar_fill.update(
            size,
            Rectangle::from_size(size),
            GradientInterpolation::default(),
            bar.fill,
            bar.fill,
            0.,
            geometry,
            0.,
            corner_radius,
            bar.scale as f32,
            bar.alpha,
        );
    }

    pub fn render(
        &self,
        renderer: &mut impl NiriRenderer,
        location: Point<f64, Logical>,
        with_corner: bool,
        push: &mut dyn FnMut(TitleTabRenderElement),
    ) {
        if !self.visible {
            return;
        }

        // Text and close button go on top. An inactive Mac bar shows no boxes.
        let show_close = self.config.close_button && (!self.mac || self.is_active);
        if show_close && self.close_rect.size.w > 0. {
            for (loc, buffer) in &self.close_grid {
                push(
                    SolidColorRenderElement::from_buffer(
                        buffer,
                        location + *loc,
                        self.alpha,
                        Kind::Unspecified,
                    )
                    .into(),
                );
            }
            push(
                self.close_button
                    .clone()
                    .with_location(location + self.close_rect.loc)
                    .into(),
            );
        }
        if self.show_maximize && self.max_rect.size.w > 0. {
            push(
                self.max_front
                    .clone()
                    .with_location(location + self.max_front_loc)
                    .into(),
            );
            if !self.mac {
                push(
                    self.max_front_fill
                        .clone()
                        .with_location(location + self.max_front_loc)
                        .into(),
                );
            }
            push(
                self.max_back
                    .clone()
                    .with_location(location + self.max_back_loc)
                    .into(),
            );
        }
        if let Some(buffer) = self.text_texture(renderer.as_gles_renderer()) {
            let elem = TextureRenderElement::from_texture_buffer(
                buffer,
                location + self.text_loc,
                self.alpha,
                None,
                None,
                Kind::Unspecified,
            );
            push(PrimaryGpuTextureRenderElement(elem).into());
        }
        for (loc, buffer) in &self.stripes {
            push(
                SolidColorRenderElement::from_buffer(
                    buffer,
                    location + *loc,
                    self.alpha,
                    Kind::Unspecified,
                )
                .into(),
            );
        }

        let has_shader = BorderRenderElement::has_shader(renderer);
        if self.has_bar && self.has_bar_max {
            if let Some(buffer) = self.bar_max_texture(renderer.as_gles_renderer()) {
                let elem = TextureRenderElement::from_texture_buffer(
                    buffer,
                    location + self.bar_max_rect.loc,
                    self.alpha,
                    None,
                    None,
                    Kind::Unspecified,
                );
                push(PrimaryGpuTextureRenderElement(elem).into());
            }
        }
        if self.has_bar && has_shader {
            let loc = location + self.bar_rect.loc;
            if self.bar_params.is_some_and(|bar| bar.draw_line) {
                push(self.bar_line.clone().with_location(loc).into());
            }
            push(self.bar_fill.clone().with_location(loc).into());
        }
        if with_corner && self.has_corner_r && has_shader {
            push(
                self.corner_r
                    .clone()
                    .with_location(location + self.corner_r_loc)
                    .into(),
            );
        }
        if !with_corner && self.has_skirt && has_shader {
            push(
                self.skirt
                    .clone()
                    .with_location(location + self.skirt_loc)
                    .into(),
            );
        }
        if with_corner && self.has_corner && has_shader {
            push(
                self.corner
                    .clone()
                    .with_location(location + self.corner_loc)
                    .into(),
            );
        }
        if self.use_shader && has_shader {
            if self.draw_outline {
                push(
                    self.outline
                        .clone()
                        .with_location(location + self.rect.loc)
                        .into(),
                );
                if self.has_left_bend {
                    push(
                        self.fillet_l
                            .clone()
                            .with_location(location + self.fillet_l_loc)
                            .into(),
                    );
                } else {
                    push(
                        self.stub
                            .clone()
                            .with_location(location + self.stub_loc)
                            .into(),
                    );
                }
                push(
                    self.fillet
                        .clone()
                        .with_location(location + self.fillet_loc)
                        .into(),
                );
            }
            if self.has_bar_outline {
                push(
                    self.outline
                        .clone()
                        .with_location(location + self.rect.loc)
                        .into(),
                );
            }
            push(
                self.background
                    .clone()
                    .with_location(location + self.rect.loc)
                    .into(),
            );
            if self.has_fill {
                push(
                    self.fillet_fill
                        .clone()
                        .with_location(location + self.fillet_loc)
                        .into(),
                );
                if self.has_left_bend {
                    push(
                        self.fillet_fill_l
                            .clone()
                            .with_location(location + self.fillet_l_loc)
                            .into(),
                    );
                }
            } else if self.has_filled_join {
                push(
                    self.fillet_fill
                        .clone()
                        .with_location(location + self.fillet_loc)
                        .into(),
                );
            }
        } else {
            push(
                SolidColorRenderElement::from_buffer(
                    &self.solid,
                    location + self.rect.loc,
                    self.alpha,
                    Kind::Unspecified,
                )
                .into(),
            );
        }
    }

    fn bar_max_texture(&self, renderer: &mut GlesRenderer) -> Option<TextureBuffer<GlesTexture>> {
        let bar = self.bar_params?;
        let (btn, _, _, line) = bar.max_button?;
        let size = (btn * bar.scale).round() as i32;
        let line = (line * bar.scale).round() as i32;
        if size <= 0 {
            return None;
        }

        let mut cache = self.bar_max_icon.borrow_mut();
        if let Some(entry) = &*cache {
            if entry.size == size && entry.line == line && entry.color == bar.icon_color {
                return Some(entry.buffer.clone());
            }
        }

        let buffer = match generate_maximize_icon(renderer, size, line, bar.icon_color, bar.scale) {
            Ok(buffer) => buffer,
            Err(err) => {
                warn!("error generating maximize icon texture: {err:?}");
                return None;
            }
        };

        *cache = Some(IconTexture {
            size,
            line,
            color: bar.icon_color,
            buffer: buffer.clone(),
        });

        Some(buffer)
    }

    fn text_texture(&self, renderer: &mut GlesRenderer) -> Option<TextureBuffer<GlesTexture>> {
        if self.title.is_empty() || self.text_max_width <= 0 {
            return None;
        }

        let needs_ellipsis = self.text_natural_size.w > self.text_max_width;
        let wanted = needs_ellipsis.then_some(self.text_max_width);

        let mut cache = self.texture.borrow_mut();
        if let Some(entry) = &*cache {
            if entry.title == self.title
                && entry.scale == self.scale
                && entry.color == self.text_color
                && entry.ellipsized_to == wanted
            {
                return Some(entry.buffer.clone());
            }
        }

        let buffer = match generate_text_texture(
            renderer,
            &self.config.font,
            &self.title,
            self.scale,
            self.text_color,
            wanted,
        ) {
            Ok(buffer) => buffer,
            Err(err) => {
                warn!("error generating title tab texture: {err:?}");
                return None;
            }
        };

        *cache = Some(TextTexture {
            title: self.title.clone(),
            scale: self.scale,
            color: self.text_color,
            ellipsized_to: wanted,
            buffer: buffer.clone(),
        });

        Some(buffer)
    }
}

/// Picks black or white text depending on the perceived brightness of the background.
fn contrasting_text_color(bg: Color) -> Color {
    let luma = 0.2126 * bg.r + 0.7152 * bg.g + 0.0722 * bg.b;
    if luma > 0.55 {
        Color::from_rgba8_unpremul(20, 20, 24, 255)
    } else {
        Color::from_rgba8_unpremul(240, 240, 240, 255)
    }
}

fn make_layout(
    cr: &cairo::Context,
    font: &str,
    title: &str,
    scale: f64,
    ellipsize_to: Option<i32>,
) -> pango::Layout {
    let mut font = FontDescription::from_string(font);
    font.set_absolute_size(to_physical_precise_round(scale, font.size()));

    let layout = pangocairo::functions::create_layout(cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_single_paragraph_mode(true);
    layout.set_font_description(Some(&font));
    layout.set_text(title);

    if let Some(width) = ellipsize_to {
        layout.set_width(width * pango::SCALE);
        layout.set_ellipsize(pango::EllipsizeMode::End);
    }

    layout
}

fn measure_text(font: &str, title: &str, scale: f64) -> anyhow::Result<Size<i32, Physical>> {
    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let layout = make_layout(&cr, font, title, scale, None);
    let (width, height) = layout.pixel_size();
    Ok(Size::from((width, height)))
}

/// Draws the Towns style's maximize icon: an X whose four ends are arrowheads pointing out to the
/// corners.
fn generate_maximize_icon(
    renderer: &mut GlesRenderer,
    size: i32,
    line: i32,
    color: Color,
    scale: f64,
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    let surface = ImageSurface::create(cairo::Format::ARgb32, size, size)?;
    let cr = cairo::Context::new(&surface)?;
    cr.set_source_rgba(
        f64::from(color.r),
        f64::from(color.g),
        f64::from(color.b),
        f64::from(color.a),
    );
    let s = f64::from(size);
    let w = f64::from(line);
    cr.set_line_width(w);
    cr.set_line_cap(cairo::LineCap::Butt);
    cr.set_line_join(cairo::LineJoin::Miter);

    // Corner points sit half a line in so the arrowheads' outer edges touch the box.
    let a = w / 2.;
    let b = s - w / 2.;
    let head = (s * 0.3).round();

    // The two diagonals.
    cr.move_to(a, a);
    cr.line_to(b, b);
    cr.move_to(b, a);
    cr.line_to(a, b);
    cr.stroke()?;

    // Arrowheads: an L in each corner.
    for (x, y, dx, dy) in [
        (a, a, 1., 1.),
        (b, a, -1., 1.),
        (a, b, 1., -1.),
        (b, b, -1., -1.),
    ] {
        cr.move_to(x, y + dy * head);
        cr.line_to(x, y);
        cr.line_to(x + dx * head, y);
        cr.stroke()?;
    }

    drop(cr);
    let data = surface.take_data().unwrap();
    let buffer = TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (size, size),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )?;

    Ok(buffer)
}

fn generate_text_texture(
    renderer: &mut GlesRenderer,
    font: &str,
    title: &str,
    scale: f64,
    color: Color,
    ellipsize_to: Option<i32>,
) -> anyhow::Result<TextureBuffer<GlesTexture>> {
    let _span = tracy_client::span!("title_tab::generate_text_texture");

    let surface = ImageSurface::create(cairo::Format::ARgb32, 0, 0)?;
    let cr = cairo::Context::new(&surface)?;
    let layout = make_layout(&cr, font, title, scale, ellipsize_to);

    let (width, height) = layout.pixel_size();
    ensure!(width > 0 && height > 0);
    let width = min(width, MAX_TEXTURE_SIZE);
    let height = min(height, MAX_TEXTURE_SIZE);

    let surface = ImageSurface::create(cairo::Format::ARgb32, width, height)?;
    let cr = cairo::Context::new(&surface)?;
    cr.set_source_rgba(
        f64::from(color.r),
        f64::from(color.g),
        f64::from(color.b),
        f64::from(color.a),
    );
    pangocairo::functions::show_layout(&cr, &layout);

    drop(cr);
    let data = surface.take_data().unwrap();
    let buffer = TextureBuffer::from_memory(
        renderer,
        &data,
        Fourcc::Argb8888,
        (width, height),
        false,
        scale,
        Transform::Normal,
        Vec::new(),
    )?;

    Ok(buffer)
}
