//! Native framebuffer verification; synthetic pixels never reach the clipboard.
use crate::{CaptureWindow, OverlayWindow, Shot, hwnd, image_from_rgba, native};
use slint::{ComponentHandle, ModelRc, VecModel, winit_030::WinitWindowAccessor};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::RECT,
    UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, IsWindowVisible,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    },
};

fn configure(
    window: &slint::Window,
    monitor: native::MonitorInfo,
    height: f64,
) -> Result<(), String> {
    window
        .with_winit_window(|w| w.set_cursor_hittest(false))
        .ok_or("Native window unavailable")?
        .map_err(|e| e.to_string())?;
    let h = hwnd(window).ok_or("HWND unavailable")?;
    native::configure_overlay(h).map_err(|e| e.to_string())?;
    native::set_click_through(h, true).map_err(|e| e.to_string())?;
    native::place_overlay(h, monitor, height).map_err(|e| e.to_string())
}

struct Frame {
    report: serde_json::Value,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    colored: usize,
    bounds: Option<[u32; 4]>,
    mean_red: f64,
}

fn snapshot(
    window: &slint::Window,
    directory: &std::path::Path,
    name: &str,
) -> Result<Frame, String> {
    let buffer = window.take_snapshot().map_err(|e| e.to_string())?;
    let width = buffer.width();
    let height = buffer.height();
    let pixels = buffer.as_bytes().to_vec();
    image::save_buffer(
        directory.join(format!("{name}.png")),
        &pixels,
        width,
        height,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    let mut colored = 0usize;
    let mut red = 0u64;
    let mut bounds = [width, height, 0, 0];
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        if p[3] > 200
            && i16::from(p[2]) - i16::from(p[0]) > 25
            && i16::from(p[1]) - i16::from(p[0]) > 10
        {
            let (x, y) = (i as u32 % width, i as u32 / width);
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
            colored += 1;
            red += u64::from(p[0]);
        }
    }
    let bounds = (colored > 0).then_some(bounds);
    let mean_red = red as f64 / colored.max(1) as f64;
    let report = serde_json::json!({"name": name, "width": width, "height": height,
        "colored_pixels": colored, "photo_bounds": bounds, "mean_photo_red": mean_red});
    Ok(Frame {
        report,
        pixels,
        width,
        height,
        colored,
        bounds,
        mean_red,
    })
}

fn region(frame: &Frame, x: f32, y: f32, width: f32, height: f32, scale: f32) -> Vec<u8> {
    let mut output = Vec::new();
    let left = (x * scale).max(0.0) as u32;
    let top = (y * scale).max(0.0) as u32;
    let right = ((x + width) * scale).min(frame.width as f32) as u32;
    let bottom = ((y + height) * scale).min(frame.height as f32) as u32;
    for row in top..bottom {
        output.extend_from_slice(
            &frame.pixels[((row * frame.width + left) * 4) as usize
                ..((row * frame.width + right) * 4) as usize],
        );
    }
    output
}

pub fn run(directory: PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let foreground = unsafe { GetForegroundWindow() };
    let monitor = native::monitor_at_cursor().ok_or("Current display unavailable")?;
    let logical_width = monitor.width as f32 / monitor.scale as f32;
    let logical_height = monitor.height as f32 / monitor.scale as f32;
    let overlay = OverlayWindow::new().map_err(|e| e.to_string())?;
    let capture = CaptureWindow::new().map_err(|e| e.to_string())?;
    let sample = image::RgbaImage::from_pixel(600, 420, image::Rgba([25, 145, 205, 255]));
    let picture = image_from_rgba(&sample);
    let target_x = 160.0;
    let old_x = 390.0;
    let target_y = 54.0;
    overlay.set_shots(ModelRc::new(VecModel::from(vec![
        Shot {
            id: "new".into(),
            title: "Capture check".into(),
            thumbnail: picture.clone(),
            x: target_x,
            y: target_y - 16.0,
            card_width: 150.0,
            card_height: 104.0,
            angle: 0.0,
            video: false,
        },
        Shot {
            id: "old".into(),
            title: "Existing capture".into(),
            thumbnail: picture.clone(),
            x: old_x,
            y: target_y - 16.0,
            card_width: 150.0,
            card_height: 104.0,
            angle: 0.0,
            video: false,
        },
    ])));
    overlay.set_arriving_id("new".into());
    capture.set_snapshot(picture);
    let start_width = 720.0f32
        .min(logical_width * 0.65)
        .min(logical_height * 0.65 / 0.7);
    let start_height = start_width * 0.7;
    capture.set_start_x((logical_width - start_width) / 2.0);
    capture.set_start_y((logical_height - start_height) / 2.0);
    capture.set_start_width(start_width);
    capture.set_start_height(start_height);
    capture.set_target_x(target_x);
    capture.set_target_y(target_y);
    capture.set_target_width(150.0);
    capture.set_target_height(104.0);
    capture.set_target_angle(0.0);
    overlay.show().map_err(|e| e.to_string())?;
    capture.show().map_err(|e| e.to_string())?;
    let reports = Rc::new(RefCell::new(Vec::<serde_json::Value>::new()));
    let frames = Rc::new(RefCell::new(Vec::<Frame>::new()));
    let failure = Rc::new(RefCell::new(None::<String>));
    let collected = reports.clone();
    let images = frames.clone();
    let error = failure.clone();
    let output = directory.clone();
    let timer = slint::Timer::default();
    let mut stage = 0;
    let mut configured = false;
    let mut since = Instant::now();
    let mut old_pixels = Vec::new();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(20),
        move || {
            // Winit creates the native windows only after its event loop starts.
            if !configured {
                let result = configure(overlay.window(), monitor, 300.0)
                    .and_then(|()| configure(capture.window(), monitor, f64::from(logical_height)));
                if let Err(e) = result {
                    *error.borrow_mut() = Some(e);
                    let _ = slint::quit_event_loop();
                    return;
                }
                configured = true;
                since = Instant::now();
                return;
            }
            let wait = [650, 100, 550, 900, 100][stage];
            if since.elapsed() < Duration::from_millis(wait) {
                return;
            }
            let result = (|| -> Result<(), String> {
                let h = hwnd(capture.window()).ok_or("Capture HWND disappeared")?;
                let styles = unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) } as u32;
                let required = WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0;
                let mut rect = RECT::default();
                unsafe { GetWindowRect(h, &mut rect) }.map_err(|e| e.to_string())?;
                let visible = unsafe { IsWindowVisible(h) }.as_bool();
                let native_ok = !visible || styles & required == required;
                let visibility_ok = visible == (stage != 3);
                let geometry_ok = rect.left == monitor.x
                    && rect.top == monitor.y
                    && rect.right - rect.left == monitor.width as i32
                    && rect.bottom - rect.top == monitor.height as i32;
                let focus_ok = unsafe { GetForegroundWindow() } == foreground;
                collected
                    .borrow_mut()
                    .push(serde_json::json!({"stage": stage,
                "check": "native_window", "noactivate_toolwindow_clickthrough": native_ok,
                "full_monitor_bounds": geometry_ok, "foreground_unchanged": focus_ok,
                "visible": visible, "expected_visibility": visibility_ok,
                "passed": native_ok && geometry_ok && focus_ok && visibility_ok}));
                match stage {
                    0 => {
                        let frame = snapshot(capture.window(), &output, "01-shutter")?;
                        let passed = frame.colored > 20_000 && frame.mean_red > 140.0;
                        collected
                            .borrow_mut()
                            .push(serde_json::json!({"frame": frame.report,
                        "check": "large_photo_shutter_flash", "passed": passed}));
                        images.borrow_mut().push(frame);
                        let shelf = snapshot(overlay.window(), &output, "02-arriving-hidden")?;
                        old_pixels = region(
                            &shelf,
                            old_x - 20.0,
                            20.0,
                            190.0,
                            180.0,
                            overlay.window().scale_factor(),
                        );
                        // One existing photo remains; the new card is absent while flying.
                        let left = region(
                            &shelf,
                            target_x + 10.0,
                            target_y + 10.0,
                            130.0,
                            80.0,
                            overlay.window().scale_factor(),
                        );
                        let hidden = left
                            .chunks_exact(4)
                            .all(|p| !(p[3] > 200 && i16::from(p[2]) - i16::from(p[0]) > 25));
                        collected
                            .borrow_mut()
                            .push(serde_json::json!({"frame": shelf.report,
                        "check": "new_card_hidden_during_flight", "passed": hidden}));
                        capture.set_flash(false);
                        capture.set_land(true);
                    }
                    1 => {
                        let frame = snapshot(capture.window(), &output, "03-shrinking")?;
                        let before = images.borrow();
                        let passed =
                            frame.colored > 1000 && frame.colored < before[0].colored * 85 / 100;
                        collected
                            .borrow_mut()
                            .push(serde_json::json!({"frame": frame.report,
                        "check": "intermediate_photo_is_smaller", "passed": passed}));
                        drop(before);
                        images.borrow_mut().push(frame);
                    }
                    2 => {
                        let frame = snapshot(capture.window(), &output, "04-target")?;
                        let scale = capture.window().scale_factor();
                        let landed = frame.bounds.is_some_and(|b| {
                            (b[0] as f32 / scale - (target_x + 4.0)).abs() < 5.0
                                && (b[1] as f32 / scale - (target_y + 4.0)).abs() < 5.0
                                && b[2] - b[0] < (151.0 * scale) as u32
                        });
                        let shrinking = frame.colored < images.borrow()[1].colored;
                        collected
                            .borrow_mut()
                            .push(serde_json::json!({"frame": frame.report,
                        "check": "photo_reaches_peg", "passed": landed && shrinking}));
                        images.borrow_mut().push(frame);
                        capture.hide().map_err(|e| e.to_string())?;
                        overlay.set_arriving_id("".into());
                        overlay.set_landed_id("new".into());
                        overlay.set_landing_serial(1);
                    }
                    3 => {
                        let frame = snapshot(overlay.window(), &output, "05-hanging-settled")?;
                        let existing = region(
                            &frame,
                            old_x - 20.0,
                            20.0,
                            190.0,
                            180.0,
                            overlay.window().scale_factor(),
                        );
                        collected
                            .borrow_mut()
                            .push(serde_json::json!({"frame": frame.report,
                        "check": "new_photo_hangs_and_old_photo_stays_still",
                        "passed": frame.colored > images.borrow()[2].colored * 17 / 10
                            && existing == old_pixels}));
                        capture.set_reduced_motion(true);
                        capture.set_land(false);
                        capture.set_flash(false);
                        capture.show().map_err(|e| e.to_string())?;
                        configure(capture.window(), monitor, f64::from(logical_height))?;
                    }
                    4 => {
                        let frame = snapshot(capture.window(), &output, "06-reduced-motion")?;
                        let normal = &images.borrow()[2];
                        collected
                            .borrow_mut()
                            .push(serde_json::json!({"frame": frame.report,
                        "check": "reduced_motion_starts_at_peg_without_flash",
                        "passed": frame.bounds == normal.bounds && frame.mean_red < 80.0
                            && frame.colored.abs_diff(normal.colored) < 100}));
                        capture.hide().map_err(|e| e.to_string())?;
                        overlay.hide().map_err(|e| e.to_string())?;
                    }
                    _ => unreachable!(),
                }
                Ok(())
            })();
            if let Err(e) = result {
                *error.borrow_mut() = Some(e);
                let _ = slint::quit_event_loop();
                return;
            }
            stage += 1;
            since = Instant::now();
            if stage == 5 {
                let _ = slint::quit_event_loop();
            }
        },
    );
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    let checks = reports.borrow();
    let passed = failure.borrow().is_none()
        && checks.len() == 11
        && checks.iter().all(|check| check["passed"] == true);
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed": passed, "error": *failure.borrow(), "checks": *checks}),
        )
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if passed {
        Ok(())
    } else {
        Err(format!(
            "Capture animation check failed; see {}",
            directory.join("report.json").display()
        ))
    }
}
