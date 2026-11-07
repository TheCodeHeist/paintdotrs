//! Paint.RS: a layered image editor for Linux.
//!
//! Usage: pixel_easel [picture files...] [--screenshot out.png]

mod app;
mod doc;
mod effects;
mod io;
mod raster;
mod tools;
mod clip;
mod smart;
mod fonts;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let mut shot = None;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            shot = args.next().map(PathBuf::from);
        } else if a == "--help" || a == "-h" {
            println!("Usage: pixel_easel [picture files...] [--screenshot out.png]");
            return Ok(());
        } else {
            files.push(PathBuf::from(a));
        }
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(app::APP_NAME)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 520.0])
            .with_app_id("paint-rs")
            .with_drag_and_drop(true),
        // Native Wayland now has drag and drop and a picture clipboard (our addition to the window library).
        // PAINT_RS_X11=1 runs through X11/XWayland instead.
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            let has_x11 = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
            if has_x11 && std::env::var_os("PAINT_RS_X11").is_some() {
                builder.with_x11();
            }
        })),
        // Ctrl+C / Ctrl+X / Ctrl+V come to the app as keys; it handles pictures and passes text to text boxes.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    eframe::run_native(app::APP_NAME, options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, shot, files)))))
}
