//! Dock visual tree: shadow, glass, specular border, dividers, icons, indicators,
//! and labels. It is built once per configuration; only its elements move per frame.

use anyhow::Result;
use docky_core::{
    layout::{ItemKind, Layout, Rect},
    magnify::Magnified,
};
use windows::{
    UI::{
        Color,
        Composition::{
            CompositionBrush, CompositionColorSpace, CompositionRoundedRectangleGeometry,
            CompositionSurfaceBrush, ContainerVisual, ShapeVisual, SpriteVisual, Visual,
        },
    },
    core::Interface,
};
use windows_numerics::{Vector2, Vector3};

use crate::gfx::{Gfx, LabelBrush};

#[derive(Clone, Copy)]
pub struct Theme {
    pub light: bool,
    /// Blur what is behind the dock; without it the tint is drawn opaque.
    glass: bool,
    tint: Color,
    separator: Color,
    shadow: Color,
    indicator: Color,
}

impl Theme {
    /// `glass` is false while Windows has transparency off (or energy saver
    /// on): the dock is then solid gray, as macOS draws it with "Reduce
    /// transparency".
    pub fn for_system(light: bool, glass: bool) -> Self {
        if light {
            Self {
                light,
                glass,
                tint: if glass {
                    rgba(255, 255, 255, 46)
                } else {
                    rgba(226, 226, 228, 255)
                },
                separator: rgba(0, 0, 0, 56),
                shadow: rgba(0, 0, 0, 56),
                indicator: rgba(0, 0, 0, 170),
            }
        } else {
            Self {
                light,
                glass,
                tint: if glass {
                    rgba(28, 28, 30, 82)
                } else {
                    rgba(44, 44, 46, 255)
                },
                separator: rgba(255, 255, 255, 64),
                shadow: rgba(0, 0, 0, 96),
                indicator: rgba(255, 255, 255, 215),
            }
        }
    }
}

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color {
        A: a,
        R: r,
        G: g,
        B: b,
    }
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn v3(x: f32, y: f32) -> Vector3 {
    Vector3 { X: x, Y: y, Z: 0.0 }
}

#[derive(Clone, Copy, Default)]
pub struct SlotContent<'a> {
    pub icon: Option<&'a CompositionSurfaceBrush>,
    pub label: Option<&'a LabelBrush>,
    pub running: bool,
    /// App icon shown in the corner of a minimized-window thumbnail.
    pub badge: Option<&'a CompositionSurfaceBrush>,
}

pub struct SceneInput<'a> {
    pub layout: &'a Layout,
    pub contents: &'a [SlotContent<'a>],
    pub theme: Theme,
    pub scale: f32,
    pub origin: (f32, f32),
    /// Shown above an icon dragged out of the dock.
    pub remove_label: Option<&'a LabelBrush>,
}

/// App badge on a minimized-window thumbnail: side relative to the slot, and
/// how much of it lies inside the slot.
pub const BADGE_SIZE: f32 = 0.42;
pub const BADGE_INSIDE: f32 = 0.85;

/// An icon following the cursor while it is dragged.
#[derive(Clone, Copy)]
pub struct Drag {
    pub index: usize,
    /// Icon center, relative to the resting dock origin.
    pub center: (f32, f32),
    /// Released here, the icon would leave the dock.
    pub removing: bool,
}

/// Everything that changes from one frame to the next.
pub struct Frame<'a> {
    pub state: &'a Magnified,
    pub hovered: Option<usize>,
    pub lifts: &'a [f32],
    pub drag: Option<Drag>,
    /// Vertical offset of the whole dock (auto-hide), in pixels.
    pub slide: f32,
    /// Align to whole pixels; only at rest, where sharpness matters.
    pub snap: bool,
}

struct Label {
    visual: SpriteVisual,
    width: f32,
    height: f32,
}

impl Label {
    fn new(c: &windows::UI::Composition::Compositor, brush: &LabelBrush) -> Result<Self> {
        let visual = c.CreateSpriteVisual()?;
        visual.SetSize(v2(brush.width, brush.height))?;
        visual.SetBrush(&brush.brush)?;
        visual.SetIsVisible(false)?;
        Ok(Self {
            visual,
            width: brush.width,
            height: brush.height,
        })
    }
}

pub struct Scene {
    pub root: ContainerVisual,
    origin: (f32, f32),
    height: f32,
    stroke: f32,
    label_gap: f32,
    indicator_size: f32,
    indicator_y: f32,
    frames: [Visual; 3],
    outline: CompositionRoundedRectangleGeometry,
    rim_geometry: CompositionRoundedRectangleGeometry,
    slots: Vec<SpriteVisual>,
    badges: Vec<Option<SpriteVisual>>,
    indicators: Vec<Option<ShapeVisual>>,
    labels: Vec<Option<Label>>,
    remove_label: Option<Label>,
}

impl Scene {
    pub fn build(gfx: &Gfx, input: &SceneInput<'_>) -> Result<Self> {
        let c = &gfx.compositor;
        let SceneInput {
            layout,
            theme,
            scale,
            origin,
            ..
        } = *input;
        let radius = layout.height * 0.36;

        let root = c.CreateContainerVisual()?;
        root.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        let layers = root.Children()?;

        let shadow = c.CreateDropShadow()?;
        let mask = c.CreateNineGridBrush()?;
        mask.SetSource(&gfx.rounded_mask(radius)?)?;
        mask.SetInsets(radius.ceil() + 1.0)?;
        shadow.SetMask(&mask)?;
        shadow.SetBlurRadius(22.0 * scale)?;
        shadow.SetOffset(v3(0.0, 8.0 * scale))?;
        shadow.SetColor(theme.shadow)?;
        let shadow_host = c.CreateSpriteVisual()?;
        shadow_host.SetShadow(&shadow)?;
        layers.InsertAtTop(&shadow_host)?;

        let glass = c.CreateContainerVisual()?;
        let outline = c.CreateRoundedRectangleGeometry()?;
        outline.SetCornerRadius(v2(radius, radius))?;
        glass.SetClip(&c.CreateGeometricClipWithGeometry(&outline)?)?;

        let sheen = c.CreateLinearGradientBrush()?;
        sheen.SetStartPoint(v2(0.0, 0.0))?;
        sheen.SetEndPoint(v2(0.0, 1.0))?;
        let stops = sheen.ColorStops()?;
        stops
            .Append(&c.CreateColorGradientStopWithOffsetAndColor(0.0, rgba(255, 255, 255, 38))?)?;
        stops
            .Append(&c.CreateColorGradientStopWithOffsetAndColor(0.55, rgba(255, 255, 255, 0))?)?;

        let backdrop: Option<CompositionBrush> = if theme.glass {
            Some(c.CreateHostBackdropBrush()?.cast()?)
        } else {
            None
        };
        let fills = [
            c.CreateColorBrushWithColor(theme.tint)?.cast()?,
            sheen.cast()?,
        ];
        for brush in backdrop.iter().chain(&fills) {
            let fill = c.CreateSpriteVisual()?;
            fill.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
            fill.SetBrush(brush)?;
            glass.Children()?.InsertAtTop(&fill)?;
        }
        layers.InsertAtTop(&glass)?;

        let stroke = scale.max(1.0);
        let rim_brush = c.CreateLinearGradientBrush()?;
        rim_brush.SetStartPoint(v2(0.0, 0.0))?;
        rim_brush.SetEndPoint(v2(1.0, 1.0))?;
        rim_brush.SetInterpolationSpace(CompositionColorSpace::RgbLinear)?;
        let stops = rim_brush.ColorStops()?;
        for (at, alpha) in [(0.0, 150), (0.35, 40), (0.65, 30), (1.0, 105)] {
            stops.Append(
                &c.CreateColorGradientStopWithOffsetAndColor(at, rgba(255, 255, 255, alpha))?,
            )?;
        }
        let rim_geometry = c.CreateRoundedRectangleGeometry()?;
        rim_geometry.SetOffset(v2(stroke / 2.0, stroke / 2.0))?;
        rim_geometry.SetCornerRadius(v2(radius - stroke / 2.0, radius - stroke / 2.0))?;
        let rim_shape = c.CreateSpriteShapeWithGeometry(&rim_geometry)?;
        rim_shape.SetStrokeBrush(&rim_brush)?;
        rim_shape.SetStrokeThickness(stroke)?;
        let rim = c.CreateShapeVisual()?;
        rim.Shapes()?.Append(&rim_shape)?;
        layers.InsertAtTop(&rim)?;

        let content = c.CreateContainerVisual()?;
        content.SetOffset(v3(origin.0, origin.1))?;
        let children = content.Children()?;
        let indicator_size = (4.0 * scale).round();
        let dot = c.CreateEllipseGeometry()?;
        dot.SetCenter(v2(indicator_size / 2.0, indicator_size / 2.0))?;
        dot.SetRadius(v2(indicator_size / 2.0, indicator_size / 2.0))?;
        let dot_brush = c.CreateColorBrushWithColor(theme.indicator)?;

        let mut slots = Vec::with_capacity(layout.slots.len());
        let mut badges = Vec::with_capacity(layout.slots.len());
        let mut indicators = Vec::with_capacity(layout.slots.len());
        let mut labels = Vec::with_capacity(layout.slots.len());
        for (slot, item) in layout.slots.iter().zip(input.contents) {
            let visual = c.CreateSpriteVisual()?;
            match (slot.kind, item.icon) {
                (ItemKind::Separator, _) => {
                    visual.SetBrush(&c.CreateColorBrushWithColor(theme.separator)?)?;
                }
                (ItemKind::Icon, Some(icon)) => visual.SetBrush(icon)?,
                (ItemKind::Icon, None) => {}
            }
            children.InsertAtTop(&visual)?;
            slots.push(visual);
            badges.push(match item.badge {
                Some(brush) => {
                    let badge = c.CreateSpriteVisual()?;
                    badge.SetBrush(brush)?;
                    children.InsertAtTop(&badge)?;
                    Some(badge)
                }
                None => None,
            });

            indicators.push(if item.running {
                let shape = c.CreateSpriteShapeWithGeometry(&dot)?;
                shape.SetFillBrush(&dot_brush)?;
                let visual = c.CreateShapeVisual()?;
                visual.SetSize(v2(indicator_size, indicator_size))?;
                visual.Shapes()?.Append(&shape)?;
                children.InsertAtTop(&visual)?;
                Some(visual)
            } else {
                None
            });

            labels.push(item.label.map(|label| Label::new(c, label)).transpose()?);
        }

        let remove_label = input
            .remove_label
            .map(|label| Label::new(c, label))
            .transpose()?;
        for label in labels.iter().flatten().chain(&remove_label) {
            children.InsertAtTop(&label.visual)?;
        }
        layers.InsertAtTop(&content)?;

        Ok(Self {
            root,
            origin,
            height: layout.height,
            stroke,
            label_gap: 8.0 * scale,
            indicator_size,
            indicator_y: layout.height - indicator_size - (2.0 * scale).round(),
            frames: [shadow_host.cast()?, glass.cast()?, rim.cast()?],
            outline,
            rim_geometry,
            slots,
            badges,
            indicators,
            labels,
            remove_label,
        })
    }

    /// - `lifts`: how far each slot rises above its resting position (bounce).
    /// - `snap`: whether to align to pixels. Used at rest for sharpness; fractional
    ///   positions are retained during movement for smooth animation.
    pub fn apply(&self, frame: &Frame<'_>) -> Result<()> {
        let Frame {
            state,
            hovered,
            lifts,
            drag,
            slide,
            snap,
        } = *frame;
        let fit = |value: f32| if snap { value.round() } else { value };
        self.root.SetOffset(v3(0.0, slide))?;

        let size = v2(fit(state.right - state.left), self.height);
        let offset = v3(self.origin.0 + fit(state.left), self.origin.1);
        for frame in &self.frames {
            frame.SetSize(size)?;
            frame.SetOffset(offset)?;
        }
        self.outline.SetSize(size)?;
        self.rim_geometry
            .SetSize(v2(size.X - self.stroke, size.Y - self.stroke))?;

        for (index, rect) in state.rects.iter().enumerate() {
            let Rect {
                x,
                y,
                width,
                height,
            } = *rect;
            let dragged = drag.filter(|drag| drag.index == index);
            let (x, y) = match dragged {
                Some(drag) => (drag.center.0 - width / 2.0, drag.center.1 - height / 2.0),
                None => (x, y - lifts.get(index).copied().unwrap_or(0.0)),
            };

            if let Some(visual) = self.slots.get(index) {
                visual.SetOffset(v3(fit(x), fit(y)))?;
                visual.SetSize(v2(fit(width), fit(height)))?;
                let removing = dragged.is_some_and(|drag| drag.removing);
                visual.SetOpacity(if removing { 0.55 } else { 1.0 })?;
            }
            if let Some(Some(badge)) = self.badges.get(index) {
                let side = width * BADGE_SIZE;
                badge.SetSize(v2(fit(side), fit(side)))?;
                badge.SetOffset(v3(
                    fit(x + width - side * BADGE_INSIDE),
                    fit(y + height - side * BADGE_INSIDE),
                ))?;
            }
            if let Some(Some(indicator)) = self.indicators.get(index) {
                indicator.SetIsVisible(dragged.is_none())?;
                indicator.SetOffset(v3(
                    fit(x + (width - self.indicator_size) / 2.0),
                    self.indicator_y,
                ))?;
            }
            if let Some(Some(label)) = self.labels.get(index) {
                let shown = hovered == Some(index) && drag.is_none();
                label.visual.SetIsVisible(shown)?;
                if shown {
                    self.place_label(label, x, y, width)?;
                }
            }
            if let (Some(label), Some(drag)) = (&self.remove_label, dragged) {
                label.visual.SetIsVisible(drag.removing)?;
                if drag.removing {
                    self.place_label(label, x, y, width)?;
                }
            }
        }
        if drag.is_none()
            && let Some(label) = &self.remove_label
        {
            label.visual.SetIsVisible(false)?;
        }
        Ok(())
    }

    fn place_label(&self, label: &Label, x: f32, y: f32, width: f32) -> Result<()> {
        label.visual.SetOffset(v3(
            (x + (width - label.width) / 2.0).round(),
            (y - self.label_gap - label.height).round(),
        ))?;
        Ok(())
    }
}
