//! BeOS-style title tab drawn above the top-left corner of a tile.
//!
//! The tab lives in a strip reserved at the top of the tile (see `Tile::tab_height`). Its
//! background is drawn with the border shader so it shares the frame's color, gradient and corner
//! radius, and it overlaps the top edge of the focus ring so the two blend into one shape.

use std::cell::RefCell;
use std::cmp::min;

use anyhow::ensure;
use niri_config::{Color, CornerRadius, Gradient, GradientInterpolation, GradientRelativeTo};
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

/// Placement of a tab within a row of tabs (tabbed columns).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabRow {
    /// Tile-relative X where this tab starts (ignored for the first tab, which is flush left).
    pub x: f64,
    pub index: usize,
    pub count: usize,
}

#[derive(Debug)]
pub struct TitleTab {
    config: niri_config::TitleTab,

    // Everything below is computed in update_render_elements().
    visible: bool,
    /// Tab background rectangle, relative to the tile origin.
    rect: Rectangle<f64, Logical>,
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
    has_left_bend: bool,
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
    /// Maximum text width in physical pixels; longer titles get ellipsized.
    text_max_width: i32,
    /// Measured natural text size in physical pixels.
    text_natural_size: Size<i32, Physical>,
    title: String,

    texture: RefCell<Option<TextTexture>>,
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
}

impl TitleTab {
    pub fn new(config: niri_config::TitleTab) -> Self {
        Self {
            config,
            visible: false,
            rect: Rectangle::default(),
            background: BorderRenderElement::default(),
            outline: BorderRenderElement::default(),
            fillet: BorderRenderElement::default(),
            fillet_fill: BorderRenderElement::default(),
            fillet_loc: Point::from((0., 0.)),
            fillet_radius: 0.,
            fillet_l: BorderRenderElement::default(),
            fillet_fill_l: BorderRenderElement::default(),
            fillet_l_loc: Point::from((0., 0.)),
            has_left_bend: false,
            row_gap: 0.,
            stub: BorderRenderElement::default(),
            stub_loc: Point::from((0., 0.)),
            outline_width: 0.,
            has_fill: false,
            draw_outline: false,
            solid: SolidColorBuffer::new((0., 0.), [0., 0., 0., 0.]),
            use_shader: false,
            alpha: 1.,
            scale: 1.,
            text_color: Color::new_unpremul(0., 0., 0., 1.),
            text_loc: Point::from((0., 0.)),
            close_rect: Rectangle::default(),
            close_button: BorderRenderElement::default(),
            max_rect: Rectangle::default(),
            max_back: BorderRenderElement::default(),
            max_front: BorderRenderElement::default(),
            max_front_fill: BorderRenderElement::default(),
            max_back_loc: Point::from((0., 0.)),
            max_front_loc: Point::from((0., 0.)),
            text_max_width: 0,
            text_natural_size: Size::from((0, 0)),
            title: String::new(),
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
        self.stub.damage_all();
        self.close_button.damage_all();
        self.max_back.damage_all();
        self.max_front.damage_all();
        self.max_front_fill.damage_all();
    }

    /// Tab background rectangle relative to the tile origin, valid after update_render_elements.
    pub fn rect(&self) -> Rectangle<f64, Logical> {
        self.rect
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

    /// Tile-relative X where the next tab in a row should start.
    pub fn row_next_x(&self) -> f64 {
        self.rect.loc.x + self.rect.size.w + self.row_gap
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
        if !self.visible || !self.rect.contains(point) {
            return None;
        }

        // Buttons get a bit of slack around them so they are easy to hit.
        let slack = self.config.padding / 2.;
        let with_slack = |mut rect: Rectangle<f64, Logical>| {
            rect.loc -= Point::from((slack, slack));
            rect.size += Size::from((slack * 2., slack * 2.));
            rect
        };

        if self.config.close_button && with_slack(self.close_rect).contains(point) {
            Some(TitleTabHit::Close)
        } else if self.config.maximize_button && with_slack(self.max_rect).contains(point) {
            Some(TitleTabHit::Maximize)
        } else {
            Some(TitleTabHit::Body)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_render_elements(
        &mut self,
        visible: bool,
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
        let (color, gradient) = if is_urgent {
            (self.config.urgent_color, self.config.urgent_gradient)
        } else if is_active {
            (self.config.active_color, self.config.active_gradient)
        } else {
            (self.config.inactive_color, self.config.inactive_gradient)
        };
        let gradient = gradient
            .or_else(|| color.map(Gradient::from))
            .unwrap_or(frame.gradient);

        let text_color = if is_urgent {
            self.config.urgent_text_color
        } else if is_active {
            self.config.active_text_color
        } else {
            self.config.inactive_text_color
        };
        let fill = self.config.fill_color;
        self.has_fill = fill.is_some();
        // Like the focus ring, the outline only appears on tiles that have a frame.
        self.draw_outline = fill.is_some() && frame.frame_drawn;
        self.text_color = text_color.unwrap_or_else(|| match fill {
            // Outlined tab: the title matches the frame line.
            Some(_) if self.draw_outline => gradient.from,
            // Solid enough fill: contrast against it. Transparent fill: use the frame color.
            Some(fill) if fill.a >= 0.5 => contrasting_text_color(fill),
            Some(_) => gradient.from,
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
        let max_w_btn = if self.config.maximize_button {
            round(self.config.close_button_size)
        } else {
            0.
        };
        let trail = if self.config.maximize_button {
            button_gap + max_w_btn + button_inset
        } else {
            padding
        };

        // Bend and gap sizes are fixed per tab so a row lines up regardless of focus.
        let join_r = round(f64::from(frame.top_left_radius) * self.config.join_radius_factor);
        let line_w = round(frame.line_width);
        self.row_gap = (join_r + line_w) * 2.;

        let first_in_row = row.is_none_or(|row| row.index == 0);
        self.has_left_bend = !first_in_row;
        let x0 = match row {
            Some(row) if !first_in_row => row.x,
            _ => -outset,
        };

        let min_w = f64::max(radius * 2., lead + trail);
        let mut max_w = round(tile_width * self.config.max_width_proportion) + outset;
        if let Some(row) = row {
            let count = row.count.max(1) as f64;
            let share = (tile_width - self.row_gap * (count - 1.)) / count;
            max_w = f64::min(max_w, round(share));
        }
        let max_w = f64::max(min_w, max_w);
        let width = (natural_w + lead + trail + outset).clamp(min_w, max_w);
        let width = round(width);

        self.rect = Rectangle::new(
            Point::from((x0, -outset)),
            Size::from((width, height + outset)),
        );

        let avail_w = width - outset - lead - trail;
        self.text_max_width = (avail_w * scale).floor().max(0.) as i32;

        let text_h = self.text_natural_size.h as f64 / scale;
        self.text_loc = Point::from((
            self.rect.loc.x + outset + lead,
            self.rect.loc.y + round((self.rect.size.h - text_h) / 2.),
        ))
        .to_physical_precise_round(scale)
        .to_logical(scale);

        self.close_rect = Rectangle::new(
            Point::from((
                self.rect.loc.x + outset + left_inset,
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

        // Close button: an outlined square in the text color.
        if self.config.close_button {
            let line = f64::max(1. / scale, round(button_w / 8.));
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

        // Maximize button: two overlapping outlined squares, BeOS zoom style. The front square
        // is filled so it covers the back one.
        if self.config.maximize_button {
            let s = self.max_rect.size.w;
            let line = f64::max(1. / scale, round(s / 8.));
            let back = round(s * 0.72);
            let front = round(s * 0.58);
            self.max_back_loc = self.max_rect.loc + Point::from((s - back, 0.));
            self.max_front_loc = self.max_rect.loc + Point::from((0., s - front));
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
            let fill_color = self.config.fill_color.unwrap_or(gradient.from);
            square(&mut self.max_front_fill, front, fill_color, 0.);
        }

        // Background. Only the top corners are rounded: the bottom merges into the frame.
        let corner_radius = CornerRadius {
            top_left: radius as f32,
            top_right: radius as f32,
            bottom_right: 0.,
            bottom_left: 0.,
        }
        .fit_to(width as f32, (height + outset) as f32);
        self.use_shader = corner_radius != CornerRadius::default() || gradient.from != gradient.to;

        let gradient_area = match gradient.relative_to {
            GradientRelativeTo::Window => frame.frame_rect,
            GradientRelativeTo::WorkspaceView => frame.view_rect,
        };

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
        }
    }

    pub fn render(
        &self,
        renderer: &mut impl NiriRenderer,
        location: Point<f64, Logical>,
        push: &mut dyn FnMut(TitleTabRenderElement),
    ) {
        if !self.visible {
            return;
        }

        // Text and close button go on top.
        if self.config.close_button && self.close_rect.size.w > 0. {
            push(
                self.close_button
                    .clone()
                    .with_location(location + self.close_rect.loc)
                    .into(),
            );
        }
        if self.config.maximize_button && self.max_rect.size.w > 0. {
            push(
                self.max_front
                    .clone()
                    .with_location(location + self.max_front_loc)
                    .into(),
            );
            push(
                self.max_front_fill
                    .clone()
                    .with_location(location + self.max_front_loc)
                    .into(),
            );
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

        let has_shader = BorderRenderElement::has_shader(renderer);
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
