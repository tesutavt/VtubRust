//! UI pieces for the extra trackers (NVIDIA / phone) and the VNet-style collab window.

use egui::{Color32, RichText};

use crate::app::App;
use crate::i18n::tr;
use crate::tracking::backend;

/// Backend-specific settings shown under "Tracking quality" in the camera tab.
pub fn tracker_extra(app: &mut App, ui: &mut egui::Ui, running: bool) {
    match app.settings.track_backend {
        backend::PHONE => {
            ui.add_space(4.0);
            ui.label(RichText::new(tr("IP телефона (видно в приложении VTube Studio, включите «3rd Party PC Clients»)")).size(15.0));
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut app.settings.phone_ip).hint_text("192.168.1.23").desired_width(170.0));
                    ui.label(tr("порт"));
                    ui.add(egui::DragValue::new(&mut app.settings.phone_port).range(1..=65535));
                    ui.label(tr("локальный"));
                    ui.add(egui::DragValue::new(&mut app.settings.phone_local_port).range(1024..=65535));
                });
            });
            ui.label(
                RichText::new(tr("Телефон может одновременно отправлять данные и в VBridger. Кнопки 1-8 на экране телефона запускают горячие клавиши с ScreenButton 1-8."))
                    .size(13.0)
                    .color(Color32::GRAY),
            );
        }
        backend::NVIDIA => {
            ui.add_space(4.0);
            let found = std::path::Path::new(&app.settings.nv_exe).is_file();
            let (txt, col) = if found {
                (tr("ExpressionApp.exe найден"), Color32::from_rgb(60, 160, 90))
            } else {
                (tr("ExpressionApp.exe не найден: установите бесплатное DLC VTube Studio «NVIDIA Broadcast Tracker» в Steam или укажите путь в настройках"), Color32::from_rgb(220, 80, 80))
            };
            ui.label(RichText::new(txt).size(14.0).color(col));
            ui.label(
                RichText::new(tr("Только Windows и видеокарты RTX 20xx и новее. Нажмите «Калибровать» с нейтральным лицом — коэффициенты сохранятся для следующих запусков."))
                    .size(13.0)
                    .color(Color32::GRAY),
            );
            if !app.settings.nv_cal.is_empty() && ui.add_enabled(!running, egui::Button::new(tr("Сбросить калибровку NVIDIA"))).clicked() {
                app.settings.nv_cal.clear();
                app.save();
            }
        }
        _ => {}
    }
}
