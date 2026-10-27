//! Real offscreen rendering of the production egui UI, without an OS window or audio device.
//! This is framebuffer/layout evidence, NOT a native-window, OS-input, or playback test.

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use aqua_pattern_core::{desktop::Leviathan, editor::Field, song::Note};
    use eframe::{egui, glow, App};
    use glow::HasContext;
    use glutin::{
        api::egl::{device::Device, display::Display},
        config::{Api, ConfigSurfaceTypes, ConfigTemplateBuilder},
        context::{ContextApi, ContextAttributesBuilder, Version},
        prelude::*,
        surface::{PbufferSurface, SurfaceAttributesBuilder},
    };
    use std::{
        ffi::CString,
        fs::OpenOptions,
        io::{BufWriter, Write},
        num::NonZeroU32,
        sync::Arc,
    };

    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 && args.len() != 6 {
        return Err("usage: ui_capture NEW.png WIDTH HEIGHT shell|edit|follow [NEW.json]".into());
    }
    let width: u32 = args[2].parse()?;
    let height: u32 = args[3].parse()?;
    if !(320..=4096).contains(&width) || !(240..=4096).contains(&height) {
        return Err("capture dimensions must be 320..4096 by 240..4096".into());
    }
    let scene = args[4].as_str();
    if !matches!(scene, "shell" | "edit" | "follow") {
        return Err("scene must be shell, edit, or follow".into());
    }
    if std::path::Path::new(&args[1]).exists()
        || args
            .get(5)
            .is_some_and(|p| std::path::Path::new(p).exists())
    {
        return Err("capture outputs must not already exist".into());
    }

    let device = Device::query_devices()?
        .next()
        .ok_or("no EGL device available; install/configure a real EGL implementation")?;
    // SAFETY: the device was enumerated by glutin, and no raw native display is supplied.
    let display = unsafe { Display::with_device(&device, None)? };
    let template = ConfigTemplateBuilder::new()
        .with_api(Api::OPENGL)
        .with_surface_type(ConfigSurfaceTypes::PBUFFER)
        .with_alpha_size(8)
        .with_depth_size(0)
        .with_stencil_size(0)
        .build();
    // SAFETY: the template contains no raw native handles.
    let config = unsafe { display.find_configs(template)? }
        .next()
        .ok_or("no EGL OpenGL pbuffer configuration")?;
    let attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(None);
    // SAFETY: configuration belongs to this display, no raw native window is used.
    let context = unsafe { display.create_context(&config, &attributes)? };
    let attributes = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
        NonZeroU32::new(width).unwrap(),
        NonZeroU32::new(height).unwrap(),
    );
    // SAFETY: the configuration belongs to this display and dimensions are nonzero.
    let surface = unsafe { display.create_pbuffer_surface(&config, &attributes)? };
    let context = context.make_current(&surface)?;
    // SAFETY: this thread owns a current GL context for the lifetime of all GL calls.
    let gl = Arc::new(unsafe {
        glow::Context::from_loader_function(|name| {
            display.get_proc_address(&CString::new(name).expect("GL name has no NUL"))
        })
    });
    // SAFETY: the context is current and these standard strings have no side effects.
    let (renderer, version) = unsafe {
        (
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VERSION),
        )
    };
    let mut painter = eframe::egui_glow::Painter::new(Arc::clone(&gl), "", None, false)?;
    let ctx = egui::Context::default();
    let creation = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Leviathan::new(&creation);
    if scene == "edit" {
        let editor = app.capture_editor_mut();
        editor.edit_mode = true;
        editor.edit_step = 0;
        editor.field = Field::Note;
        editor.enter_note(Note::Note(60))?;
        editor.field = Field::Instrument;
        editor.enter_value("1")?;
        editor.field = Field::Velocity;
        editor.enter_value("0.75")?;
        editor.row = 4;
        editor.field = Field::Note;
        editor.enter_note(Note::Note(64))?;
        editor.row = 8;
        editor.enter_note(Note::Off)?;
        editor.row = 4;
    }
    let follow_song = app.capture_editor_mut().song.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let mut primitive_count = 0;
    let mut text_labels = Vec::new();
    // Multiple ordinary frames let egui settle auto-sized panels and font textures.
    for n in 0..5 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            time: Some(n as f64 / 60.0),
            focused: true,
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            if scene == "follow" {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("OFFSCREEN COMPONENT CHECK · no audio device or transport is running");
                    aqua_pattern_core::tracker_ui::playback_view(ui, &follow_song, 0, 8);
                });
            } else {
                app.update(ctx, &mut frame);
            }
        });
        if n == 4 {
            for clipped in &output.shapes {
                collect_text(&clipped.shape, &mut text_labels);
            }
        }
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        primitive_count = primitives.len();
        painter.clear([width, height], [0.07, 0.09, 0.12, 1.0]);
        painter.paint_and_update_textures(
            [width, height],
            output.pixels_per_point,
            &primitives,
            &output.textures_delta,
        );
    }
    if primitive_count == 0 {
        return Err("UI produced no paint primitives".into());
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: a current context/pbuffer is alive, and the RGBA buffer exactly fits the read.
    unsafe {
        gl.finish();
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut rgba)),
        );
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            painter.destroy();
            return Err(format!("GL framebuffer read failed: 0x{error:x}").into());
        }
    }
    painter.destroy();
    // GL's origin is bottom-left; PNG uses top-left. This preserves captured pixels.
    let stride = width as usize * 4;
    for y in 0..height as usize / 2 {
        let split = (height as usize - y - 1) * stride;
        let (top, bottom) = rgba.split_at_mut(split);
        top[y * stride..(y + 1) * stride].swap_with_slice(&mut bottom[..stride]);
    }
    let unique_colors: std::collections::HashSet<_> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    if unique_colors.len() < 20 {
        return Err("framebuffer is blank or unexpectedly uniform".into());
    }
    let png = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?;
    let mut png = BufWriter::new(png);
    image::write_buffer_with_format(
        &mut png,
        &rgba,
        width,
        height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    png.flush()?;
    let report = serde_json::json!({
        "evidence": "real EGL pbuffer / egui Glow framebuffer; no OS window, physical audio, or OS-input validation",
        "scene": if scene == "follow" { "isolated production playback_view component" } else { scene },
        "app_update": scene != "follow",
        "width": width, "height": height, "scale": 1,
        "egl": display.version_string(), "renderer": renderer, "gl_version": version,
        "paint_primitives": primitive_count, "unique_rgb_colors": unique_colors.len(),
        "text_labels": text_labels,
        "physical_audio_feature": cfg!(feature = "realtime"),
    });
    if let Some(path) = args.get(5) {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &report)?;
        writer.flush()?;
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    drop(app);
    drop(painter);
    drop(gl);
    drop(context.make_not_current()?);
    drop(surface);
    // SAFETY: this standalone capture process has destroyed every EGL/GL resource.
    unsafe { display.terminate() };
    Ok(())
}

#[cfg(target_os = "linux")]
fn collect_text(shape: &eframe::egui::epaint::Shape, labels: &mut Vec<String>) {
    use eframe::egui::epaint::Shape;
    match shape {
        Shape::Text(text) => labels.push(text.galley.text().to_owned()),
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, labels);
            }
        }
        _ => (),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("ui_capture is a Linux EGL validation tool; use the native app on this platform.");
    std::process::exit(1);
}
