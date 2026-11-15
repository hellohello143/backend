//! Shared visual language for the desktop app and standalone updater.
use eframe::egui::epaint::Shadow;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Margin, Painter, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};
use std::sync::Arc;

const COCOA: Color32 = Color32::from_rgb(52, 34, 31);
const MILK: Color32 = Color32::from_rgb(255, 248, 241);
const JAM: Color32 = Color32::from_rgb(140, 45, 71);
const JAM_PRESSED: Color32 = Color32::from_rgb(105, 32, 54);
const BLUSH: Color32 = Color32::from_rgb(246, 207, 218);
const PISTACHIO: Color32 = Color32::from_rgb(193, 211, 167);
const CORNER_CONTROL: u8 = 8;
const CORNER_SURFACE: u8 = 14;
const FAST_MOTION: f32 = 0.08;
const TAB_MOTION: f32 = 0.16;

pub struct Palette {
    pub bg: Color32,
    pub paper: Color32,
    pub line: Color32,
    pub secondary: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub focus: Color32,
    track: Color32,
    hover: Color32,
    pressed: Color32,
    raised: Color32,
    shadow: Color32,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                bg: Color32::from_rgb(40, 28, 30),
                paper: Color32::from_rgb(55, 38, 42),
                line: Color32::from_rgb(101, 70, 77),
                secondary: Color32::from_rgb(72, 46, 53),
                ink: MILK,
                muted: Color32::from_rgb(213, 181, 184),
                accent: Color32::from_rgb(245, 170, 190),
                focus: Color32::from_rgb(255, 203, 142),
                track: Color32::from_rgb(32, 22, 24),
                hover: Color32::from_rgb(59, 41, 46),
                pressed: Color32::from_rgb(83, 53, 63),
                raised: Color32::from_rgb(65, 45, 50),
                shadow: Color32::from_black_alpha(55),
            }
        } else {
            Self {
                bg: Color32::from_rgb(250, 246, 240),
                paper: Color32::from_rgb(255, 252, 248),
                line: Color32::from_rgb(228, 212, 205),
                secondary: Color32::from_rgb(247, 228, 231),
                ink: COCOA,
                muted: Color32::from_rgb(107, 79, 76),
                accent: JAM,
                focus: Color32::from_rgb(130, 67, 146),
                track: Color32::from_rgb(243, 235, 228),
                hover: Color32::from_rgb(244, 234, 228),
                pressed: Color32::from_rgb(242, 222, 225),
                raised: Color32::from_rgb(255, 253, 250),
                shadow: Color32::from_rgba_unmultiplied(60, 27, 24, 23),
            }
        }
    }

    pub fn ui(ui: &egui::Ui) -> Self {
        Self::new(ui.visuals().dark_mode)
    }

    fn lift(&self, popup: bool) -> Shadow {
        Shadow {
            offset: [0, if popup { 6 } else { 2 }],
            blur: if popup { 18 } else { 9 },
            spread: 0,
            color: self.shadow,
        }
    }
}

#[cfg(windows)]
fn os_allows_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };

    let mut allowed = 0i32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut allowed as *mut i32).cast(),
            0,
        ) != 0
            && allowed != 0
    }
}

#[cfg(not(windows))]
fn os_allows_motion() -> bool {
    true
}

fn motion_enabled(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new("kuma interface motion")))
        .unwrap_or(false)
}

fn mix_color(from: Color32, to: Color32, amount: f32) -> Color32 {
    let mix = |a: u8, b: u8| {
        (f32::from(a) + f32::from(i16::from(b) - i16::from(a)) * amount).round() as u8
    };
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "instrument",
            include_bytes!("../assets/fonts/InstrumentSans.ttf").as_slice(),
        ),
        (
            "bricolage",
            include_bytes!("../assets/fonts/BricolageGrotesque.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let proportional = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    proportional.insert(0, "instrument".into());
    let mut display = proportional.clone();
    display.insert(0, "bricolage".into());
    fonts
        .families
        .insert(FontFamily::Name("display".into()), display);
    ctx.set_fonts(fonts);
}

pub fn theme(ctx: &egui::Context, dark: bool) {
    let p = Palette::new(dark);
    let motion = os_allows_motion();
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("kuma interface motion"), motion));
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.raised;
    v.window_stroke = Stroke::new(1.0_f32, p.line);
    v.window_corner_radius = CornerRadius::same(CORNER_SURFACE);
    v.window_shadow = p.lift(true);
    v.popup_shadow = p.lift(true);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = p.track;
    v.faint_bg_color = p.hover;
    v.override_text_color = Some(p.ink);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.secondary;
    v.selection.stroke = Stroke::new(1.5_f32, p.accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    v.widgets.inactive.bg_fill = p.paper;
    v.widgets.inactive.weak_bg_fill = p.paper;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.line);
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.focus);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = p.pressed;
    v.widgets.active.weak_bg_fill = p.pressed;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.focus);
    v.widgets.active.expansion = 0.0;
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        widget.fg_stroke.color = p.ink;
        widget.corner_radius = CornerRadius::same(CORNER_CONTROL);
    }
    ctx.set_visuals(v);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 9.0);
    style.spacing.button_padding = Vec2::new(11.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.menu_margin = Margin::same(5);
    style.animation_time = if motion { FAST_MOTION } else { 0.0 };
    ctx.set_style(style);
}

pub fn heading(text: &str, size: f32) -> RichText {
    RichText::new(text).font(FontId::new(size, FontFamily::Name("display".into())))
}

pub fn surface(ui: &egui::Ui, pad: i8) -> Frame {
    let p = Palette::ui(ui);
    Frame::new()
        .inner_margin(Margin::same(pad))
        .fill(p.paper)
        .corner_radius(CornerRadius::same(CORNER_SURFACE))
        .stroke(Stroke::new(1.0_f32, p.line))
        .shadow(p.lift(false))
}

#[derive(Clone, Copy)]
pub enum Mark {
    Camera,
    Replay,
    Film,
    Record,
    Stop,
    Settings,
    Photo,
    Update,
}

pub fn mark(painter: &Painter, rect: Rect, kind: Mark, ink: Color32, accent: Color32) {
    let scale = rect.width().min(rect.height()) / 24.0;
    let at = |x: f32, y: f32| rect.center() + Vec2::new((x - 12.0) * scale, (y - 12.0) * scale);
    let box_at = |x1, y1, x2, y2| Rect::from_min_max(at(x1, y1), at(x2, y2));
    let stroke = Stroke::new(1.55 * scale, ink);
    match kind {
        Mark::Camera => {
            for x in [5.5, 18.5] {
                painter.circle_filled(at(x, 6.7), 2.6 * scale, accent);
            }
            painter.rect_filled(box_at(2.0, 7.0, 22.0, 20.0), 4.0 * scale, ink);
            painter.circle_filled(at(12.0, 13.5), 4.0 * scale, accent);
            painter.circle_filled(at(12.0, 13.5), 2.2 * scale, ink);
        }
        Mark::Replay => {
            let points = (0..23)
                .map(|i| {
                    let a = -0.62 + i as f32 * 0.235;
                    at(12.0 + a.cos() * 7.4, 12.0 + a.sin() * 7.4)
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            painter.add(egui::Shape::convex_polygon(
                vec![at(17., 5.), at(10., 3.), at(13., 10.)],
                accent,
                Stroke::NONE,
            ));
            painter.circle_filled(at(12., 12.), 2.1 * scale, accent);
        }
        Mark::Film | Mark::Photo => {
            painter.rect_stroke(
                box_at(3., 4., 21., 20.),
                3.0 * scale,
                stroke,
                StrokeKind::Inside,
            );
            if matches!(kind, Mark::Film) {
                painter.add(egui::Shape::convex_polygon(
                    vec![at(10., 8.), at(10., 16.), at(16., 12.)],
                    accent,
                    Stroke::NONE,
                ));
                for x in [5.7, 18.3] {
                    for y in [8., 12., 16.] {
                        painter.circle_filled(at(x, y), 0.8 * scale, ink);
                    }
                }
            } else {
                painter.circle_filled(at(15., 9.), 1.8 * scale, accent);
                painter.add(egui::Shape::line(
                    vec![
                        at(5., 17.),
                        at(9., 12.),
                        at(13., 16.),
                        at(15., 13.),
                        at(19., 17.),
                    ],
                    stroke,
                ));
            }
        }
        Mark::Record | Mark::Stop => {
            painter.circle_stroke(at(12., 12.), 8.2 * scale, stroke);
            if matches!(kind, Mark::Stop) {
                painter.rect_filled(box_at(9., 9., 15., 15.), 1.2 * scale, accent);
            } else {
                painter.circle_filled(at(12., 12.), 4.4 * scale, accent);
            }
        }
        Mark::Settings => {
            for (y, x) in [(6., 8.), (12., 16.), (18., 10.)] {
                painter.line_segment([at(3., y), at(21., y)], stroke);
                painter.circle_filled(at(x, y), 2.7 * scale, accent);
            }
        }
        Mark::Update => {
            painter.line_segment([at(12., 3.), at(12., 15.)], stroke);
            painter.add(egui::Shape::line(
                vec![at(7., 11.), at(12., 16.), at(17., 11.)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![at(4., 16.), at(4., 21.), at(20., 21.), at(20., 16.)],
                Stroke::new(1.55 * scale, accent),
            ));
        }
    }
}

pub fn bear(painter: &Painter, rect: Rect, smiling: bool) {
    let unit = rect.width().min(rect.height()) / 64.0;
    let point = |x: f32, y: f32| rect.center() + Vec2::new(x * unit, y * unit);
    let crumb = Color32::from_rgb(188, 119, 74);
    for x in [-20., 20.] {
        painter.circle_filled(point(x, -15.), 11. * unit, crumb);
        painter.circle_filled(point(x, -15.), 6.5 * unit, BLUSH);
    }
    painter.add(egui::epaint::EllipseShape::filled(
        point(0., 4.),
        Vec2::new(25., 20.) * unit,
        crumb,
    ));
    painter.add(egui::epaint::EllipseShape::filled(
        point(0., 11.),
        Vec2::new(11., 8.) * unit,
        MILK,
    ));
    for x in [-10., 10.] {
        if smiling {
            painter.circle_filled(point(x, 1.), 2.4 * unit, COCOA);
        } else {
            painter.line_segment(
                [point(x - 2.5, 1.), point(x + 2.5, 1.)],
                Stroke::new(1.4 * unit, COCOA),
            );
        }
    }
    painter.circle_filled(point(0., 8.), 2.8 * unit, COCOA);
    painter.add(egui::Shape::line(
        vec![point(-3., 13.), point(0., 10.), point(3., 13.)],
        Stroke::new(1.1 * unit, COCOA),
    ));
}

pub fn command(
    ui: &mut egui::Ui,
    label: &str,
    shortcut: &str,
    icon: Mark,
    primary: bool,
    enabled: bool,
) -> bool {
    let p = Palette::ui(ui);
    let size = Vec2::new(
        ui.available_width().max(120.0),
        if primary { 67.0 } else { 49.0 },
    );
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(
        rect,
        ui.id().with(label),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let solid = primary && enabled;
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let motion = motion_enabled(ui.ctx());
    let hover_amount = if motion {
        ui.ctx()
            .animate_bool_with_time(response.id.with("hover"), hovered, FAST_MOTION)
    } else {
        f32::from(u8::from(hovered))
    };
    let fill = match (solid, pressed, enabled) {
        (true, true, _) => JAM_PRESSED,
        (true, false, _) => mix_color(JAM, Color32::from_rgb(121, 37, 61), hover_amount),
        (_, true, _) => p.pressed,
        (_, _, false) => p.bg,
        _ => mix_color(p.paper, p.hover, hover_amount),
    };
    let text = if solid {
        MILK
    } else if enabled {
        p.ink
    } else {
        p.muted
    };
    let outline = if response.has_focus() {
        Stroke::new(2.0_f32, p.focus)
    } else if solid {
        Stroke::NONE
    } else {
        Stroke::new(1.0_f32, p.line)
    };
    let painter = ui.painter();
    let rect = if pressed && motion {
        rect.shrink(1.)
    } else {
        rect
    };
    painter.rect(rect, 11., fill, outline, StrokeKind::Inside);
    let icon_rect =
        Rect::from_center_size(rect.left_center() + Vec2::new(27., 0.), Vec2::splat(31.));
    painter.rect_filled(
        icon_rect,
        7.,
        if solid {
            Color32::from_rgb(163, 63, 92)
        } else if enabled {
            p.track
        } else {
            p.bg
        },
    );
    mark(
        painter,
        icon_rect.shrink(4.),
        icon,
        if solid {
            MILK
        } else if enabled {
            p.ink
        } else {
            p.muted
        },
        if matches!(icon, Mark::Record | Mark::Stop) {
            p.accent
        } else if solid {
            BLUSH
        } else {
            p.accent
        },
    );
    painter.text(
        rect.left_center() + Vec2::new(52., 0.),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(if primary { 17. } else { 14. }),
        text,
    );
    if !shortcut.is_empty() && rect.width() > 230. {
        let key = painter.layout_no_wrap(
            shortcut.replace('+', " + "),
            FontId::proportional(11.),
            text,
        );
        let key_rect = Rect::from_min_size(
            Pos2::new(
                rect.right() - 12. - key.size().x - 12.,
                rect.center().y - 9.,
            ),
            Vec2::new(key.size().x + 12., 18.),
        );
        painter.rect_filled(key_rect, 5., if solid { JAM_PRESSED } else { p.track });
        painter.galley(key_rect.center() - key.size() * 0.5, key, text);
    }
    response.clicked()
}

pub struct CapturePanel<'a> {
    pub shortcuts: &'a [String; 7],
    pub display: &'a str,
    pub replay: Option<(u64, u64)>,
    pub recording: Option<u64>,
    pub working: bool,
}

pub fn capture(
    ui: &mut egui::Ui,
    data: CapturePanel<'_>,
    mode: &mut usize,
    art: Option<&egui::TextureHandle>,
) -> Option<usize> {
    let p = Palette::ui(ui);
    let mut action = None;
    ui.label(heading("Capture", 36.));
    ui.add_space(1.);
    ui.label(
        RichText::new("Screenshots, instant replay, and full-display recording.")
            .size(14.)
            .color(p.muted),
    );
    ui.add_space(17.);
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        left.horizontal(|ui| {
            ui.label(heading("Screenshot", 20.));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let modes = ["Region", "Window", "Display", "All screens"];
                egui::ComboBox::from_id_salt("capture mode")
                    .selected_text(modes[*mode])
                    .width(104.)
                    .show_ui(ui, |ui| {
                        for (index, name) in modes.into_iter().enumerate() {
                            ui.selectable_value(mode, index, name);
                        }
                    });
            });
        });
        if command(
            left,
            "Take screenshot",
            &data.shortcuts[*mode],
            Mark::Camera,
            true,
            !data.working,
        ) {
            action = Some(*mode);
        }
        left.add_space(12.);
        left.label(heading("Clips and recording", 20.));
        if command(
            left,
            if data.replay.is_some() {
                "Stop replay"
            } else {
                "Arm replay"
            },
            &data.shortcuts[5],
            Mark::Replay,
            false,
            !data.working,
        ) {
            action = Some(5);
        }
        if command(
            left,
            "Save clip",
            &data.shortcuts[4],
            Mark::Film,
            false,
            !data.working && data.replay.is_some(),
        ) {
            action = Some(4);
        }
        if command(
            left,
            if data.recording.is_some() {
                "Stop recording"
            } else {
                "Start recording"
            },
            &data.shortcuts[6],
            if data.recording.is_some() {
                Mark::Stop
            } else {
                Mark::Record
            },
            false,
            !data.working,
        ) {
            action = Some(6);
        }

        let right = &mut columns[1];
        surface(right, 0).show(right, |ui| {
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(width, (width * 0.53).clamp(138., 235.)),
                Sense::hover(),
            );
            if let Some(texture) = art {
                egui::Image::new((texture.id(), texture.size_vec2()))
                    .corner_radius(CornerRadius {
                        nw: CORNER_SURFACE - 1,
                        ne: CORNER_SURFACE - 1,
                        sw: 0,
                        se: 0,
                    })
                    .paint_at(ui, rect);
            } else {
                ui.painter().rect_filled(
                    rect,
                    CornerRadius {
                        nw: CORNER_SURFACE - 1,
                        ne: CORNER_SURFACE - 1,
                        sw: 0,
                        se: 0,
                    },
                    BLUSH,
                );
                bear(
                    ui.painter(),
                    Rect::from_center_size(rect.center(), Vec2::splat(145.)),
                    false,
                );
            }
            Frame::new()
                .inner_margin(Margin::symmetric(17, 12))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new(data.display).size(14.).color(p.muted));
                    ui.add_space(1.);
                    let (label, value, dot) = if let Some(elapsed) = data.recording {
                        (
                            "Recording",
                            format!("{:02}:{:02}", elapsed / 60, elapsed % 60),
                            p.accent,
                        )
                    } else if let Some((elapsed, total)) = data.replay {
                        (
                            if elapsed >= total {
                                "Replay ready"
                            } else {
                                "Filling replay buffer"
                            },
                            format!("{} / {} s", elapsed.min(total), total),
                            if ui.visuals().dark_mode {
                                PISTACHIO
                            } else {
                                Color32::from_rgb(76, 108, 53)
                            },
                        )
                    } else {
                        ("Replay off", "".into(), p.muted)
                    };
                    ui.horizontal(|ui| {
                        Frame::new()
                            .fill(p.track)
                            .corner_radius(CornerRadius::same(CORNER_CONTROL))
                            .inner_margin(Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                let (dot_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(6.), Sense::hover());
                                ui.painter().circle_filled(dot_rect.center(), 3., dot);
                                ui.label(RichText::new(label).size(13.).color(p.ink));
                            });
                        if !value.is_empty() {
                            ui.label(RichText::new(value).color(p.muted));
                        }
                    });
                    if let Some((elapsed, total)) = data.replay.filter(|_| data.recording.is_none())
                    {
                        ui.add(
                            egui::ProgressBar::new((elapsed as f32 / total.max(1) as f32).min(1.0))
                                .desired_height(6.)
                                .fill(p.accent),
                        );
                    }
                });
        });
    });
    action
}

#[derive(Clone, Copy)]
struct HorizontalSpring {
    position: f32,
    velocity: f32,
}

fn spring_x(ctx: &egui::Context, id: egui::Id, target: f32, settle: f32) -> f32 {
    if !motion_enabled(ctx) {
        return target;
    }
    let dt = ctx.input(|input| input.stable_dt.min(1. / 30.));
    let mut spring = ctx
        .data(|data| data.get_temp::<HorizontalSpring>(id))
        .unwrap_or(HorizontalSpring {
            position: target,
            velocity: 0.,
        });
    let offset = spring.position - target;
    if offset.abs() < 0.25 && spring.velocity.abs() < 2. {
        spring.position = target;
        spring.velocity = 0.;
    } else {
        // Integrate a critically damped spring without resetting velocity when the target moves.
        let rate = 6. / settle;
        let drift = spring.velocity + rate * offset;
        let decay = (-rate * dt).exp();
        spring.position = target + (offset + drift * dt) * decay;
        spring.velocity = (spring.velocity - rate * drift * dt) * decay;
        ctx.request_repaint();
    }
    ctx.data_mut(|data| data.insert_temp(id, spring));
    spring.position
}

pub fn navigation(ui: &mut egui::Ui, current: usize) -> Option<usize> {
    let p = Palette::ui(ui);
    ui.horizontal(|ui| {
        let tabs = [
            ("Capture", Mark::Camera),
            ("History", Mark::Photo),
            ("Settings", Mark::Settings),
        ];
        let responses = tabs.map(|_| ui.allocate_exact_size(Vec2::new(109., 36.), Sense::click()));
        let track = responses[0].0.union(responses[2].0);
        let painter = ui.painter();
        painter.rect_filled(track, 11., p.track);
        let active = responses[current].0.shrink(3.);
        let selected_id = ui.id().with("active page spring");
        let selection_x = spring_x(ui.ctx(), selected_id, active.left(), TAB_MOTION);

        let hover_id = ui.id().with("page hover spring");
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let hovered = pointer
            .filter(|_| ui.rect_contains_pointer(track))
            .map(|pointer| {
                responses
                    .iter()
                    .enumerate()
                    .min_by(|(_, (a, _)), (_, (b, _))| {
                        (a.center().x - pointer.x)
                            .abs()
                            .total_cmp(&(b.center().x - pointer.x).abs())
                    })
                    .map_or(current, |(index, _)| index)
            });
        if let Some(index) = hovered {
            let hover = responses[index].0.shrink(3.);
            let x = spring_x(ui.ctx(), hover_id, hover.left(), FAST_MOTION);
            if index != current {
                painter.with_clip_rect(track).rect_filled(
                    hover.translate(Vec2::new(x - hover.left(), 0.)),
                    8.,
                    p.hover,
                );
            }
        } else {
            ui.ctx()
                .data_mut(|data| data.remove::<HorizontalSpring>(hover_id));
        }
        painter.with_clip_rect(track).rect_filled(
            active.translate(Vec2::new(selection_x - active.left(), 0.)),
            8.,
            p.secondary,
        );

        let mut next = None;
        for (i, ((rect, response), (name, icon))) in responses.into_iter().zip(tabs).enumerate() {
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    current == i,
                    name,
                )
            });
            let selected = current == i;
            if response.has_focus() {
                painter.rect_stroke(
                    rect.shrink(3.),
                    8.,
                    Stroke::new(2.0_f32, p.focus),
                    StrokeKind::Inside,
                );
            }
            let ink = if selected {
                p.accent
            } else if hovered == Some(i) {
                p.ink
            } else {
                p.muted
            };
            mark(
                painter,
                Rect::from_center_size(rect.left_center() + Vec2::new(17., 0.), Vec2::splat(17.)),
                icon,
                ink,
                ink,
            );
            painter.text(
                rect.left_center() + Vec2::new(32., 0.),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(14.),
                ink,
            );
            if response.clicked() {
                next = Some(i);
            }
        }
        next
    })
    .inner
}

pub fn artwork(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory(include_bytes!("../assets/camera-bear.jpg"))
        .ok()?
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(ctx.load_texture(
        "Kuma with camera",
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    ))
}
