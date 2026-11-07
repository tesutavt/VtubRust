//! Scripted demo (RIGRS_DEMO=1): synthetic head motion and a fixed UI/VFX timeline,
//! used to record the showcase video and screenshots without a camera or mouse.

use crate::app::App;
use crate::mapping::input;
use crate::ui::{Strip, Tab};

pub struct Demo {
    pub t: f32,
    step: usize,
}

impl Demo {
    pub fn new() -> Demo {
        Demo { t: 0.0, step: 0 }
    }
}

fn set_fx(app: &mut App, on: &[&str]) {
    for c in &mut app.settings.vfx {
        c.on = on.contains(&c.id.as_str());
    }
    app.settings.vfx_on = !on.is_empty();
    if let Some(c) = app.settings.vfx.iter_mut().find(|c| c.id == "weather") {
        c.p[0] = 1.0; // snow
    }
    if let Some(c) = app.settings.vfx.iter_mut().find(|c| c.id == "color") {
        c.p = vec![0.03, 1.1, 1.25, 0.0, 0.45, 1.0];
    }
}

/// Advances the demo; returns false when it is over.
pub fn step(app: &mut App, dt: f32) -> bool {
    let Some(d) = app.demo.as_mut() else { return true };
    d.t += dt;
    let t = d.t;

    // Synthetic tracking: gentle head sway, blinking, talking.
    let v = &mut app.inputs;
    v[input::FACE_ANGLE_X] = (t * 0.9).sin() * 18.0;
    v[input::FACE_ANGLE_Y] = (t * 0.6).sin() * 9.0;
    v[input::FACE_ANGLE_Z] = (t * 0.5).sin() * 7.0;
    v[input::EYE_RIGHT_X] = (t * 0.7).sin() * 0.6;
    v[input::EYE_LEFT_X] = (t * 0.7).sin() * 0.6;
    let blink = (t % 3.2) < 0.15;
    v[input::EYE_OPEN_LEFT] = if blink { 0.0 } else { 1.0 };
    v[input::EYE_OPEN_RIGHT] = if blink { 0.0 } else { 1.0 };
    let talk = ((t * 0.5).sin() > 0.2) as i32 as f32;
    v[input::MOUTH_OPEN] = ((t * 9.0).sin().max(0.0) * 0.7) * talk;
    v[input::MOUTH_SMILE] = 0.75;
    v[input::BROWS] = 0.5 + (t * 0.8).sin() * 0.15;
    app.tracking = true;

    // UI / effects timeline: (time, action id).
    const TIMELINE: [(f32, u8); 16] = [
        (5.0, 1),   // models strip
        (9.0, 2),   // close strip
        (10.0, 3),  // hide UI
        (12.5, 4),  // show UI
        (14.0, 5),  // settings: general
        (17.5, 6),  // settings: camera
        (21.0, 7),  // settings: model
        (24.5, 8),  // close settings
        (26.0, 9),  // snow + glow + rim
        (31.0, 10), // ascii
        (34.0, 11), // glitches
        (37.0, 12), // colour grade + cinema
        (41.0, 13), // effects off, items strip
        (44.0, 2),  // close strip
        (47.0, 14), // end
        (999.0, 0),
    ];
    let mut due = Vec::new();
    {
        let d = app.demo.as_mut().unwrap();
        while d.step < TIMELINE.len() && TIMELINE[d.step].0 <= t {
            due.push(TIMELINE[d.step].1);
            d.step += 1;
        }
    }
    for action in due {
        match action {
            1 => app.ui.strip = Strip::Models,
            2 => app.ui.strip = Strip::None,
            3 => app.ui.visible = false,
            4 => app.ui.visible = true,
            5 => {
                app.ui.settings_open = true;
                app.ui.tab = Tab::General;
            }
            6 => app.ui.tab = Tab::Camera,
            7 => app.ui.tab = Tab::Model,
            8 => app.ui.settings_open = false,
            9 => set_fx(app, &["weather", "rim"]),
            10 => set_fx(app, &["ascii"]),
            11 => set_fx(app, &["analog", "digital"]),
            12 => set_fx(app, &["color", "cinema"]),
            13 => {
                set_fx(app, &[]);
                app.ui.strip = Strip::Items;
            }
            14 => return false,
            _ => {}
        }
    }
    true
}
