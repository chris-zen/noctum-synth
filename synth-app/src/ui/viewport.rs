use std::sync::Arc;
use std::sync::Mutex;

use eframe::egui::{self, ViewportCommand, ViewportInfo};

pub const GEOMETRY_TOLERANCE: f32 = 2.0;
const SETTLE_FRAMES: u8 = 2;
const MAX_RESTORE_FRAMES: u32 = 90;
const RESTORE_PLACEHOLDER_HEIGHT: f32 = 400.0;
const MAIN_DEFAULT_SIZE: [f32; 2] = [1200.0, 900.0];

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn main_viewport_builder(persisted: Option<WindowGeometry>) -> egui::ViewportBuilder {
    let builder = egui::ViewportBuilder::default().with_inner_size(MAIN_DEFAULT_SIZE);
    match persisted {
        Some(geometry) => builder
            .with_position([geometry.x, geometry.y])
            .with_inner_size([geometry.w, geometry.h]),
        None => builder.with_maximized(true),
    }
}

impl WindowGeometry {
    pub fn from_viewport(vp: &ViewportInfo) -> Option<Self> {
        let outer = vp.outer_rect?;
        let inner = vp.inner_rect?;
        Some(Self {
            x: outer.min.x,
            y: outer.min.y,
            w: inner.width(),
            h: inner.height(),
        })
    }

    pub fn approx_eq(&self, other: &Self) -> bool {
        (self.x - other.x).abs() <= GEOMETRY_TOLERANCE
            && (self.y - other.y).abs() <= GEOMETRY_TOLERANCE
            && (self.w - other.w).abs() <= GEOMETRY_TOLERANCE
            && (self.h - other.h).abs() <= GEOMETRY_TOLERANCE
    }
}

#[derive(Default)]
enum Restore {
    #[default]
    Idle,
    Applying {
        target: WindowGeometry,
        matched_frames: u8,
        frame: u32,
    },
    Aborted(WindowGeometry),
}

impl Restore {
    fn begin(target: Option<WindowGeometry>) -> Self {
        match target {
            Some(target) => Self::Applying {
                target,
                matched_frames: 0,
                frame: 0,
            },
            None => Self::Idle,
        }
    }

    fn drive(&mut self, ctx: &egui::Context) {
        let Self::Applying {
            target,
            matched_frames,
            frame,
        } = self
        else {
            return;
        };

        *frame += 1;

        let already_matches = ctx.input(|input| {
            WindowGeometry::from_viewport(input.viewport())
                .is_some_and(|current| current.approx_eq(target))
        });

        if already_matches {
            *matched_frames += 1;
            if *matched_frames >= SETTLE_FRAMES {
                *self = Self::Idle;
            }
            return;
        }

        *matched_frames = 0;
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(
            target.x, target.y,
        )));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(target.w, target.h)));

        if *frame >= MAX_RESTORE_FRAMES {
            let observed = ctx.input(|input| WindowGeometry::from_viewport(input.viewport()));
            *self = observed.map_or(Self::Idle, Self::Aborted);
        }
    }

    fn allows_capture(&self, current: &WindowGeometry) -> bool {
        match self {
            Self::Idle => true,
            Self::Applying { .. } => false,
            Self::Aborted(observed) => !observed.approx_eq(current),
        }
    }
}

struct GeometryTracker {
    restore: Restore,
    persisted: Option<WindowGeometry>,
    snapshot: Option<WindowGeometry>,
}

impl GeometryTracker {
    fn new(persisted: Option<WindowGeometry>) -> Self {
        Self {
            restore: Restore::Idle,
            persisted,
            snapshot: None,
        }
    }

    fn begin_restore(&mut self) {
        self.restore = Restore::begin(self.persisted);
    }

    fn capture(&mut self, ctx: &egui::Context) {
        self.restore.drive(ctx);
        if let Some(geometry) = ctx.input(|input| WindowGeometry::from_viewport(input.viewport())) {
            if self.restore.allows_capture(&geometry) {
                self.snapshot = Some(geometry);
            }
        }
    }

    fn geometry(&mut self) -> Option<WindowGeometry> {
        if let Some(geometry) = self.snapshot.take() {
            self.persisted = Some(geometry);
        }
        self.persisted
    }
}

pub struct RootViewport {
    tracker: GeometryTracker,
}

impl RootViewport {
    pub fn from_config(persisted: Option<WindowGeometry>) -> Self {
        let mut tracker = GeometryTracker::new(persisted);
        tracker.begin_restore();
        Self { tracker }
    }

    pub fn drive(&mut self, ctx: &egui::Context) {
        self.tracker.capture(ctx);
    }

    pub fn geometry(&mut self) -> Option<WindowGeometry> {
        self.tracker.geometry()
    }
}

pub struct DeferredViewport {
    pub open: bool,
    was_open: bool,
    created: bool,
    id: &'static str,
    title: &'static str,
    default_size: [f32; 2],
    tracker: Arc<Mutex<GeometryTracker>>,
}

impl DeferredViewport {
    pub fn from_config(
        id: &'static str,
        title: &'static str,
        default_size: [f32; 2],
        open: bool,
        persisted: Option<WindowGeometry>,
    ) -> Self {
        Self {
            open,
            was_open: open,
            created: false,
            id,
            title,
            default_size,
            tracker: Arc::new(Mutex::new(GeometryTracker::new(persisted))),
        }
    }

    pub fn geometry(&mut self) -> Option<WindowGeometry> {
        self.tracker.lock().unwrap().geometry()
    }

    pub fn show_deferred<F>(&mut self, ctx: &egui::Context, draw: F)
    where
        F: Fn(&mut egui::Ui) + Send + Sync + 'static,
    {
        if !self.open {
            if self.was_open {
                self.created = false;
            }
            self.was_open = false;
            return;
        }

        self.was_open = true;
        let viewport_id = egui::ViewportId::from_hash_of(self.id);
        let builder = self.creation_builder();
        let tracker = self.tracker.clone();

        ctx.show_viewport_deferred(viewport_id, builder, move |vctx, _class| {
            vctx.request_repaint();
            tracker.lock().unwrap().capture(vctx);
            egui::CentralPanel::default().show_inside(vctx, |ui| {
                draw(ui);
            });
        });
    }

    fn creation_builder(&mut self) -> egui::ViewportBuilder {
        if self.created {
            return egui::ViewportBuilder::default();
        }

        self.created = true;
        let mut builder = egui::ViewportBuilder::default().with_title(self.title);

        let mut tracker = self.tracker.lock().unwrap();
        if let Some(target) = tracker.persisted {
            builder = builder
                .with_position([target.x, target.y])
                .with_inner_size([target.w, RESTORE_PLACEHOLDER_HEIGHT]);
            tracker.begin_restore();
        } else {
            builder = builder.with_inner_size(self.default_size);
        }

        builder
    }
}
