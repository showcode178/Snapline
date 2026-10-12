//! Real Slint interactions with an isolated profile; clipboard writes are intercepted.
use crate::{App, hwnd, native, storage};
use slint::{
    ComponentHandle, Model,
    platform::{PointerEventButton, WindowEvent},
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

fn click(window: &slint::Window, position: slint::LogicalPosition, button: PointerEventButton) {
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn shelf_shot(app: &App, id: &str) -> Result<crate::Shot, String> {
    app.overlay
        .get_shots()
        .iter()
        .find(|shot| shot.id.as_str() == id)
        .ok_or_else(|| format!("Missing shelf photo: {id}"))
}

fn photo_position(app: &App, id: &str) -> Result<slint::LogicalPosition, String> {
    let shot = shelf_shot(app, id)?;
    Ok(slint::LogicalPosition::new(
        shot.x + shot.card_width / 2.0,
        shot.y + 16.0 + shot.card_height / 2.0,
    ))
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
        Err(format!("Edit interaction check failed: {name}"))
    }
}

pub fn run(app: Rc<App>, directory: PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let source = directory.join("imported-source.png");
    let anchor_source = directory.join("anchor-source.png");
    image::RgbaImage::from_pixel(360, 240, image::Rgba([42, 116, 182, 255]))
        .save(&source)
        .map_err(|e| e.to_string())?;
    image::RgbaImage::from_pixel(80, 100, image::Rgba([80, 160, 80, 255]))
        .save(&anchor_source)
        .map_err(|e| e.to_string())?;
    let source_before = std::fs::read(&source).map_err(|e| e.to_string())?;
    let (initial, anchor, initial_path, initial_thumb, initial_slot, anchor_slot) = {
        let mut store = app.store.lock().unwrap();
        if !store.items.is_empty() {
            return Err("Edit check requires an empty isolated profile".into());
        }
        let anchor = store
            .import(&anchor_source)?
            .ok_or("Anchor import was deduplicated")?;
        let initial = store
            .import(&source)?
            .ok_or("Photo import was deduplicated")?;
        let path = store.path(&initial);
        let thumb = std::fs::read(store.thumb(&initial)).map_err(|e| e.to_string())?;
        let initial_slot = store.items.iter().position(|item| item.id == initial.id);
        let anchor_slot = store.items.iter().position(|item| item.id == anchor.id);
        (initial, anchor, path, thumb, initial_slot, anchor_slot)
    };
    app.refresh();
    app.overlay.set_reduced_motion(true);
    app.show_line(true);
    let clipboard_before = native::clipboard_sequence();
    let actions = Rc::new(RefCell::new(Vec::<String>::new()));
    let recorded = actions.clone();
    let weak = Rc::downgrade(&app);
    app.overlay.on_item_action(move |action, id| {
        recorded.borrow_mut().push(action.to_string());
        if action != "copy" {
            if let Some(app) = weak.upgrade() {
                app.item_action(&action, &id);
            }
        }
    });
    let reports = Rc::new(RefCell::new(Vec::<serde_json::Value>::new()));
    let failure = Rc::new(RefCell::new(None::<String>));
    let checks = reports.clone();
    let error = failure.clone();
    let output = directory.clone();
    let timer = slint::Timer::default();
    let mut stage = 0;
    let mut latest = initial.clone();
    let mut prior_thumb = initial_thumb;
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(650), move || {
        let result = (|| -> Result<(), String> {
            match stage {
                0 => {
                    let monitor = app.monitor.get().ok_or("Display unavailable")?;
                    let handle = hwnd(app.overlay.window()).ok_or("Overlay HWND unavailable")?;
                    native::configure_overlay(handle).map_err(|e| e.to_string())?;
                    native::set_click_through(handle, true).map_err(|e| e.to_string())?;
                    native::place_overlay(handle, monitor, 300.0).map_err(|e| e.to_string())?;
                    snapshot(app.overlay.window(), &output, "01-shelf.png")?;
                    click(app.overlay.window(), photo_position(&app, &initial.id)?, PointerEventButton::Left);
                    check(&mut checks.borrow_mut(), "single left click does not copy or open",
                        actions.borrow().is_empty() && !app.preview.window().is_visible(),
                        serde_json::json!({"actions": *actions.borrow()}))?;
                }
                1 => {
                    let position = photo_position(&app, &initial.id)?;
                    click(app.overlay.window(), position, PointerEventButton::Left);
                    click(app.overlay.window(), position, PointerEventButton::Left);
                }
                2 => {
                    check(&mut checks.borrow_mut(), "double left click opens the internal editor",
                        actions.borrow().as_slice() == ["preview"]
                            && app.preview.window().is_visible()
                            && app.preview_id.borrow().as_deref() == Some(initial.id.as_str()),
                        serde_json::json!({"actions": *actions.borrow(),
                            "editor_visible": app.preview.window().is_visible(),
                            "preview_id": *app.preview_id.borrow()}))?;
                    snapshot(app.preview.window(), &output, "02-editor-open.png")?;
                    app.preview.set_tool_index(0);
                    app.preview.invoke_pointer_event("down".into(), 20.0, 25.0);
                    app.preview.invoke_pointer_event("move".into(), 140.0, 70.0);
                    app.preview.invoke_pointer_event("up".into(), 140.0, 70.0);
                    check(&mut checks.borrow_mut(), "annotation marks the editor dirty",
                        app.preview.get_dirty() && app.preview.get_can_undo(), serde_json::json!({}))?;
                    snapshot(app.preview.window(), &output, "03-annotated.png")?;
                    app.preview.invoke_save();
                    let store = app.store.lock().unwrap();
                    let saved_id = app.preview_id.borrow().clone().ok_or("Missing saved photo id")?;
                    let item = store.items.iter().find(|item| item.id == saved_id)
                        .ok_or("Saved photo disappeared")?.clone();
                    let path = store.path(&item);
                    let thumb = std::fs::read(store.thumb(&item)).map_err(|e| e.to_string())?;
                    let persisted = storage::load_image(&path)?.to_rgba8();
                    let image_matches = app.edit.borrow().as_ref()
                        .is_some_and(|edit| edit.image == persisted);
                    let passed = store.items.len() == 2 && item.id != initial.id
                        && path != initial_path && thumb != prior_thumb && image_matches
                        && store.items.iter().position(|candidate| candidate.id == item.id) == initial_slot
                        && store.items.iter().position(|candidate| candidate.id == anchor.id) == anchor_slot
                        && item.name == initial.name
                        && item.created_at == initial.created_at && !app.preview.get_dirty()
                        && app.preview_id.borrow().as_deref() == Some(item.id.as_str())
                        && app.preview_path.borrow().as_ref() == Some(&path);
                    check(&mut checks.borrow_mut(), "Save replaces the shelf photo and thumbnail",
                        passed, serde_json::json!({"item_count": store.items.len(),
                            "old_id": initial.id, "new_id": item.id,
                            "preview_id": *app.preview_id.borrow(), "image_matches_editor": image_matches,
                            "editor_status": app.preview.get_status().to_string()}))?;
                    latest = item;
                    prior_thumb = thumb;
                }
                3 => {
                    app.preview.set_tool_index(3);
                    app.preview.invoke_pointer_event("down".into(), 10.0, 10.0);
                    app.preview.invoke_pointer_event("up".into(), 119.0, 89.0);
                    app.preview.invoke_save();
                    let store = app.store.lock().unwrap();
                    let saved_id = app.preview_id.borrow().clone().ok_or("Missing replacement photo id")?;
                    let item = store.items.iter().find(|item| item.id == saved_id)
                        .ok_or("Second save lost the photo")?.clone();
                    let thumb = std::fs::read(store.thumb(&item)).map_err(|e| e.to_string())?;
                    let shot = shelf_shot(&app, &item.id)?;
                    let passed = store.items.len() == 2
                        && store.items.iter().position(|candidate| candidate.id == item.id) == initial_slot
                        && store.items.iter().position(|candidate| candidate.id == anchor.id) == anchor_slot
                        && item.id != latest.id && item.width == 110 && item.height == 80
                        && item.name == latest.name && item.created_at == latest.created_at
                        && thumb != prior_thumb && shot.id == item.id && !app.preview.get_dirty()
                        && app.preview_id.borrow().as_deref() == Some(item.id.as_str())
                        && app.preview_path.borrow().as_ref() == Some(&store.path(&item));
                    check(&mut checks.borrow_mut(), "repeated Save updates the same slot and cropped dimensions",
                        passed, serde_json::json!({"item_count": store.items.len(), "width": item.width,
                            "height": item.height, "old_id": latest.id, "new_id": item.id,
                            "shelf_id": shot.id.to_string()}))?;
                    latest = item;
                    snapshot(app.preview.window(), &output, "04-saved-crop.png")?;
                    app.preview.hide().map_err(|e| e.to_string())?;
                }
                4 => {
                    let position = photo_position(&app, &latest.id)?;
                    click(app.overlay.window(), position, PointerEventButton::Left);
                    click(app.overlay.window(), position, PointerEventButton::Left);
                }
                5 => {
                    let passed = app.preview.window().is_visible()
                        && app.preview.get_image_width() == 110 && app.preview.get_image_height() == 80
                        && app.preview_id.borrow().as_deref() == Some(latest.id.as_str())
                        && actions.borrow().as_slice() == ["preview", "preview"];
                    check(&mut checks.borrow_mut(), "double click reopens the saved replacement",
                        passed, serde_json::json!({"preview_id": *app.preview_id.borrow(),
                            "width": app.preview.get_image_width(), "height": app.preview.get_image_height()}))?;
                    app.preview.hide().map_err(|e| e.to_string())?;
                    actions.borrow_mut().clear();
                    click(app.overlay.window(), photo_position(&app, &latest.id)?, PointerEventButton::Right);
                    check(&mut checks.borrow_mut(), "right click dispatches copy only",
                        actions.borrow().as_slice() == ["copy"] && !app.preview.window().is_visible(),
                        serde_json::json!({"actions": *actions.borrow(), "clipboard_write_intercepted": true}))?;
                    let source_after = std::fs::read(&source).map_err(|e| e.to_string())?;
                    check(&mut checks.borrow_mut(), "imported source remains unchanged",
                        source_after == source_before, serde_json::json!({"source": source}))?;
                    let clipboard_after = native::clipboard_sequence();
                    check(&mut checks.borrow_mut(), "user clipboard remains unchanged",
                        clipboard_before == clipboard_after,
                        serde_json::json!({"before": clipboard_before, "after": clipboard_after}))?;
                    snapshot(app.overlay.window(), &output, "05-shelf-replaced.png")?;
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
        if stage == 6 { let _ = slint::quit_event_loop(); }
    });
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    let checks = reports.borrow();
    let passed = failure.borrow().is_none()
        && checks.len() == 9
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
                "Edit interaction check failed; see {}",
                directory.join("report.json").display()
            )
        }))
    }
}
