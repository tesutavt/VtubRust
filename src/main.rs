//! rigrs — a lightweight, independent Live2D face-tracking avatar app written in Rust.
//! Educational project: reads the same public model formats VTube Studio users have
//! (.model3.json / .vtube.json), but shares no code with it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod api_events;
mod artmesh;
mod app;
mod audio;
mod sfx;
mod controller;
mod cubism_editor;
mod hotkeys;
mod twitch;
mod twitch_fx;
mod i18n;
mod ui_hotkeys;
mod ui_items;
mod ui_vts;
mod updater;
mod ui_model_extra;
mod cubism;
mod formats;
mod gpu;
mod library;
mod mapping;
mod ndi;
mod record;
mod tobii;
mod lighting;
mod lipsync;
mod vfx;
mod vfx_schema;
mod vcam;
mod demo;
mod scene;
mod settings;
mod spout;
mod stats;
mod tracking;
mod ui;

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct Runner {
    app: Option<app::App>,
    next_frame: Instant,
}

impl ApplicationHandler for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        let demo = std::env::var_os("RIGRS_DEMO").is_some();
        let mut attrs = Window::default_attributes()
            .with_title(if demo { "rigrs demo" } else { "rigrs — Live2D avatar" })
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        if demo {
            attrs = attrs
                .with_inner_size(winit::dpi::PhysicalSize::new(1600u32, 900u32))
                .with_position(winit::dpi::PhysicalPosition::new(0, 0))
                .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
                .with_resizable(false);
        }
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("window: {e}");
                el.exit();
                return;
            }
        };
        match app::App::new(window) {
            Ok(a) => self.app = Some(a),
            Err(e) => {
                eprintln!("startup failed: {e:#}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(app) = &mut self.app else { return };
        app.on_window_event(&event);
        match event {
            WindowEvent::CloseRequested => {
                app.shutdown();
                el.exit();
            }
            WindowEvent::Resized(s) => app.resize(s.width, s.height),
            WindowEvent::Occluded(o) => app.occluded = o,
            WindowEvent::DroppedFile(p) => app.on_dropped_file(p),
            WindowEvent::RedrawRequested => {
                app.frame();
                if app.quit {
                    app.shutdown();
                    if app.restart_after_quit {
                        crate::updater::restart();
                    }
                    el.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Some(app) = &mut self.app else { return };
        // Windows sends no WM_PAINT to a minimized window, so redraw requests are never
        // answered; run the frame directly to keep outputs, hotkeys and the API alive.
        let minimized = app.window.is_minimized().unwrap_or(false);
        let cap = match app.settings.fps {
            settings::FpsMode::VSyncHalf => {
                let hz = app.window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).unwrap_or(60_000) as f32 / 1000.0;
                Some(hz / 2.0)
            }
            m => m.cap(),
        };
        match cap {
            Some(fps) => {
                let now = Instant::now();
                let period = Duration::from_secs_f32(1.0 / fps);
                if now >= self.next_frame {
                    if minimized {
                        app.frame();
                    } else {
                        app.window.request_redraw();
                    }
                    self.next_frame += period;
                    if self.next_frame < now {
                        self.next_frame = now + period;
                    }
                }
                el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
            }
            None => {
                if minimized {
                    app.frame();
                    el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(16)));
                    return;
                }
                app.window.request_redraw();
                el.set_control_flow(ControlFlow::Wait);
            }
        }
    }
}

fn main() {
    // 1 ms timer resolution so frame pacing via WaitUntil is accurate on Windows.
    #[cfg(windows)]
    unsafe {
        windows::Win32::Media::timeBeginPeriod(1);
    }
    let el = EventLoop::new().expect("event loop");
    let mut runner = Runner { app: None, next_frame: Instant::now() };
    if let Err(e) = el.run_app(&mut runner) {
        eprintln!("{e}");
    }
}
