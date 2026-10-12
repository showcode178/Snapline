//! Settings UI checks use an independent window and never save a user profile.
use crate::{CaptureWindow, DropWindow, OverlayWindow, SettingsWindow, native};
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

fn click(window: &slint::Window, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn key(window: &slint::Window, key: Key) {
    let text: slint::SharedString = key.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn navigate(settings: &SettingsWindow, section: i32) {
    click(settings.window(), 78.0, 147.0 + section as f32 * 53.0);
}

fn snapshot(window: &slint::Window, directory: &Path, name: &str) -> Result<(), String> {
    let buffer = window.take_snapshot().map_err(|e| e.to_string())?;
    image::save_buffer(
        directory.join(name),
        buffer.as_bytes(),
        buffer.width(),
        buffer.height(),
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}

fn check(checks: &mut Vec<serde_json::Value>, name: &str, passed: bool) -> Result<(), String> {
    checks.push(serde_json::json!({"name": name, "passed": passed}));
    // Finish the visual batch even when an interaction assertion fails.
    Ok(())
}

pub fn run(directory: PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    // Keep the app's first three transparent helper-window slots in the backend hook.
    let _overlay = OverlayWindow::new().map_err(|e| e.to_string())?;
    let _capture = CaptureWindow::new().map_err(|e| e.to_string())?;
    let _drop = DropWindow::new().map_err(|e| e.to_string())?;
    let settings = Rc::new(SettingsWindow::new().map_err(|e| e.to_string())?);
    settings.set_watch_enabled(false);
    settings.set_watch_folder(
        "C:\\Users\\截图用户\\Pictures\\Screenshots\\Work and personal captures".into(),
    );
    settings.set_max_items(5);
    settings.set_size_index(2);
    settings.set_theme_index(1);
    let clipboard_before = native::clipboard_sequence();
    let browse_count = Rc::new(Cell::new(0));
    let save_count = Rc::new(Cell::new(0));
    let close_count = Rc::new(Cell::new(0));
    let about_count = Rc::new(Cell::new(0));
    {
        let count = browse_count.clone();
        settings.on_choose_folder(move || count.set(count.get() + 1));
        let count = save_count.clone();
        settings.on_save(move || count.set(count.get() + 1));
        let count = close_count.clone();
        settings.on_done(move || count.set(count.get() + 1));
        let count = about_count.clone();
        settings.on_about(move || count.set(count.get() + 1));
    }
    let reports = Rc::new(RefCell::new(Vec::<serde_json::Value>::new()));
    let failure = Rc::new(RefCell::new(None::<String>));
    let checks = reports.clone();
    let error = failure.clone();
    let output = directory.clone();
    let timer = slint::Timer::default();
    settings.show().map_err(|e| e.to_string())?;
    let window = settings.clone();
    let recorded_counts = (
        browse_count.clone(),
        save_count.clone(),
        close_count.clone(),
        about_count.clone(),
    );
    let mut stage = 0;
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(350), move || {
        let result = (|| -> Result<(), String> {
            let phase = stage / 2;
            let section = phase % 4;
            if stage % 2 == 0 {
                let english = phase >= 4;
                window.set_language_index(if english { 1 } else { 0 });
                window.set_english(english);
                window.set_theme_index(if english { 2 } else { 1 });
                window.set_dark(english);
                if phase == 8 {
                    window.window().set_size(slint::LogicalSize::new(660.0, 560.0));
                    window.set_status("Choose an existing screenshot folder first. Please browse to the folder where your screenshots are saved.".into());
                }
                navigate(&window, section);
                check(&mut checks.borrow_mut(), &format!("navigate page {phase}"),
                    window.get_section_index() == section)?;
            } else {
                let variant = if phase < 4 { "zh-light" } else if phase < 8 { "en-dark" } else { "small-en-dark" };
                let name = ["sources", "line", "appearance", "startup"][section as usize];
                snapshot(window.window(), &output, &format!("{variant}-{name}.png"))?;
                if phase == 0 {
                    // Coordinates below refer to the settings content, in logical pixels.
                    let right = window.window().size().to_logical(window.window().scale_factor()).width;
                    click(window.window(), right - 49.0, 234.0);
                    check(&mut checks.borrow_mut(), "clipboard checkbox responds to a pointer click", !window.get_clipboard_enabled())?;
                    key(window.window(), Key::Space);
                    check(&mut checks.borrow_mut(), "clipboard checkbox responds to Space", window.get_clipboard_enabled())?;
                    click(window.window(), right - 49.0, 234.0);
                    click(window.window(), right - 49.0, 326.0);
                    check(&mut checks.borrow_mut(), "sound checkbox keeps its two-way binding", !window.get_capture_sound())?;
                    click(window.window(), right - 67.0, 482.0);
                    check(&mut checks.borrow_mut(), "disabled folder button does not open a picker", browse_count.get() == 0)?;
                    click(window.window(), right - 49.0, 418.0);
                    check(&mut checks.borrow_mut(), "folder checkbox keeps its two-way binding", window.get_watch_enabled())?;
                    click(window.window(), right - 67.0, 482.0);
                    check(&mut checks.borrow_mut(), "enabled folder button dispatches its callback", browse_count.get() == 1)?;
                    navigate(&window, 0);
                    window.set_section_index(1);
                    key(window.window(), Key::Space);
                    check(&mut checks.borrow_mut(), "navigation accepts keyboard activation", window.get_section_index() == 0)?;
                }
                if phase == 1 {
                    let right = window.window().size().to_logical(window.window().scale_factor()).width;
                    click(window.window(), right - 49.0, 418.0);
                    check(&mut checks.borrow_mut(), "fullscreen checkbox keeps its two-way binding", window.get_show_over_fullscreen())?;
                }
                if phase == 2 {
                    let right = window.window().size().to_logical(window.window().scale_factor()).width;
                    click(window.window(), right - 113.0, 234.0);
                    key(window.window(), Key::Escape);
                    key(window.window(), Key::DownArrow);
                    check(&mut checks.borrow_mut(), "language selection immediately previews English", window.get_language_index() == 1 && window.get_english())?;
                    key(window.window(), Key::UpArrow);
                    check(&mut checks.borrow_mut(), "language selection immediately restores Chinese", window.get_language_index() == 0 && !window.get_english())?;
                    click(window.window(), right - 113.0, 326.0);
                    key(window.window(), Key::Escape);
                    key(window.window(), Key::DownArrow);
                    check(&mut checks.borrow_mut(), "theme selection immediately previews dark", window.get_theme_index() == 2 && window.get_dark())?;
                    key(window.window(), Key::UpArrow);
                    check(&mut checks.borrow_mut(), "theme selection immediately restores light", window.get_theme_index() == 1 && !window.get_dark())?;
                    window.set_system_dark(true);
                    key(window.window(), Key::UpArrow);
                    check(&mut checks.borrow_mut(), "system theme uses the supplied Windows appearance", window.get_theme_index() == 0 && window.get_dark())?;
                    key(window.window(), Key::DownArrow);
                    window.set_system_dark(false);
                }
                if phase == 3 {
                    let right = window.window().size().to_logical(window.window().scale_factor()).width;
                    click(window.window(), right - 49.0, 234.0);
                    check(&mut checks.borrow_mut(), "startup checkbox responds to a pointer click", window.get_autostart())?;
                    key(window.window(), Key::Space);
                    check(&mut checks.borrow_mut(), "startup checkbox responds to Space", !window.get_autostart())?;
                    check(&mut checks.borrow_mut(), "source drafts survive page switches",
                        !window.get_clipboard_enabled() && !window.get_capture_sound()
                            && window.get_watch_enabled() && window.get_max_items() == 5
                            && window.get_size_index() == 2 && window.get_show_over_fullscreen())?;
                }
                if phase == 11 {
                    let size = window.window().size().to_logical(window.window().scale_factor());
                    click(window.window(), size.width - 55.0, size.height - 38.0);
                    key(window.window(), Key::Space);
                    check(&mut checks.borrow_mut(), "Save dispatches by pointer and keyboard", save_count.get() == 2)?;
                    click(window.window(), size.width - 200.0, size.height - 38.0);
                    click(window.window(), 60.0, size.height - 38.0);
                    check(&mut checks.borrow_mut(), "Close and About dispatch their callbacks", close_count.get() == 1 && about_count.get() == 1)?;
                }
            }
            Ok(())
        })();
        if let Err(message) = result {
            *error.borrow_mut() = Some(message);
            let _ = slint::quit_event_loop();
            return;
        }
        stage += 1;
        if stage == 24 {
            let result = check(&mut checks.borrow_mut(), "user clipboard is unchanged", native::clipboard_sequence() == clipboard_before);
            if let Err(message) = result { *error.borrow_mut() = Some(message); }
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    let checks = reports.borrow();
    if failure.borrow().is_none() {
        *failure.borrow_mut() = checks
            .iter()
            .find(|check| check["passed"] != true)
            .map(|check| {
                format!(
                    "Settings check failed: {}",
                    check["name"].as_str().unwrap_or("unknown")
                )
            });
    }
    let passed = failure.borrow().is_none() && checks.iter().all(|check| check["passed"] == true);
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed": passed, "error": *failure.borrow(), "checks": *checks,
            "clipboard_before": clipboard_before, "clipboard_after": native::clipboard_sequence(),
            "callbacks": {"browse": recorded_counts.0.get(), "save": recorded_counts.1.get(), "close": recorded_counts.2.get(), "about": recorded_counts.3.get()}
        })).map_err(|e| e.to_string())?,
    ).map_err(|e| e.to_string())?;
    if passed {
        Ok(())
    } else {
        Err(failure
            .borrow()
            .clone()
            .unwrap_or_else(|| "Settings check failed".into()))
    }
}
