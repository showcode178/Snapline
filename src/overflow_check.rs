//! One real overflow sequence with synthetic images and an isolated profile.
use crate::{App, Shot, hwnd, native, storage};
use slint::{ComponentHandle, Model};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{POINT, RECT},
    UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, IsWindowVisible,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WindowFromPoint,
    },
};

fn import(app: &App, index: usize) -> Result<storage::Item, String> {
    let colors = [
        [225, 35, 55, 255],
        [35, 160, 80, 255],
        [35, 90, 215, 255],
        [140, 50, 180, 255],
        [210, 145, 20, 255],
        [30, 170, 180, 255],
        [85, 85, 95, 255],
        [170, 110, 70, 255],
        [75, 150, 130, 255],
    ];
    let image = image::RgbaImage::from_pixel(240, 160, image::Rgba(colors[index]));
    app.store
        .lock()
        .unwrap()
        .import_rgba(image.width(), image.height(), image.as_raw())?
        .ok_or_else(|| "Synthetic image was deduplicated".into())
}

fn shots(app: &App) -> Vec<Shot> {
    (0..app.shots_model.row_count())
        .filter_map(|row| app.shots_model.row_data(row))
        .collect()
}

fn ids(shots: &[Shot]) -> Vec<String> {
    shots.iter().map(|shot| shot.id.to_string()).collect()
}

fn positions_unchanged(before: &[Shot], after: &[Shot]) -> bool {
    after.iter().all(|shot| {
        before.iter().any(|old| {
            old.id == shot.id && old.x == shot.x && old.y == shot.y && old.angle == shot.angle
        })
    })
}

fn green_has_not_moved(before: Option<[u32; 4]>, after: Option<[u32; 4]>, scale: f32) -> bool {
    before
        .zip(after)
        .is_some_and(|(before, after)| (before[0] as f32 - after[0] as f32).abs() <= 2.0 * scale)
}

fn photo_bounds(
    pixels: &slint::SharedPixelBuffer<slint::Rgba8Pixel>,
    green: bool,
) -> Option<[u32; 4]> {
    let mut bounds = [pixels.width(), pixels.height(), 0, 0];
    let mut count = 0;
    for (index, pixel) in pixels.as_bytes().chunks_exact(4).enumerate() {
        let matches = if green {
            pixel[3] > 200 && pixel[0] < 70 && pixel[1] > 120 && pixel[2] < 110
        } else {
            pixel[3] > 30 && pixel[0] > 150 && pixel[1] < 80 && pixel[2] < 90
        };
        if matches {
            let (x, y) = (index as u32 % pixels.width(), index as u32 / pixels.width());
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
            count += 1;
        }
    }
    (count > 50).then_some(bounds)
}

fn snapshot(
    window: &slint::Window,
    directory: &Path,
    name: &str,
) -> Result<Option<[u32; 4]>, String> {
    let pixels = window.take_snapshot().map_err(|e| e.to_string())?;
    image::save_buffer(
        directory.join(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    Ok(photo_bounds(&pixels, false))
}

fn check(
    checks: &mut Vec<serde_json::Value>,
    name: &str,
    passed: bool,
    evidence: serde_json::Value,
) -> Result<(), String> {
    checks.push(serde_json::json!({"name": name, "passed": passed, "evidence": evidence}));
    if passed {
        Ok(())
    } else {
        Err(format!("Overflow check failed: {name}"))
    }
}

pub fn run(app: Rc<App>, directory: PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut settings = app.settings.borrow().clone();
    settings.max_items = 5;
    settings.size = "small".into();
    settings.clipboard_enabled = false;
    settings.watch_enabled = false;
    settings.capture_sound = false;
    settings.show_over_fullscreen = true;
    {
        let mut store = app.store.lock().unwrap();
        if !store.items.is_empty() {
            return Err("Overflow check requires an empty isolated profile".into());
        }
        store.save_settings(settings.clone())?;
    }
    *app.settings.borrow_mut() = settings;
    app.overlay.set_reduced_motion(true);
    app.show_line(true);
    let mut imported = Vec::new();
    for index in 0..5 {
        imported.push(import(&app, index)?);
    }
    let (old_path, old_thumb) = {
        let store = app.store.lock().unwrap();
        (store.path(&imported[0]), store.thumb(&imported[0]))
    };
    app.refresh();
    app.tick();
    let foreground = unsafe { GetForegroundWindow() };
    let clipboard = native::clipboard_sequence();
    let reports = Rc::new(RefCell::new(Vec::<serde_json::Value>::new()));
    let failure = Rc::new(RefCell::new(None::<String>));
    let checks = reports.clone();
    let error = failure.clone();
    let output = directory.clone();
    let timer = slint::Timer::default();
    let mut stage = 0;
    let initial_started = Instant::now();
    let mut fall_started = Instant::now();
    let mut shift_started = None::<Instant>;
    let mut rapid_started = Instant::now();
    let mut rapid_shift_started = None::<Instant>;
    let mut first_rapid_expiry = None::<Instant>;
    let mut red_start = None;
    let mut rapid_green_start = None;
    let mut green_start = None;
    let mut green_old_x = 0.0;
    let mut green_target_x = 0.0;
    let mut initial = Vec::<Shot>::new();
    let mut rapid_positions = Vec::<Shot>::new();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), move || {
        if initial_started.elapsed() > Duration::from_secs(8) {
            *error.borrow_mut() = Some(format!("Overflow sequence timed out at stage {stage}"));
            let _ = slint::quit_event_loop();
            return;
        }
        // Winit can defer native creation until the first event-loop turn.
        if !app.configured.get() { app.tick(); }
        let waiting = app.pending_shots.borrow().is_some();
        app.advance_drop_motion();
        if waiting && app.pending_shots.borrow().is_none() {
            if stage < 5 { shift_started = Some(Instant::now()); }
            else { rapid_shift_started = Some(Instant::now()); }
        }
        let ready = match stage {
            0 => initial_started.elapsed() >= Duration::from_millis(350),
            1 => fall_started.elapsed() >= Duration::from_millis(60),
            2 => fall_started.elapsed() >= Duration::from_millis(230),
            3 => shift_started.is_some_and(|time| time.elapsed() >= Duration::from_millis(60)),
            4 => shift_started.is_some_and(|time| time.elapsed() >= Duration::from_millis(300)),
            5 => rapid_started.elapsed() >= Duration::from_millis(60),
            6 => rapid_started.elapsed() >= Duration::from_millis(230),
            7 => rapid_shift_started.is_some_and(|time| time.elapsed() >= Duration::from_millis(300)),
            _ => unreachable!(),
        };
        if !ready { return; }
        let result = (|| -> Result<(), String> {
            match stage {
                0 => {
                    let current = shots(&app);
                    let expected: Vec<_> = imported.iter().map(|item| item.id.clone()).collect();
                    check(&mut checks.borrow_mut(), "five photos run oldest to newest from left to right",
                        ids(&current) == expected && current.windows(2).all(|pair| pair[0].x < pair[1].x),
                        serde_json::json!({"ids": ids(&current), "expected": expected}))?;
                    snapshot(app.overlay.window(), &output, "01-five-photos.png")?;
                    green_start = photo_bounds(&app.overlay.window().take_snapshot()
                        .map_err(|e| e.to_string())?, true);
                    green_old_x = current[1].x;
                    initial = current;
                    app.overlay.set_reduced_motion(false);
                    imported.push(import(&app, 5)?);
                    app.refresh();
                    fall_started = Instant::now();
                    let falling = app.drop_model.row_data(0).ok_or("Oldest photo did not fall")?;
                    let current = shots(&app);
                    let pending = app.pending_shots.borrow();
                    let target = pending.as_ref().ok_or("Overflow did not defer its final layout")?;
                    green_target_x = target[0].x;
                    check(&mut checks.borrow_mut(), "sixth arrival removes only the oldest while four survivors keep their slots",
                        current.len() == 4 && falling.id == imported[0].id
                            && ids(&current) == imported[1..5].iter().map(|item| item.id.clone()).collect::<Vec<_>>()
                            && positions_unchanged(&initial, &current)
                            && ids(target) == imported[1..6].iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
                        serde_json::json!({"falling_id": falling.id.to_string(), "hanging_count": current.len(),
                            "pending_count": target.len(), "old_green_x": green_old_x, "pending_green_x": green_target_x}))?;
                    drop(pending);
                    check(&mut checks.borrow_mut(), "evicted cache is deleted while its in-memory texture survives",
                        !old_path.exists() && !old_thumb.exists()
                            && falling.thumbnail.size().width > 0 && falling.thumbnail.size().height > 0,
                        serde_json::json!({"cache_exists": old_path.exists(), "thumbnail_exists": old_thumb.exists(),
                            "texture_width": falling.thumbnail.size().width}))?;
                    app.capture_image.replace(Some(image::RgbaImage::new(1, 1)));
                    app.advance_capture_motion();
                    check(&mut checks.borrow_mut(), "capture image waits intact for the falling phase",
                        app.capture_image.borrow().is_some() && app.capture_started.get().is_none(),
                        serde_json::json!({"image_retained": app.capture_image.borrow().is_some()}))?;
                    app.capture_image.borrow_mut().take();
                }
                1 => {
                    let rendered_green = photo_bounds(&app.overlay.window().take_snapshot()
                        .map_err(|e| e.to_string())?, true);
                    let scale = app.overlay.window().scale_factor();
                    check(&mut checks.borrow_mut(), "survivor pixels stay in the old slot at the start of the fall",
                        green_has_not_moved(green_start, rendered_green, scale)
                            && positions_unchanged(&initial, &shots(&app)) && app.pending_shots.borrow().is_some(),
                        serde_json::json!({"old_green_bounds": green_start, "rendered_green_bounds": rendered_green,
                            "fall_elapsed_ms": fall_started.elapsed().as_millis(), "physical_scale": scale}))?;
                    red_start = snapshot(app.drop_window.window(), &output, "02-released.png")?;
                    check(&mut checks.borrow_mut(), "released photo renders before falling out of view",
                        red_start.is_some(), serde_json::json!({"bounds": red_start}))?;
                    let monitor = app.monitor.get().ok_or("Display unavailable")?;
                    let handle = hwnd(app.drop_window.window()).ok_or("Drop HWND unavailable")?;
                    let styles = unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32;
                    let required = WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0;
                    let mut rect = RECT::default();
                    unsafe { GetWindowRect(handle, &mut rect) }.map_err(|e| e.to_string())?;
                    let under = unsafe { WindowFromPoint(POINT { x: monitor.x + monitor.width as i32 / 2,
                        y: monitor.y + monitor.height as i32 / 2 }) };
                    let native_ok = styles & required == required && under != handle
                        && unsafe { GetForegroundWindow() } == foreground
                        && rect.left == monitor.x && rect.top == monitor.y
                        && rect.right - rect.left == monitor.width as i32
                        && rect.bottom - rect.top == monitor.height as i32;
                    check(&mut checks.borrow_mut(), "full-screen animation passes desktop input through and preserves focus",
                        native_ok, serde_json::json!({"styles": styles, "desktop_hit_passes_through": under != handle,
                            "foreground_unchanged": unsafe { GetForegroundWindow() } == foreground,
                            "bounds": [rect.left, rect.top, rect.right, rect.bottom]}))?;
                }
                2 => {
                    let rendered_green = photo_bounds(&app.overlay.window().take_snapshot()
                        .map_err(|e| e.to_string())?, true);
                    let middle = snapshot(app.drop_window.window(), &output, "03-falling-before-shift.png")?;
                    check(&mut checks.borrow_mut(), "oldest photo moves down while survivor pixels remain stationary",
                        red_start.zip(middle).is_some_and(|(start, next)| next[1] > start[1] + 20)
                            && green_has_not_moved(green_start, rendered_green, app.overlay.window().scale_factor())
                            && positions_unchanged(&initial, &shots(&app)) && app.pending_shots.borrow().is_some(),
                        serde_json::json!({"released_bounds": red_start, "falling_bounds": middle,
                            "old_green_bounds": green_start, "rendered_green_bounds": rendered_green,
                            "fall_elapsed_ms": fall_started.elapsed().as_millis()}))?;
                }
                3 => {
                    let rendered_green = photo_bounds(&app.overlay.window().take_snapshot()
                        .map_err(|e| e.to_string())?, true);
                    let scale = app.overlay.window().scale_factor();
                    let target_left = green_start.map(|bounds| bounds[0] as f32
                        + (green_target_x - green_old_x) * scale);
                    check(&mut checks.borrow_mut(), "survivor starts sliding only after the falling row is fully removed",
                        app.drop_model.row_count() == 0 && app.pending_shots.borrow().is_none()
                            && green_start.zip(rendered_green).zip(target_left).is_some_and(|((old, rendered), target)|
                                rendered[0] as f32 > target + 3.0 * scale
                                    && (rendered[0] as f32) < old[0] as f32 - 3.0 * scale),
                        serde_json::json!({"old_green_bounds": green_start, "rendered_green_bounds": rendered_green,
                            "expected_target_left": target_left,
                            "shift_elapsed_ms": shift_started.map(|time| time.elapsed().as_millis()),
                            "fall_elapsed_ms": fall_started.elapsed().as_millis()}))?;
                    snapshot(app.overlay.window(), &output, "04-shift-after-fall.png")?;
                }
                4 => {
                    let handle = hwnd(app.drop_window.window()).ok_or("Drop HWND disappeared")?;
                    check(&mut checks.borrow_mut(), "completed falls release textures and hide the native helper",
                        app.drop_model.row_count() == 0 && app.drop_expiry.borrow().is_empty()
                            && !unsafe { IsWindowVisible(handle) }.as_bool(),
                        serde_json::json!({"falling_count": app.drop_model.row_count(),
                            "native_visible": unsafe { IsWindowVisible(handle) }.as_bool()}))?;
                    let current = shots(&app);
                    check(&mut checks.borrow_mut(), "the sixth photo appears at the right after the gap closes",
                        ids(&current) == imported[1..6].iter().map(|item| item.id.clone()).collect::<Vec<_>>()
                            && current.windows(2).all(|pair| pair[0].x < pair[1].x),
                        serde_json::json!({"ids": ids(&current)}))?;
                    rapid_positions = current;
                    imported.push(import(&app, 6)?);
                    app.refresh();
                    rapid_started = Instant::now();
                    first_rapid_expiry = app.drop_expiry.borrow().first().copied();
                }
                5 => {
                    rapid_green_start = photo_bounds(&app.drop_window.window().take_snapshot()
                        .map_err(|e| e.to_string())?, true);
                    imported.push(import(&app, 7)?);
                    app.refresh();
                    let current = shots(&app);
                    let pending = app.pending_shots.borrow();
                    check(&mut checks.borrow_mut(), "rapid arrivals keep independent falls without restarting the first deadline",
                        app.drop_model.row_count() == 2
                            && app.drop_expiry.borrow().first().copied() == first_rapid_expiry
                            && app.drop_model.row_data(0).is_some_and(|shot| shot.id == imported[1].id)
                            && app.drop_model.row_data(1).is_some_and(|shot| shot.id == imported[2].id),
                        serde_json::json!({"falling_count": app.drop_model.row_count(),
                            "first_deadline_unchanged": app.drop_expiry.borrow().first().copied() == first_rapid_expiry}))?;
                    check(&mut checks.borrow_mut(), "rapid arrivals defer the newest complete layout and keep surviving slots fixed",
                        current.len() == 3 && positions_unchanged(&rapid_positions, &current)
                            && ids(&current) == imported[3..6].iter().map(|item| item.id.clone()).collect::<Vec<_>>()
                            && pending.as_ref().is_some_and(|target| ids(target)
                                == imported[3..8].iter().map(|item| item.id.clone()).collect::<Vec<_>>()),
                        serde_json::json!({"hanging_count": current.len(),
                            "pending_count": pending.as_ref().map(Vec::len)}))?;
                }
                6 => {
                    let green_middle = photo_bounds(&app.drop_window.window().take_snapshot()
                        .map_err(|e| e.to_string())?, true);
                    check(&mut checks.borrow_mut(), "a second arrival does not restart the first fall or move its survivors",
                        rapid_green_start.zip(green_middle).is_some_and(|(start, next)| next[1] > start[1] + 20)
                            && positions_unchanged(&rapid_positions, &shots(&app))
                            && app.pending_shots.borrow().is_some(),
                        serde_json::json!({"released_green_bounds": rapid_green_start,
                            "falling_green_bounds": green_middle, "elapsed_ms": rapid_started.elapsed().as_millis()}))?;
                }
                7 => {
                    let current = shots(&app);
                    check(&mut checks.borrow_mut(), "rapid arrivals settle once into the latest five photos in chronological order",
                        app.drop_model.row_count() == 0 && app.drop_expiry.borrow().is_empty()
                            && app.pending_shots.borrow().is_none() && !app.drop_window.window().is_visible()
                            && ids(&current) == imported[3..8].iter().map(|item| item.id.clone()).collect::<Vec<_>>()
                            && current.windows(2).all(|pair| pair[0].x < pair[1].x),
                        serde_json::json!({"ids": ids(&current), "hanging_count": current.len()}))?;
                    snapshot(app.overlay.window(), &output, "05-final-latest-five.png")?;
                    app.overlay.set_reduced_motion(true);
                    imported.push(import(&app, 8)?);
                    app.refresh();
                    let current = shots(&app);
                    check(&mut checks.borrow_mut(), "reduced motion evicts immediately without a falling helper",
                        app.drop_model.row_count() == 0 && !app.drop_window.window().is_visible()
                            && app.pending_shots.borrow().is_none()
                            && current.len() == 5 && current.first().is_some_and(|shot| shot.id == imported[4].id)
                            && current.last().is_some_and(|shot| shot.id == imported[8].id),
                        serde_json::json!({"falling_count": app.drop_model.row_count(), "hanging_count": current.len()}))?;
                    app.overlay.set_reduced_motion(false);
                    // This color was evicted long ago, so its fresh import is not a duplicate.
                    let last = import(&app, 0)?;
                    app.refresh();
                    let was_pending = app.pending_shots.borrow().is_some();
                    app.hide_line();
                    check(&mut checks.borrow_mut(), "hiding the line cancels pending layout and falling textures together",
                        was_pending && app.pending_shots.borrow().is_none()
                            && app.drop_model.row_count() == 0 && app.drop_expiry.borrow().is_empty()
                            && !app.drop_window.window().is_visible(),
                        serde_json::json!({"was_pending": was_pending,
                            "pending_after_hide": app.pending_shots.borrow().is_some()}))?;
                    app.show_line(true);
                    app.refresh();
                    let restored = shots(&app);
                    check(&mut checks.borrow_mut(), "showing the line restores the newest layout without stale deferred work",
                        restored.len() == 5 && restored.last().is_some_and(|shot| shot.id == last.id)
                            && app.pending_shots.borrow().is_none() && app.drop_model.row_count() == 0,
                        serde_json::json!({"hanging_count": restored.len(), "last_id": last.id}))?;
                    check(&mut checks.borrow_mut(), "verification preserves the user clipboard",
                        native::clipboard_sequence() == clipboard,
                        serde_json::json!({"before": clipboard, "after": native::clipboard_sequence()}))?;
                    app.overlay.hide().map_err(|e| e.to_string())?;
                }
                _ => unreachable!(),
            }
            Ok(())
        })();
        if let Err(message) = result {
            *error.borrow_mut() = Some(message);
            let _ = slint::quit_event_loop();
            return;
        }
        stage += 1;
        if stage == 8 { let _ = slint::quit_event_loop(); }
    });
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    let checks = reports.borrow();
    let passed = failure.borrow().is_none()
        && checks.len() == 19
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
        Err(failure.borrow().clone().unwrap_or_else(|| {
            format!(
                "Overflow check failed; see {}",
                directory.join("report.json").display()
            )
        }))
    }
}
