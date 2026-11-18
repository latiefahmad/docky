//! Graphics device connecting the Windows compositor to Direct2D, allowing
//! icons and masks to be drawn on compositor surfaces.

use anyhow::{Context, Result};
use docky_shell::icons::IconImage;
use windows::{
    Foundation::Size,
    Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat},
    UI::{
        Color,
        Composition::{
            CompositionDrawingSurface, CompositionGraphicsDevice, CompositionSurfaceBrush,
            Compositor,
        },
    },
    Win32::{
        Foundation::{HMODULE, POINT},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT,
                    D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
                    D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
                },
                D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1,
                D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
                D2D1_COLOR_SPACE_SRGB, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
                D2D1_EXTEND_MODE_CLAMP, D2D1_INTERPOLATION_MODE,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
                D2D1_PRIMITIVE_BLEND_COPY, D2D1_PRIMITIVE_BLEND_SOURCE_OVER, D2D1_ROUNDED_RECT,
                D2D1CreateDevice, ID2D1Device, ID2D1DeviceContext,
            },
            Direct3D::D3D_DRIVER_TYPE_WARP,
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_SINGLETHREADED,
                D3D11_SDK_VERSION, D3D11CreateDevice,
            },
            DirectWrite::{
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, DWriteCreateFactory,
                IDWriteFactory,
            },
            Dxgi::{Common::DXGI_FORMAT_B8G8R8A8_UNORM, IDXGIDevice},
        },
        System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorInterop},
    },
    core::{Interface, w},
};
use windows_numerics::{Matrix3x2, Vector2};

const TRANSPARENT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};
const WHITE: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

pub struct LabelBrush {
    pub brush: CompositionSurfaceBrush,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone)]
pub struct Gfx {
    pub compositor: Compositor,
    graphics: CompositionGraphicsDevice,
    // Surfaces depend on this device, so it must outlive them.
    _d2d: ID2D1Device,
    dwrite: IDWriteFactory,
}

impl Gfx {
    pub fn new() -> Result<Self> {
        let compositor = Compositor::new()?;
        // SAFETY: D3D writes only to the initialized output slot; the returned
        // device is immediately wrapped in an owned COM interface.
        let (d2d, graphics) = unsafe {
            let mut d3d = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_SINGLETHREADED,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
            let dxgi: IDXGIDevice = d3d.context("Direct3D did not return a device")?.cast()?;
            let d2d = D2D1CreateDevice(&dxgi, None)?;
            let graphics = compositor
                .cast::<ICompositorInterop>()?
                .CreateGraphicsDevice(&d2d)?;
            (d2d, graphics)
        };
        Ok(Self {
            compositor,
            graphics,
            _d2d: d2d,
            dwrite: unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? },
        })
    }

    /// Creates a `width` × `height` pixel surface, draws on it, and returns it
    /// as a brush for a visual.
    fn draw(
        &self,
        width: u32,
        height: u32,
        paint: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
    ) -> Result<CompositionSurfaceBrush> {
        let surface = self.draw_surface(width, height, paint)?;
        Ok(self.compositor.CreateSurfaceBrushWithSurface(&surface)?)
    }

    fn draw_surface(
        &self,
        width: u32,
        height: u32,
        paint: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
    ) -> Result<CompositionDrawingSurface> {
        let surface = self.graphics.CreateDrawingSurface(
            Size {
                Width: width as f32,
                Height: height as f32,
            },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )?;
        let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;

        // SAFETY: BeginDraw and EndDraw are paired, and `offset` lives through the call.
        unsafe {
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
            // The surface may be part of an atlas, so draw at the specified offset.
            dc.SetDpi(96.0, 96.0);
            dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            dc.Clear(Some(&TRANSPARENT));
            let painted = paint(&dc);
            interop.EndDraw()?;
            painted?;
        }
        Ok(surface)
    }

    pub fn icon_brush(&self, icon: &IconImage, side: u32) -> Result<CompositionSurfaceBrush> {
        self.draw(side, side, |dc| {
            let quality = D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC;
            draw_image(dc, icon, side as f32, side as f32, quality)
        })
    }

    /// A surface holding `image` at its own size, to be shared by many brushes.
    pub fn image_surface(&self, image: &IconImage) -> Result<CompositionDrawingSurface> {
        self.draw_surface(image.width, image.height, |dc| {
            // Drawn 1:1, so there is nothing to interpolate.
            let (width, height) = (image.width as f32, image.height as f32);
            draw_image(
                dc,
                image,
                width,
                height,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            )
        })
    }

    /// Creates a minimal opaque rounded rectangle (side `2·radius + 2`). A
    /// *nine-grid* brush stretches it to any width without repainting or
    /// distorting its corners during magnification.
    pub fn rounded_mask(&self, radius: f32) -> Result<CompositionSurfaceBrush> {
        let side = (2.0 * radius).ceil() + 2.0;
        self.draw(side as u32, side as u32, |dc| {
            let shape = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: side,
                    bottom: side,
                },
                radiusX: radius,
                radiusY: radius,
            };
            // SAFETY: The brush and geometry remain alive during the call.
            unsafe {
                let brush = dc.CreateSolidColorBrush(&WHITE, None)?;
                dc.FillRoundedRectangle(&shape, &brush);
            }
            Ok(())
        })
    }

    /// Opaque island silhouette with curved shoulders at the screen edge.
    pub fn island_mask(
        &self,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<CompositionSurfaceBrush> {
        self.draw(width.ceil() as u32, height.ceil() as u32, |dc| {
            let shoulder = 12.0 * scale;
            let corner = 22.0 * scale;
            // SAFETY: The geometry, sink and brush stay alive through the drawing calls.
            unsafe {
                let shape = dc.GetFactory()?.CreatePathGeometry()?;
                let sink = shape.Open()?;
                sink.BeginFigure(Vector2 { X: 0.0, Y: 0.0 }, D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLine(Vector2 { X: width, Y: 0.0 });
                sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                    point1: Vector2 {
                        X: width - shoulder,
                        Y: 0.0,
                    },
                    point2: Vector2 {
                        X: width - shoulder,
                        Y: shoulder * 0.7,
                    },
                    point3: Vector2 {
                        X: width - shoulder,
                        Y: shoulder,
                    },
                });
                sink.AddLine(Vector2 {
                    X: width - shoulder,
                    Y: height - corner,
                });
                sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                    point1: Vector2 {
                        X: width - shoulder,
                        Y: height - corner * 0.35,
                    },
                    point2: Vector2 {
                        X: width - shoulder - corner * 0.35,
                        Y: height,
                    },
                    point3: Vector2 {
                        X: width - shoulder - corner,
                        Y: height,
                    },
                });
                sink.AddLine(Vector2 {
                    X: shoulder + corner,
                    Y: height,
                });
                sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                    point1: Vector2 {
                        X: shoulder + corner * 0.35,
                        Y: height,
                    },
                    point2: Vector2 {
                        X: shoulder,
                        Y: height - corner * 0.35,
                    },
                    point3: Vector2 {
                        X: shoulder,
                        Y: height - corner,
                    },
                });
                sink.AddLine(Vector2 {
                    X: shoulder,
                    Y: shoulder,
                });
                sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                    point1: Vector2 {
                        X: shoulder,
                        Y: shoulder * 0.7,
                    },
                    point2: Vector2 {
                        X: shoulder,
                        Y: 0.0,
                    },
                    point3: Vector2 { X: 0.0, Y: 0.0 },
                });
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                sink.Close()?;
                let white = dc.CreateSolidColorBrush(&WHITE, None)?;
                dc.FillGeometry(&shape, &white, None);
            }
            Ok(())
        })
    }

    pub fn usage_ring(
        &self,
        used_percent: Option<u8>,
        scale: f32,
        accent: Color,
    ) -> Result<LabelBrush> {
        let side = (18.0 * scale).round();
        let center = side / 2.0;
        let radius = center - 2.0 * scale;
        let stroke = (2.4 * scale).max(1.0);
        let brush = self.draw(side as u32, side as u32, |dc| {
            // SAFETY: Brushes remain alive for the drawing calls.
            unsafe {
                let track = dc.CreateSolidColorBrush(&rgb(0.53, 0.56, 0.6, 0.4), None)?;
                dc.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: Vector2 {
                            X: center,
                            Y: center,
                        },
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &track,
                    stroke,
                    None,
                );
                if let Some(percent) = used_percent {
                    let foreground = dc.CreateSolidColorBrush(
                        &rgb(
                            f32::from(accent.R) / 255.0,
                            f32::from(accent.G) / 255.0,
                            f32::from(accent.B) / 255.0,
                            1.0,
                        ),
                        None,
                    )?;
                    let steps = usize::from(percent) * 36 / 100;
                    for index in 0..steps {
                        let start = -std::f32::consts::FRAC_PI_2
                            + index as f32 * std::f32::consts::TAU / 36.0;
                        let end = start + std::f32::consts::TAU / 36.0;
                        let a = Vector2 {
                            X: center + radius * start.cos(),
                            Y: center + radius * start.sin(),
                        };
                        let b = Vector2 {
                            X: center + radius * end.cos(),
                            Y: center + radius * end.sin(),
                        };
                        dc.DrawLine(a, b, &foreground, stroke, None);
                    }
                }
            }
            Ok(())
        })?;
        Ok(LabelBrush {
            brush,
            width: side,
            height: side,
        })
    }

    /// Creates an app-name label: a dark pill (light in light theme) with a
    /// downward-pointing arrow.
    pub fn label_brush(&self, text: &str, scale: f32, light: bool) -> Result<LabelBrush> {
        let (pad_x, pad_y, arrow) = (11.0 * scale, 5.0 * scale, 6.0 * scale);
        let (fill, ink) = if light {
            (rgb(0.96, 0.96, 0.97, 0.92), rgb(0.0, 0.0, 0.0, 0.88))
        } else {
            (rgb(0.13, 0.13, 0.14, 0.86), rgb(1.0, 1.0, 1.0, 0.95))
        };

        let utf16: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: Strings and structures passed to each call remain alive for its duration.
        let (layout, metrics) = unsafe {
            let format = self.dwrite.CreateTextFormat(
                text_font(),
                None,
                DWRITE_FONT_WEIGHT_MEDIUM,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                13.0 * scale,
                w!(""),
            )?;
            let layout = self
                .dwrite
                .CreateTextLayout(&utf16, &format, 4096.0, 256.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            (layout, metrics)
        };

        let body_height = (metrics.height + 2.0 * pad_y).ceil();
        let width = (metrics.width + 2.0 * pad_x).ceil();
        let height = body_height + arrow;
        let brush = self.draw(width as u32, height as u32, |dc| {
            let body = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: width,
                    bottom: body_height,
                },
                radiusX: 7.0 * scale,
                radiusY: 7.0 * scale,
            };
            let middle = width / 2.0;
            // SAFETY: All created objects remain alive until the end of the block.
            unsafe {
                let fill = dc.CreateSolidColorBrush(&fill, None)?;
                dc.FillRoundedRectangle(&body, &fill);

                // The arrow triangle hangs from the center of the pill and
                // overlaps it by 1 px to avoid a visible seam.
                let triangle = dc.GetFactory()?.CreatePathGeometry()?;
                let sink = triangle.Open()?;
                sink.BeginFigure(
                    Vector2 {
                        X: middle - arrow,
                        Y: body_height - 1.0,
                    },
                    D2D1_FIGURE_BEGIN_FILLED,
                );
                sink.AddLine(Vector2 {
                    X: middle + arrow,
                    Y: body_height - 1.0,
                });
                sink.AddLine(Vector2 {
                    X: middle,
                    Y: height,
                });
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                sink.Close()?;
                dc.FillGeometry(&triangle, &fill, None);

                let ink = dc.CreateSolidColorBrush(&ink, None)?;
                dc.DrawTextLayout(
                    Vector2 { X: pad_x, Y: pad_y },
                    &layout,
                    &ink,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            Ok(())
        })?;
        Ok(LabelBrush {
            brush,
            width,
            height,
        })
    }

    /// Shared temporary app tile while a shell icon is loading or unavailable.
    pub fn placeholder_brush(&self, side: u32, light: bool) -> Result<CompositionSurfaceBrush> {
        let size = side as f32;
        self.draw(side, side, |dc| {
            let tile = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: size * 0.08,
                    top: size * 0.08,
                    right: size * 0.92,
                    bottom: size * 0.92,
                },
                radiusX: size * 0.18,
                radiusY: size * 0.18,
            };
            let shade = if light { 0.72 } else { 0.36 };
            // SAFETY: Geometry and brushes remain valid for these drawing calls.
            unsafe {
                let fill = dc.CreateSolidColorBrush(&rgb(shade, shade, shade, 1.0), None)?;
                dc.FillRoundedRectangle(&tile, &fill);
                let ink = dc.CreateSolidColorBrush(&rgb(1.0, 1.0, 1.0, 0.7), None)?;
                dc.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: size * 0.27,
                            top: size * 0.28,
                            right: size * 0.73,
                            bottom: size * 0.72,
                        },
                        radiusX: size * 0.04,
                        radiusY: size * 0.04,
                    },
                    &ink,
                    (size * 0.035).max(1.0),
                    None,
                );
            }
            Ok(())
        })
    }

    /// Start tile: a 3x3 grid of rounded squares on a blue gradient, in the
    /// style of the other dock icons (no Windows logo: it is a trademark).
    pub fn start_brush(&self, side: u32) -> Result<CompositionSurfaceBrush> {
        let size = side as f32;
        self.draw(side, side, |dc| {
            // Same margin and corner ratio as a macOS app icon.
            let inset = size * 0.08;
            let tile = D2D_RECT_F {
                left: inset,
                top: inset,
                right: size - inset,
                bottom: size - inset,
            };
            let radius = (size - 2.0 * inset) * 0.225;
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: rgb(0.40, 0.80, 1.0, 1.0),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: rgb(0.04, 0.42, 0.95, 1.0),
                },
            ];
            // SAFETY: Brushes and geometry live for the duration of the drawing calls.
            unsafe {
                let collection = dc.CreateGradientStopCollection(
                    &stops,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_BUFFER_PRECISION_8BPC_UNORM,
                    D2D1_EXTEND_MODE_CLAMP,
                    D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
                )?;
                let gradient = dc.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2 {
                            X: 0.0,
                            Y: tile.top,
                        },
                        endPoint: Vector2 {
                            X: 0.0,
                            Y: tile.bottom,
                        },
                    },
                    None,
                    &collection,
                )?;
                dc.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: tile,
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &gradient,
                );

                // The Windows logo: four square panes split by a thin gap
                // (0.85 of 18.5 units in its SVG), on whole pixels so the
                // edges stay sharp at every icon size.
                let white = dc.CreateSolidColorBrush(&rgb(1.0, 1.0, 1.0, 0.97), None)?;
                let logo = (size * 0.44).round();
                let gap = (logo * 0.046).round().max(1.0);
                let pane = ((logo - gap) / 2.0).floor();
                let origin = ((size - (2.0 * pane + gap)) / 2.0).round();
                for row in 0..2 {
                    for column in 0..2 {
                        let left = origin + column as f32 * (pane + gap);
                        let top = origin + row as f32 * (pane + gap);
                        dc.FillRectangle(
                            &D2D_RECT_F {
                                left,
                                top,
                                right: left + pane,
                                bottom: top + pane,
                            },
                            &white,
                        );
                    }
                }
            }
            Ok(())
        })
    }

    /// Plain text on a transparent background (menu bar).
    pub fn text_brush(&self, text: &str, scale: f32, bold: bool, ink: Ink) -> Result<LabelBrush> {
        self.text_brush_sized(text, scale, 13.0, bold, ink)
    }

    /// Text at a chosen logical font size for the expanded usage panel.
    pub fn text_brush_sized(
        &self,
        text: &str,
        scale: f32,
        font_size: f32,
        bold: bool,
        ink: Ink,
    ) -> Result<LabelBrush> {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let weight = if bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        // SAFETY: Strings and structures passed to each call remain alive for its duration.
        let (layout, metrics) = unsafe {
            let format = self.dwrite.CreateTextFormat(
                text_font(),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font_size * scale,
                w!(""),
            )?;
            let layout = self
                .dwrite
                .CreateTextLayout(&utf16, &format, 4096.0, 256.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            (layout, metrics)
        };
        let (width, height) = (
            metrics.width.ceil().max(1.0),
            metrics.height.ceil().max(1.0),
        );
        let brush = self.draw(width as u32, height as u32, |dc| {
            // SAFETY: The brush and layout live for the drawing call.
            unsafe {
                let ink = dc.CreateSolidColorBrush(&ink.color(), None)?;
                dc.DrawTextLayout(
                    Vector2 { X: 0.0, Y: 0.0 },
                    &layout,
                    &ink,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            Ok(())
        })?;
        Ok(LabelBrush {
            brush,
            width,
            height,
        })
    }

    /// Symbols from the system icon font (see `icon_font`), centered
    /// in a square. Layers are drawn in order with their opacity, so a dim full
    /// Wi-Fi fan can sit behind the bars actually received.
    pub fn glyph_brush(&self, layers: &[(char, f32)], scale: f32, ink: Ink) -> Result<LabelBrush> {
        let side = (18.0 * scale).round();
        // SAFETY: The format lives for the drawing below.
        let format = unsafe {
            let format = self.dwrite.CreateTextFormat(
                icon_font(),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                15.0 * scale,
                w!(""),
            )?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format
        };
        let brush = self.draw(side as u32, side as u32, |dc| {
            let area = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: side,
                bottom: side,
            };
            // SAFETY: The brush, format and text live for each drawing call.
            unsafe {
                let ink = dc.CreateSolidColorBrush(&ink.color(), None)?;
                for &(glyph, opacity) in layers {
                    let mut units = [0u16; 2];
                    let text = glyph.encode_utf16(&mut units);
                    ink.SetOpacity(opacity);
                    dc.DrawText(
                        text,
                        &format,
                        &area,
                        &ink,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
            }
            Ok(())
        })?;
        Ok(LabelBrush {
            brush,
            width: side,
            height: side,
        })
    }

    /// Two switches, one on and one off: the control center glyph.
    pub fn control_center_brush(&self, scale: f32, ink: Ink) -> Result<LabelBrush> {
        let (width, height) = ((19.0 * scale).round(), (16.0 * scale).round());
        let brush = self.draw(width as u32, height as u32, |dc| {
            let pill_height = height * 0.44;
            let radius = pill_height / 2.0;
            let stroke = (1.2 * scale).max(1.0);
            // SAFETY: The brush lives for the drawing calls.
            unsafe {
                let ink = dc.CreateSolidColorBrush(&ink.color(), None)?;
                for (row, knob_right) in [(0.0, true), (height - pill_height, false)] {
                    let pill = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: stroke / 2.0,
                            top: row + stroke / 2.0,
                            right: width - stroke / 2.0,
                            bottom: row + pill_height - stroke / 2.0,
                        },
                        radiusX: radius,
                        radiusY: radius,
                    };
                    dc.DrawRoundedRectangle(&pill, &ink, stroke, None);
                    let center = if knob_right { width - radius } else { radius };
                    dc.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: Vector2 {
                                X: center,
                                Y: row + radius,
                            },
                            radiusX: radius - stroke * 0.3,
                            radiusY: radius - stroke * 0.3,
                        },
                        &ink,
                    );
                }
            }
            Ok(())
        })?;
        Ok(LabelBrush {
            brush,
            width,
            height,
        })
    }

    /// Battery outline filled to `percent`, green while charging.
    pub fn battery_brush(
        &self,
        percent: u8,
        charging: bool,
        scale: f32,
        ink: Ink,
    ) -> Result<LabelBrush> {
        let (width, height) = ((26.0 * scale).round(), (12.0 * scale).round());
        let brush = self.draw(width as u32, height as u32, |dc| {
            let stroke = (1.0 * scale).max(1.0);
            let cap = 2.0 * scale;
            let body_right = width - cap - stroke;
            let level = if charging {
                rgb(0.20, 0.78, 0.35, 1.0)
            } else if percent <= 20 {
                rgb(1.0, 0.27, 0.23, 1.0)
            } else {
                ink.color()
            };
            // SAFETY: The brushes live for the drawing calls.
            unsafe {
                let outline = dc.CreateSolidColorBrush(&ink.faded(), None)?;
                let fill = dc.CreateSolidColorBrush(&level, None)?;
                dc.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: stroke / 2.0,
                            top: stroke / 2.0,
                            right: body_right,
                            bottom: height - stroke / 2.0,
                        },
                        radiusX: 3.0 * scale,
                        radiusY: 3.0 * scale,
                    },
                    &outline,
                    stroke,
                    None,
                );
                dc.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: body_right + stroke,
                            top: height * 0.32,
                            right: width,
                            bottom: height * 0.68,
                        },
                        radiusX: scale,
                        radiusY: scale,
                    },
                    &outline,
                );
                let inset = stroke * 2.0;
                let room = body_right - 2.0 * inset;
                dc.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: inset,
                            top: inset,
                            right: inset + room * f32::from(percent.min(100)) / 100.0,
                            bottom: height - inset,
                        },
                        radiusX: 1.5 * scale,
                        radiusY: 1.5 * scale,
                    },
                    &fill,
                );
                if charging {
                    // A bolt over the middle of the body, as on a Mac, with a
                    // clear gap cut around it so it reads over the level.
                    let bolt = dc.GetFactory()?.CreatePathGeometry()?;
                    let sink = bolt.Open()?;
                    let (bolt_width, bolt_height) = (height * 0.62, height * 1.04);
                    let left = (body_right - bolt_width) / 2.0;
                    let top = (height - bolt_height) / 2.0;
                    let at = |x: f32, y: f32| Vector2 {
                        X: left + x * bolt_width,
                        Y: top + y * bolt_height,
                    };
                    sink.BeginFigure(at(0.62, 0.0), D2D1_FIGURE_BEGIN_FILLED);
                    for (x, y) in [
                        (0.08, 0.58),
                        (0.46, 0.58),
                        (0.36, 1.0),
                        (0.92, 0.40),
                        (0.54, 0.40),
                    ] {
                        sink.AddLine(at(x, y));
                    }
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    sink.Close()?;
                    let clear = dc.CreateSolidColorBrush(&TRANSPARENT, None)?;
                    dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_COPY);
                    dc.DrawGeometry(&bolt, &clear, 2.0 * stroke, None);
                    dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
                    let ink = dc.CreateSolidColorBrush(&ink.color(), None)?;
                    dc.FillGeometry(&bolt, &ink, None);
                }
            }
            Ok(())
        })?;
        Ok(LabelBrush {
            brush,
            width,
            height,
        })
    }
}

fn draw_image(
    dc: &ID2D1DeviceContext,
    image: &IconImage,
    width: f32,
    height: f32,
    interpolation: D2D1_INTERPOLATION_MODE,
) -> Result<()> {
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..Default::default()
    };
    let dest = D2D_RECT_F {
        left: 0.0,
        top: 0.0,
        right: width,
        bottom: height,
    };
    // SAFETY: `image.bgra` contains `width * height * 4` bytes and the stride is `width * 4`.
    unsafe {
        let bitmap = dc.CreateBitmap(
            D2D_SIZE_U {
                width: image.width,
                height: image.height,
            },
            Some(image.bgra.as_ptr().cast()),
            image.width * 4,
            &properties,
        )?;
        dc.DrawBitmap(&bitmap, Some(&dest), 1.0, interpolation, None, None);
    }
    Ok(())
}

/// The interface font: Segoe UI Variable on Windows 11, Segoe UI on
/// Windows 10, which does not have it.
fn text_font() -> windows::core::PCWSTR {
    if docky_shell::system::is_windows_11() {
        w!("Segoe UI Variable Text")
    } else {
        w!("Segoe UI")
    }
}

/// The system icon font. Windows 10 has Segoe MDL2 Assets instead of Segoe
/// Fluent Icons; the symbols the menu bar uses share the same code points.
fn icon_font() -> windows::core::PCWSTR {
    if docky_shell::system::is_windows_11() {
        w!("Segoe Fluent Icons")
    } else {
        w!("Segoe MDL2 Assets")
    }
}

const fn rgb(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

/// Foreground color of menu bar content, by theme.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// Dark content for a light bar.
    Dark,
    /// Light content for a dark bar.
    Light,
}

impl Ink {
    fn color(self) -> D2D1_COLOR_F {
        match self {
            Self::Dark => rgb(0.0, 0.0, 0.0, 0.86),
            Self::Light => rgb(1.0, 1.0, 1.0, 0.94),
        }
    }

    fn faded(self) -> D2D1_COLOR_F {
        match self {
            Self::Dark => rgb(0.0, 0.0, 0.0, 0.5),
            Self::Light => rgb(1.0, 1.0, 1.0, 0.55),
        }
    }
}
