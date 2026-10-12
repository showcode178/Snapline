#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod capture_check;
mod edit_check;
mod editor;
mod monitor;
mod native;
mod overflow_check;
mod settings_check;
mod storage;

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    winit_030::{WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use storage::{Settings, Store};
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use windows::Win32::Foundation::HWND;

slint::include_modules!();

struct Gesture {
    id: String,
    x: f32,
    y: f32,
}
struct App {
    overlay: OverlayWindow,
    shots_model: Rc<VecModel<Shot>>,
    capture_window: CaptureWindow,
    drop_window: DropWindow,
    drop_model: Rc<VecModel<Shot>>,
    drop_expiry: RefCell<Vec<Instant>>,
    pending_shots: RefCell<Option<Vec<Shot>>>,
    capture_pending: RefCell<Option<String>>,
    capture_image: RefCell<Option<image::RgbaImage>>,
    capture_started: Cell<Option<Instant>>,
    settings_window: SettingsWindow,
    preview: PreviewWindow,
    about: AboutWindow,
    store: Arc<Mutex<Store>>,
    settings: RefCell<Settings>,
    sender: mpsc::Sender<monitor::Event>,
    ignored_sequence: Arc<AtomicU32>,
    hotkeys: GlobalHotKeyManager,
    hotkey: Cell<Option<HotKey>>,
    monitor: Cell<Option<native::MonitorInfo>>,
    hwnd: Cell<HWND>,
    hit_test: Cell<bool>,
    configured: Cell<bool>,
    suppress: Cell<bool>,
    pinned_until: Cell<Instant>,
    last_inside: Cell<Instant>,
    status_until: Cell<Instant>,
    page: Cell<usize>,
    refresh_pending: Cell<bool>,
    gesture: RefCell<Option<Gesture>>,
    edit: RefCell<Option<editor::Editor>>,
    preview_id: RefCell<Option<String>>,
    preview_path: RefCell<Option<PathBuf>>,
    tray: RefCell<Option<TrayIcon>>,
    tray_items: RefCell<Vec<MenuItem>>,
}
fn hwnd(window: &slint::Window) -> Option<HWND> {
    window
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
            _ => None,
        })
        .flatten()
}
fn activate_window(window: &slint::Window) {
    if let Some(handle) = hwnd(window) {
        native::activate_window(handle);
    }
}
fn image_from_rgba(image: &image::RgbaImage) -> slint::Image {
    slint::Image::from_rgba8(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            image.as_raw(),
            image.width(),
            image.height(),
        ),
    )
}
fn msg(english: bool, zh: &str, en: &str) -> String {
    if english { en.into() } else { zh.into() }
}
fn parse_hotkey(
    text: &str,
) -> std::result::Result<HotKey, global_hotkey::hotkey::HotKeyParseError> {
    text.split('+')
        .map(|token| {
            if token.trim().eq_ignore_ascii_case("win")
                || token.trim().eq_ignore_ascii_case("windows")
            {
                "Super"
            } else {
                token.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
        .parse()
}
impl App {
    fn english(&self) -> bool {
        self.settings.borrow().english()
    }
    fn dark(&self) -> bool {
        match self.settings.borrow().theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => native::system_dark(),
        }
    }
    fn status(&self, text: impl Into<String>) {
        self.overlay.set_status(text.into().into());
        self.status_until
            .set(Instant::now() + Duration::from_secs(6));
    }
    fn refresh(&self) {
        let Ok(mut store) = self.store.try_lock() else {
            self.refresh_pending.set(true);
            return;
        };
        self.refresh_pending.set(false);
        let width = self
            .monitor
            .get()
            .map_or(1280.0, |m| m.width as f32 / m.scale as f32);
        let multiplier = match self.settings.borrow().size.as_str() {
            "small" => 0.8,
            "large" => 1.35,
            _ => 1.0,
        };
        let card_width = 150.0 * multiplier;
        let card_height = 104.0 * multiplier;
        let count = ((width - 48.0) / (card_width + 18.0)).floor().max(1.0) as usize;
        let pages = store.items.len().div_ceil(count).max(1);
        let page = self.page.get().min(pages - 1);
        self.page.set(page);
        let items = store
            .items
            .iter()
            .rev()
            .skip(page * count)
            .take(count)
            .collect::<Vec<_>>();
        let occupied = items.len() as f32 * (card_width + 18.0) - 18.0;
        let shots = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let slot_x = (width - occupied) / 2.0 + index as f32 * (card_width + 18.0);
                let t = (slot_x + card_width / 2.0 + 20.0) / (width + 40.0);
                let (photo_width, photo_height) =
                    if item.video || item.width == 0 || item.height == 0 {
                        (card_width, card_height)
                    } else {
                        let fit = ((card_width - 8.0) / item.width as f32)
                            .min((card_height - 8.0) / item.height as f32);
                        (
                            item.width as f32 * fit + 8.0,
                            item.height as f32 * fit + 8.0,
                        )
                    };
                let thumbnail = if item.video {
                    image_from_rgba(
                        &image::load_from_memory(include_bytes!("../assets/snapline.png"))
                            .unwrap()
                            .to_rgba8(),
                    )
                } else {
                    slint::Image::load_from_path(&store.thumb(item)).unwrap_or_default()
                };
                Shot {
                    id: item.id.clone().into(),
                    title: item.name.clone().into(),
                    thumbnail,
                    x: slot_x + (card_width - photo_width) / 2.0,
                    y: 1.5 + 120.0 * t * (1.0 - t),
                    card_width: photo_width,
                    card_height: photo_height,
                    angle: if index % 2 == 0 { -1.4 } else { 1.5 },
                    video: item.video,
                }
            })
            .collect::<Vec<_>>();
        let evicted = store.take_evicted();
        let falling = self
            .shots_model
            .iter()
            .filter(|shot| {
                evicted.iter().any(|id| id == shot.id.as_str())
                    && store.find(shot.id.as_str()).is_none()
            })
            .collect();
        drop(store);
        self.start_drop_motion(falling);
        if self.drop_model.row_count() > 0 {
            // Detach outgoing photos now; survivors stay still until every fall ends.
            for index in (0..self.shots_model.row_count()).rev() {
                let old = self.shots_model.row_data(index).unwrap();
                if !shots.iter().any(|shot| shot.id == old.id) {
                    self.shots_model.remove(index);
                }
            }
            self.pending_shots.replace(Some(shots));
        } else {
            self.pending_shots.borrow_mut().take();
            self.apply_shots(shots);
        }
        self.overlay
            .set_page_text(format!("{} / {}", page + 1, pages).into());
        self.overlay.set_english(self.english());
        self.overlay.set_dark(self.dark());
    }
    fn apply_shots(&self, shots: Vec<Shot>) {
        self.overlay.set_landed_id("".into());
        // Keep surviving rows alive so their positions animate when a slot opens.
        for index in (0..self.shots_model.row_count()).rev() {
            let old = self.shots_model.row_data(index).unwrap();
            if !shots.iter().any(|shot| shot.id == old.id) {
                self.shots_model.remove(index);
            }
        }
        for (index, shot) in shots.into_iter().enumerate() {
            if self
                .shots_model
                .row_data(index)
                .is_some_and(|old| old.id == shot.id)
            {
                self.shots_model.set_row_data(index, shot);
            } else {
                if let Some(previous) = self.shots_model.iter().position(|old| old.id == shot.id) {
                    self.shots_model.remove(previous);
                }
                self.shots_model.insert(index, shot);
            }
        }
    }
    fn show_line(&self, pin: bool) {
        self.suppress.set(false);
        if let Some(monitor) = native::monitor_at_cursor() {
            if self.monitor.get() != Some(monitor) {
                self.cancel_capture_motion();
                self.cancel_drop_motion();
                self.monitor.set(Some(monitor));
                self.refresh();
            }
        }
        self.overlay.set_revealed(true);
        let _ = self.overlay.show();
        self.configured.set(false);
        self.last_inside.set(Instant::now());
        if pin {
            self.pinned_until
                .set(Instant::now() + Duration::from_secs(4));
        }
    }
    fn hide_line(&self) {
        self.cancel_capture_motion();
        self.cancel_drop_motion();
        self.overlay.set_revealed(false);
        self.pinned_until.set(Instant::now());
        self.last_inside.set(Instant::now());
    }
    fn capture(&self) {
        self.cancel_capture_motion();
        self.cancel_drop_motion();
        self.suppress.set(true);
        self.overlay.set_revealed(false);
        let _ = self.overlay.hide();
        // Physically hide the native window before starting screen clipping.
        let sender = self.sender.clone();
        slint::Timer::single_shot(Duration::from_millis(150), move || {
            if let Err(error) = native::open_capture() {
                let _ = sender.send(monitor::Event::Error(error.to_string()));
            }
        });
    }
    fn import_files(&self) {
        self.cancel_capture_motion();
        self.cancel_drop_motion();
        self.suppress.set(true);
        let _ = self.overlay.hide();
        let selected = rfd::FileDialog::new()
            .set_title(msg(
                self.english(),
                "导入图片或视频",
                "Import images or videos",
            ))
            .add_filter(
                "Images / Video",
                &[
                    "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "mp4", "mov", "m4v",
                    "webm",
                ],
            )
            .pick_files();
        self.suppress.set(false);
        self.show_line(true);
        if let Some(paths) = selected {
            self.import_paths(paths);
        }
    }
    fn import_paths(&self, paths: Vec<PathBuf>) {
        self.cancel_capture_motion();
        let store = self.store.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            for path in paths {
                match store.lock().unwrap().import(&path) {
                    Ok(Some(item)) => {
                        let _ = sender.send(monitor::Event::Changed {
                            id: item.id,
                            capture: false,
                        });
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = sender.send(monitor::Event::Error(e));
                    }
                }
            }
        });
    }
    fn cancel_capture_motion(&self) {
        self.capture_pending.borrow_mut().take();
        self.capture_image.borrow_mut().take();
        self.capture_started.set(None);
        self.overlay.set_arriving_id("".into());
        let _ = self.capture_window.hide();
        self.capture_window.set_snapshot(slint::Image::default());
    }
    fn new_capture(&self, id: String) {
        self.cancel_capture_motion();
        let blocked = !self.settings.borrow().show_over_fullscreen
            && native::monitor_at_cursor().is_some_and(|m| native::fullscreen_on_monitor(&m));
        self.page.set(usize::MAX);
        self.refresh_pending.set(true);
        if blocked {
            return;
        }
        self.show_line(true);
        self.overlay.set_arriving_id(id.clone().into());
        self.capture_pending.replace(Some(id.clone()));
        let store = self.store.clone();
        let sender = self.sender.clone();
        // Decode the cached original away from the UI thread; stale results are ignored.
        std::thread::spawn(move || {
            let image = (|| {
                let path = {
                    let store = store.lock().map_err(|e| e.to_string())?;
                    let item = store.find(&id).ok_or("Capture no longer exists")?;
                    store.path(&item)
                };
                storage::load_image(&path).map(|image| image.thumbnail(1280, 960).to_rgba8())
            })();
            let _ = sender.send(monitor::Event::CaptureReady { id, image });
        });
    }
    fn cancel_drop_motion(&self) {
        self.drop_model.set_vec(Vec::new());
        self.drop_expiry.borrow_mut().clear();
        if self.pending_shots.borrow_mut().take().is_some() {
            self.refresh_pending.set(true);
        }
        let _ = self.drop_window.hide();
    }
    fn start_drop_motion(&self, shots: Vec<Shot>) {
        if shots.is_empty()
            || self.overlay.get_reduced_motion()
            || !self.overlay.get_revealed()
            || self.suppress.get()
        {
            return;
        }
        let Some(monitor) = self.monitor.get() else {
            return;
        };
        let height = monitor.height as f64 / monitor.scale;
        let first = self.drop_model.row_count() == 0;
        if first {
            self.drop_window.set_dark(self.dark());
            self.drop_window.set_reduced_motion(false);
            self.drop_window
                .window()
                .set_size(slint::PhysicalSize::new(monitor.width, monitor.height));
            self.drop_window
                .window()
                .set_position(slint::PhysicalPosition::new(monitor.x, monitor.y));
        }
        for shot in shots {
            self.drop_model.push(shot);
            self.drop_expiry
                .borrow_mut()
                .push(Instant::now() + Duration::from_millis(650));
        }
        if first {
            if self.drop_window.show().is_err() {
                self.cancel_drop_motion();
                return;
            }
            self.drop_window.window().with_winit_window(|win| {
                let _ = win.set_cursor_hittest(false);
            });
            if let Some(h) = hwnd(self.drop_window.window()) {
                let _ = native::configure_overlay(h);
                let _ = native::set_click_through(h, true);
                let _ = native::place_overlay(h, monitor, height);
            }
        }
    }
    fn advance_drop_motion(&self) {
        let now = Instant::now();
        let mut expiry = self.drop_expiry.borrow_mut();
        for index in (0..expiry.len()).rev() {
            if now >= expiry[index] {
                expiry.remove(index);
                self.drop_model.remove(index);
            }
        }
        let finished = expiry.is_empty();
        drop(expiry);
        if finished {
            let _ = self.drop_window.hide();
            if !self.refresh_pending.get() {
                let pending = self.pending_shots.borrow_mut().take();
                if let Some(shots) = pending {
                    self.apply_shots(shots);
                }
            }
        }
    }
    fn advance_capture_motion(&self) {
        // The new capture must not consume its decoded image before its slot is ready.
        if self.pending_shots.borrow().is_some() || self.drop_model.row_count() > 0 {
            return;
        }
        if let Some(started) = self.capture_started.get() {
            let elapsed = started.elapsed();
            if elapsed >= Duration::from_millis(700) {
                let id = self.capture_pending.borrow().clone().unwrap_or_default();
                self.overlay.set_landed_id(id.into());
                self.overlay
                    .set_landing_serial(self.overlay.get_landing_serial().wrapping_add(1).max(1));
                self.cancel_capture_motion();
            } else {
                self.capture_window.set_preparing(false);
                self.capture_window
                    .set_reduced_motion(self.overlay.get_reduced_motion());
                self.capture_window.set_flash(false);
                if elapsed >= Duration::from_millis(180) {
                    self.capture_window.set_land(true);
                }
            }
            return;
        }
        if self.refresh_pending.get() {
            return;
        }
        let Some(image) = self.capture_image.borrow_mut().take() else {
            return;
        };
        let id = self.capture_pending.borrow().clone().unwrap_or_default();
        let target = self
            .overlay
            .get_shots()
            .iter()
            .find(|s| s.id.as_str() == id);
        let (Some(shot), Some(monitor)) = (target, self.monitor.get()) else {
            self.cancel_capture_motion();
            return;
        };
        let reduced = !native::animations_enabled();
        self.overlay.set_reduced_motion(reduced);
        let w = &self.capture_window;
        // Reset while motion is disabled so a rapid second capture starts cleanly.
        w.set_preparing(true);
        w.set_land(false);
        w.set_flash(!reduced);
        w.set_snapshot(image_from_rgba(&image));
        w.set_dark(self.dark());
        let width = monitor.width as f32 / monitor.scale as f32;
        let height = monitor.height as f32 / monitor.scale as f32;
        let minimum_fit = ((shot.card_width - 8.0) / image.width() as f32)
            .max((shot.card_height - 8.0) / image.height() as f32)
            * 2.5;
        let fit = (width * 0.70 / image.width() as f32)
            .min(height * 0.60 / image.height() as f32)
            .min((1.0 / monitor.scale as f32).max(minimum_fit));
        let photo_width = image.width() as f32 * fit + 8.0;
        let photo_height = image.height() as f32 * fit + 8.0;
        w.set_start_width(photo_width);
        w.set_start_height(photo_height);
        w.set_start_x((width - photo_width) / 2.0);
        w.set_start_y((height - photo_height) / 2.0);
        w.set_target_x(shot.x);
        w.set_target_y(shot.y + 16.0);
        w.set_target_width(shot.card_width);
        w.set_target_height(shot.card_height);
        w.set_target_angle(shot.angle);
        // Initial geometry is chosen before show; after the first frame enable travel.
        w.set_reduced_motion(reduced);
        w.window()
            .set_size(slint::PhysicalSize::new(monitor.width, monitor.height));
        w.window()
            .set_position(slint::PhysicalPosition::new(monitor.x, monitor.y));
        if w.show().is_err() {
            self.cancel_capture_motion();
            return;
        }
        w.window().with_winit_window(|win| {
            let _ = win.set_cursor_hittest(false);
        });
        if let Some(h) = hwnd(w.window()) {
            let _ = native::configure_overlay(h);
            let _ = native::set_click_through(h, true);
            let _ = native::place_overlay(h, monitor, height as f64);
        }
        self.capture_started.set(Some(Instant::now()));
        if self.settings.borrow().capture_sound {
            native::play_camera_shutter();
        }
    }
    fn show_settings(&self) {
        self.cancel_capture_motion();
        let s = self.settings.borrow().clone();
        let w = &self.settings_window;
        w.set_english(s.english());
        w.set_system_dark(native::system_dark());
        w.set_dark(self.dark());
        w.set_section_index(0);
        w.set_clipboard_enabled(s.clipboard_enabled);
        w.set_capture_sound(s.capture_sound);
        w.set_watch_enabled(s.watch_enabled);
        w.set_watch_folder(
            s.watch_folder
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into(),
        );
        w.set_size_index(match s.size.as_str() {
            "small" => 0,
            "large" => 2,
            _ => 1,
        });
        w.set_max_items(s.max_items as i32);
        w.set_show_over_fullscreen(s.show_over_fullscreen);
        w.set_autostart(s.autostart);
        w.set_hotkey(s.hotkey.clone().into());
        w.set_language_index(if s.english() { 1 } else { 0 });
        w.set_theme_index(match s.theme.as_str() {
            "light" => 1,
            "dark" => 2,
            _ => 0,
        });
        w.set_status("".into());
        let _ = w.show();
        activate_window(w.window());
    }
    fn save_settings(&self) -> std::result::Result<(), String> {
        let w = &self.settings_window;
        let mut s = self.settings.borrow().clone();
        s.clipboard_enabled = w.get_clipboard_enabled();
        s.capture_sound = w.get_capture_sound();
        s.watch_enabled = w.get_watch_enabled();
        s.watch_folder = if w.get_watch_folder().is_empty() {
            None
        } else {
            Some(PathBuf::from(w.get_watch_folder().as_str()))
        };
        if s.watch_enabled && !s.watch_folder.as_ref().is_some_and(|p| p.is_dir()) {
            return Err(msg(
                w.get_english(),
                "请先选择有效的截图文件夹",
                "Choose an existing screenshot folder first",
            ));
        }
        s.size = ["small", "medium", "large"][w.get_size_index().clamp(0, 2) as usize].into();
        s.max_items = w.get_max_items().clamp(5, 100) as usize;
        s.show_over_fullscreen = w.get_show_over_fullscreen();
        s.language = if w.get_language_index() == 1 {
            "en"
        } else {
            "zh-CN"
        }
        .into();
        s.theme = ["system", "light", "dark"][w.get_theme_index().clamp(0, 2) as usize].into();
        s.autostart = w.get_autostart();
        s.hotkey = w.get_hotkey().trim().into();
        let next: HotKey = parse_hotkey(&s.hotkey)
            .map_err(|e| format!("快捷键格式无效 / Invalid shortcut: {e}"))?;
        let old = self.hotkey.get();
        let changed = old != Some(next);
        if changed {
            self.hotkeys
                .register(next)
                .map_err(|e| format!("快捷键已被占用 / Shortcut unavailable: {e}"))?;
        }
        if let Err(e) = native::set_autostart(s.autostart) {
            if changed {
                let _ = self.hotkeys.unregister(next);
            }
            return Err(e.to_string());
        }
        if let Err(e) = self.store.lock().unwrap().save_settings(s.clone()) {
            if changed {
                let _ = self.hotkeys.unregister(next);
            }
            let _ = native::set_autostart(self.settings.borrow().autostart);
            return Err(e);
        }
        if changed {
            if let Some(old) = old {
                let _ = self.hotkeys.unregister(old);
            }
            self.hotkey.set(Some(next));
        }
        *self.settings.borrow_mut() = s;
        self.refresh();
        self.build_tray()?;
        w.set_status(msg(self.english(), "设置已保存", "Settings saved").into());
        Ok(())
    }
    fn item_action(&self, action: &str, id: &str) {
        let selected = {
            let store = self.store.lock().unwrap();
            store.find(id).map(|i| {
                let path = store.path(&i);
                (i, path)
            })
        };
        let Some((item, path)) = selected else {
            return;
        };
        self.cancel_capture_motion();
        let result: std::result::Result<(), String> = match action {
            "copy" => if item.video {
                native::copy_file(self.hwnd.get(), &path).map_err(|e| e.to_string())
            } else {
                storage::load_image(&path).and_then(|image| {
                    let pixels = image.to_rgba8();
                    arboard::Clipboard::new()
                        .map_err(|e| e.to_string())?
                        .set_image(arboard::ImageData {
                            width: pixels.width() as usize,
                            height: pixels.height() as usize,
                            bytes: std::borrow::Cow::Borrowed(pixels.as_raw()),
                        })
                        .map_err(|e| e.to_string())
                })
            }
            .inspect(|_| {
                self.ignored_sequence
                    .store(native::clipboard_sequence(), Ordering::Relaxed);
                self.status(msg(self.english(), "已复制", "Copied"));
            }),
            "open" => native::open_file(&path).map_err(|e| e.to_string()),
            "preview" => {
                if item.video {
                    native::open_file(&path).map_err(|e| e.to_string())
                } else {
                    self.open_preview(id, &path, &item.name)
                }
            }
            "save" => {
                self.suppress.set(true);
                let _ = self.overlay.hide();
                let target = rfd::FileDialog::new().set_file_name(&item.name).save_file();
                let result = target.map_or(Ok(()), |target| {
                    if target == path {
                        Ok(())
                    } else {
                        std::fs::copy(&path, target)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    }
                });
                self.suppress.set(false);
                self.show_line(true);
                result
            }
            "remove" => {
                let result = self.store.lock().unwrap().remove(id, true);
                self.refresh();
                result
            }
            "drag" => match native::drag_file(self.hwnd.get(), &path) {
                Ok(moved) => {
                    if moved {
                        let _ = self.store.lock().unwrap().forget_moved(id);
                    }
                    self.refresh();
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.status(e);
        }
    }
    fn open_preview(&self, id: &str, path: &Path, name: &str) -> std::result::Result<(), String> {
        if self.preview.get_dirty() && !self.confirm_discard() {
            return Ok(());
        }
        let image = storage::load_image(path)?.to_rgba8();
        *self.edit.borrow_mut() = Some(editor::Editor::new(image));
        *self.preview_id.borrow_mut() = Some(id.to_owned());
        *self.preview_path.borrow_mut() = Some(path.to_owned());
        self.preview.set_image_title(name.into());
        self.preview.set_english(self.english());
        self.preview.set_dark(self.dark());
        self.preview.set_status("".into());
        self.preview.set_dirty(false);
        self.update_preview();
        self.preview.show().map_err(|e| e.to_string())?;
        activate_window(self.preview.window());
        Ok(())
    }
    fn update_preview(&self) {
        if let Some(edit) = self.edit.borrow().as_ref() {
            self.preview.set_image(image_from_rgba(&edit.image));
            self.preview.set_image_width(edit.image.width() as i32);
            self.preview.set_image_height(edit.image.height() as i32);
            self.preview.set_can_undo(edit.can_undo());
        }
    }
    fn confirm_discard(&self) -> bool {
        rfd::MessageDialog::new()
            .set_title(msg(self.english(), "尚未保存的标注", "Unsaved annotations"))
            .set_description(msg(
                self.english(),
                "关闭并放弃尚未保存的修改？",
                "Discard unsaved changes and close?",
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
    }
    fn save_edited(&self) {
        let result = (|| {
            let id = self
                .preview_id
                .borrow()
                .clone()
                .ok_or("No image is being edited")?;
            let edit = self.edit.borrow();
            let edit = edit.as_ref().ok_or("No image is being edited")?;
            let mut store = self.store.lock().map_err(|e| e.to_string())?;
            let item = store.replace_image(&id, &edit.image)?;
            Ok::<_, String>((store.path(&item), item))
        })();
        match result {
            Ok((path, item)) => {
                self.preview_id.replace(Some(item.id));
                self.preview_path.replace(Some(path));
                self.preview.set_image_title(item.name.into());
                self.preview.set_dirty(false);
                self.preview.set_status(
                    msg(
                        self.english(),
                        "已保存，绳上的图片已更新",
                        "Saved and updated on the line",
                    )
                    .into(),
                );
                self.refresh();
            }
            Err(e) => self.preview.set_status(e.into()),
        }
    }
    fn save_edited_copy(&self) {
        let target = rfd::FileDialog::new()
            .set_file_name(format!("Annotated-{}.png", storage::now()))
            .add_filter("PNG", &["png"])
            .save_file();
        let Some(target) = target else {
            return;
        };
        if self.preview_path.borrow().as_ref() == Some(&target) {
            self.preview.set_status(
                msg(
                    self.english(),
                    "请选择不同文件名以保留原图",
                    "Choose a different filename to preserve the original",
                )
                .into(),
            );
            return;
        }
        let result = if let Some(edit) = self.edit.borrow().as_ref() {
            edit.image
                .save_with_format(target, image::ImageFormat::Png)
                .map_err(|e| e.to_string())
        } else {
            Ok(())
        };
        match result {
            Ok(()) => {
                self.preview.set_status(
                    msg(
                        self.english(),
                        "副本已另存；点击保存可更新绳上的图片",
                        "Copy exported; use Save to update the image on the line",
                    )
                    .into(),
                );
            }
            Err(e) => self.preview.set_status(e.into()),
        }
    }
    fn build_tray(&self) -> std::result::Result<(), String> {
        let en = self.english();
        let menu = Menu::new();
        let labels = [
            ("toggle", "展开 / 收起", "Show / hide"),
            ("capture", "截图", "Capture"),
            ("import", "导入…", "Import…"),
            ("settings", "设置…", "Settings…"),
            ("about", "关于晾图…", "About Snapline…"),
            ("clear", "清空晾衣绳…", "Clear line…"),
            ("quit", "退出", "Quit"),
        ];
        let mut items = Vec::new();
        for (id, zh, en_label) in labels {
            let item = MenuItem::with_id(id, msg(en, zh, en_label), true, None);
            menu.append(&item).map_err(|e| e.to_string())?;
            if id == "about" {
                menu.append(&PredefinedMenuItem::separator())
                    .map_err(|e| e.to_string())?;
            }
            items.push(item);
        }
        let rgba = image::load_from_memory(include_bytes!("../assets/snapline.png"))
            .map_err(|e| e.to_string())?
            .into_rgba8();
        let icon =
            tray_icon::Icon::from_rgba(rgba.into_raw(), 64, 64).map_err(|e| e.to_string())?;
        self.tray.borrow_mut().take();
        let tray = TrayIconBuilder::new()
            .with_id("snapline-native")
            .with_guid(0x80cbf1af_97f0_4b85_98e9_151646c80f12)
            .with_icon(icon)
            .with_tooltip("Snapline · 晾图")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .build()
            .map_err(|e| e.to_string())?;
        *self.tray.borrow_mut() = Some(tray);
        *self.tray_items.borrow_mut() = items;
        Ok(())
    }
    fn clear(&self) {
        self.cancel_capture_motion();
        if rfd::MessageDialog::new()
            .set_title(msg(self.english(), "清空晾衣绳", "Clear the line"))
            .set_description(msg(
                self.english(),
                "缓存文件将移入回收站，来源文件不受影响。",
                "Move cached files to the Recycle Bin? Source files stay untouched.",
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
        {
            if let Err(e) = self.store.lock().unwrap().clear() {
                self.status(e);
            }
            self.refresh();
        }
    }
    fn show_about(&self) {
        self.about.set_english(self.english());
        let _ = self.about.show();
        activate_window(self.about.window());
    }
    fn toggle(&self) {
        if self.overlay.get_revealed() && !self.suppress.get() {
            self.hide_line();
        } else {
            self.show_line(true);
        }
    }
    fn tick(&self) {
        if self.refresh_pending.get() {
            self.refresh();
        }
        self.advance_drop_motion();
        if Instant::now() > self.status_until.get() && !self.overlay.get_status().is_empty() {
            self.overlay.set_status("".into());
        }
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed
                && self.hotkey.get().is_some_and(|h| h.id() == event.id)
            {
                self.toggle();
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id.as_ref() {
                "toggle" => self.toggle(),
                "capture" => self.capture(),
                "import" => self.import_files(),
                "settings" => self.show_settings(),
                "about" => self.show_about(),
                "clear" => self.clear(),
                "quit" => {
                    if !self.preview.get_dirty() || self.confirm_discard() {
                        let _ = slint::quit_event_loop();
                    }
                }
                _ => {}
            }
        }
        let Some(cursor) = native::cursor_position() else {
            return;
        };
        let Some(at_cursor) = native::monitor_at_cursor() else {
            return;
        };
        let top = cursor.1 >= at_cursor.y && cursor.1 <= at_cursor.y + 2;
        let fullscreen = !self.settings.borrow().show_over_fullscreen
            && native::fullscreen_on_monitor(&at_cursor);
        if top && !self.suppress.get() && !fullscreen && self.capture_pending.borrow().is_none() {
            if !self.overlay.get_revealed() || self.monitor.get() != Some(at_cursor) {
                self.show_line(false);
            }
        }
        let overlay_fullscreen = !self.settings.borrow().show_over_fullscreen
            && self
                .monitor
                .get()
                .is_some_and(|m| native::fullscreen_on_monitor(&m));
        if overlay_fullscreen && self.overlay.get_revealed() {
            self.hide_line();
        }
        if let Some(h) = hwnd(self.overlay.window()) {
            self.hwnd.set(h);
            // Refresh the layout even when display settings change without a top-edge gesture.
            if self.configured.get() {
                if let Some(monitor) = native::monitor_for_window(h) {
                    if self.monitor.get() != Some(monitor) {
                        self.cancel_capture_motion();
                        self.cancel_drop_motion();
                        self.monitor.set(Some(monitor));
                        self.configured.set(false);
                        self.refresh();
                    }
                }
            }
            let reconfigured = !self.configured.get();
            if reconfigured {
                if let Some(m) = self.monitor.get() {
                    let _ = native::place_overlay(h, m, 300.0);
                }
                let _ = native::configure_overlay(h);
                self.configured.set(true);
            }
            let m = self.monitor.get().unwrap_or(at_cursor);
            let x = (cursor.0 - m.x) as f32 / m.scale as f32;
            let y = (cursor.1 - m.y) as f32 / m.scale as f32;
            let width = m.width as f32 / m.scale as f32;
            let inside =
                cursor.0 >= m.x && cursor.0 < m.x + m.width as i32 && y >= 0.0 && y < 285.0;
            if inside {
                self.last_inside.set(Instant::now());
            }
            let hit = self.overlay.get_revealed()
                && ((x >= width - 448.0 && y < 52.0)
                    || (self.overlay.get_shots().row_count() == 0
                        && (x - width / 2.0).abs() < 215.0
                        && (50.0..150.0).contains(&y))
                    || self.overlay.get_shots().iter().any(|s| {
                        s.id != self.overlay.get_arriving_id()
                            && x >= s.x - (108.0 - s.card_width).max(0.0) / 2.0 - 14.0
                            && x <= s.x
                                + s.card_width
                                + (108.0 - s.card_width).max(0.0) / 2.0
                                + 14.0
                            && y >= s.y - 12.0
                            && y <= s.y + s.card_height + 68.0
                    }));
            if reconfigured || hit != self.hit_test.get() {
                self.overlay.window().with_winit_window(|w| {
                    let _ = w.set_cursor_hittest(hit);
                });
                let _ = native::configure_overlay(h);
                let _ = native::set_click_through(h, !hit);
                self.hit_test.set(hit);
            }
            if self.overlay.get_revealed()
                && !inside
                && Instant::now() > self.pinned_until.get()
                && self.last_inside.get().elapsed() > Duration::from_millis(650)
            {
                self.hide_line();
            }
        }
        self.advance_capture_motion();
    }
}

fn bind(app: &Rc<App>) {
    macro_rules! bind0 {
        ($window:ident,$callback:ident,$method:ident) => {{
            let weak = Rc::downgrade(app);
            app.$window.$callback(move || {
                if let Some(a) = weak.upgrade() {
                    a.$method();
                }
            });
        }};
    }
    bind0!(overlay, on_capture, capture);
    bind0!(overlay, on_import_files, import_files);
    bind0!(overlay, on_settings, show_settings);
    bind0!(overlay, on_hide_line, hide_line);
    bind0!(overlay, on_clear_line, clear);
    bind0!(settings_window, on_about, show_about);
    bind0!(preview, on_save, save_edited);
    bind0!(preview, on_save_copy, save_edited_copy);
    {
        let weak = Rc::downgrade(app);
        app.overlay.on_item_action(move |action, id| {
            if let Some(a) = weak.upgrade() {
                a.item_action(&action, &id);
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.overlay.on_drag_start(move |id| {
            if let Some(a) = weak.upgrade() {
                a.item_action("drag", &id);
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.overlay.on_mouse_event(move |id, event, x, y| {
            if let Some(a) = weak.upgrade() {
                match event.as_str() {
                    "down" => {
                        a.gesture.replace(Some(Gesture {
                            id: id.into(),
                            x,
                            y,
                        }));
                    }
                    "move" => {
                        let drag = {
                            let mut g = a.gesture.borrow_mut();
                            if g.as_ref()
                                .is_some_and(|gesture| (x - gesture.x).hypot(y - gesture.y) > 6.0)
                            {
                                g.take().map(|gesture| gesture.id)
                            } else {
                                None
                            }
                        };
                        if let Some(id) = drag {
                            a.item_action("drag", &id);
                        }
                    }
                    "up" | "cancel" => {
                        a.gesture.replace(None);
                    }
                    _ => {}
                }
            }
        });
    }
    for (next, callback) in [(true, 0), (false, 1)] {
        let weak = Rc::downgrade(app);
        let handler = move || {
            if let Some(a) = weak.upgrade() {
                a.cancel_capture_motion();
                a.cancel_drop_motion();
                a.page.set(if next {
                    a.page.get() + 1
                } else {
                    a.page.get().saturating_sub(1)
                });
                a.refresh();
            }
        };
        if callback == 0 {
            app.overlay.on_next_page(handler)
        } else {
            app.overlay.on_previous_page(handler)
        }
    }
    {
        let weak = Rc::downgrade(app);
        app.settings_window.on_done(move || {
            if let Some(a) = weak.upgrade() {
                let _ = a.settings_window.hide();
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.settings_window.on_choose_folder(move || {
            if let Some(a) = weak.upgrade() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    a.settings_window
                        .set_watch_folder(path.to_string_lossy().into_owned().into());
                }
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.settings_window.on_save(move || {
            if let Some(a) = weak.upgrade() {
                if let Err(e) = a.save_settings() {
                    a.settings_window.set_status(e.into());
                }
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.about.on_done(move || {
            if let Some(a) = weak.upgrade() {
                let _ = a.about.hide();
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.preview.on_pointer_event(move |event, x, y| {
            if let Some(a) = weak.upgrade() {
                let mut changed = event != "move";
                if event == "down" {
                    a.preview.set_status("".into());
                }
                if let Some(edit) = a.edit.borrow_mut().as_mut() {
                    edit.tool = ["pen", "rect", "arrow", "crop"]
                        [a.preview.get_tool_index().clamp(0, 3) as usize]
                        .into();
                    let c = a.preview.get_stroke_color().to_argb_u8();
                    edit.color = image::Rgba([c.red, c.green, c.blue, c.alpha]);
                    edit.stroke_width = a.preview.get_stroke_width();
                    edit.pointer(&event, x, y);
                    changed |= edit.tool == "pen";
                }
                a.preview.set_dirty(true);
                if changed {
                    a.update_preview();
                }
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.preview.on_undo(move || {
            if let Some(a) = weak.upgrade() {
                if let Some(edit) = a.edit.borrow_mut().as_mut() {
                    edit.undo();
                }
                a.preview.set_dirty(true);
                a.update_preview();
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.preview.on_open_original(move || {
            if let Some(a) = weak.upgrade() {
                if let Some(path) = a.preview_path.borrow().as_ref() {
                    let _ = native::open_file(path);
                }
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.preview.window().on_close_requested(move || {
            if let Some(a) = weak.upgrade() {
                if a.preview.get_dirty() && !a.confirm_discard() {
                    return slint::CloseRequestResponse::KeepWindowShown;
                }
                a.preview.set_dirty(false);
            }
            slint::CloseRequestResponse::HideWindow
        });
    }
    {
        let weak = Rc::downgrade(app);
        app.overlay.window().on_winit_window_event(move |_, event| {
            if let Some(a) = weak.upgrade() {
                if let winit::event::WindowEvent::DroppedFile(path) = event {
                    a.import_paths(vec![path.clone()]);
                }
            }
            slint::winit_030::EventResult::Propagate
        });
    }
}

fn main() {
    if let Err(error) = run() {
        let _ = std::fs::write(
            std::env::temp_dir().join("snapline-startup-error.txt"),
            &error,
        );
        if std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--rope-check"
                    | "--capture-check"
                    | "--edit-check"
                    | "--overflow-check"
                    | "--settings-check"
            )
        }) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        rfd::MessageDialog::new()
            .set_title("Snapline · 启动失败 / Startup failed")
            .set_description(error)
            .show();
    }
}
fn run() -> std::result::Result<(), String> {
    let _ole = native::OleGuard::initialize().map_err(|e| e.to_string())?;
    let created = Cell::new(0);
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        .with_winit_window_attributes_hook(move |attributes| {
            use winit::platform::windows::WindowAttributesExtWindows;
            let n = created.get();
            created.set(n + 1);
            if n < 3 {
                attributes
                    .with_transparent(true)
                    .with_active(false)
                    .with_skip_taskbar(true)
                    .with_decorations(false)
                    .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            } else {
                attributes
            }
        })
        .select()
        .map_err(|e| e.to_string())?;
    let args = std::env::args().collect::<Vec<_>>();
    let edit_check_directory = args
        .iter()
        .position(|a| a == "--edit-check")
        .map(|i| {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or("Missing edit check directory")
        })
        .transpose()?;
    let overflow_check_directory = args
        .iter()
        .position(|a| a == "--overflow-check")
        .map(|i| {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or("Missing overflow check directory")
        })
        .transpose()?;
    if let Some(i) = args.iter().position(|a| a == "--settings-check") {
        return settings_check::run(PathBuf::from(
            args.get(i + 1).ok_or("Missing settings check directory")?,
        ));
    }
    if let Some(i) = args.iter().position(|a| a == "--capture-check") {
        return capture_check::run(PathBuf::from(
            args.get(i + 1).ok_or("Missing capture check directory")?,
        ));
    }
    if let Some(i) = args.iter().position(|a| a == "--rope-check") {
        return rope_check(PathBuf::from(
            args.get(i + 1).ok_or("Missing rope check directory")?,
        ));
    }
    if let Some(i) = args.iter().position(|a| a == "--ui-smoke") {
        return ui_smoke(PathBuf::from(
            args.get(i + 1).ok_or("Missing smoke output directory")?,
        ));
    }
    let Some(instance) = native::SingleInstance::acquire().map_err(|e| e.to_string())? else {
        native::notify_existing().map_err(|e| e.to_string())?;
        return Ok(());
    };
    let mut loaded = Store::load(
        edit_check_directory
            .as_ref()
            .or(overflow_check_directory.as_ref())
            .map_or_else(storage::data_root, |p| p.join("profile")),
    )?;
    if edit_check_directory.is_some() || overflow_check_directory.is_some() {
        loaded.settings.clipboard_enabled = false;
        loaded.settings.watch_enabled = false;
        loaded.settings.capture_sound = false;
        loaded.settings.show_over_fullscreen = true;
    }
    let store = Arc::new(Mutex::new(loaded));
    let settings = store.lock().unwrap().settings.clone();
    let (sender, receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let ignored_sequence = Arc::new(AtomicU32::new(0));
    let app = Rc::new(App {
        overlay: OverlayWindow::new().map_err(|e| e.to_string())?,
        shots_model: Rc::new(VecModel::default()),
        capture_window: CaptureWindow::new().map_err(|e| e.to_string())?,
        drop_window: DropWindow::new().map_err(|e| e.to_string())?,
        drop_model: Rc::new(VecModel::default()),
        drop_expiry: RefCell::new(Vec::new()),
        pending_shots: RefCell::new(None),
        capture_pending: RefCell::new(None),
        capture_image: RefCell::new(None),
        capture_started: Cell::new(None),
        settings_window: SettingsWindow::new().map_err(|e| e.to_string())?,
        preview: PreviewWindow::new().map_err(|e| e.to_string())?,
        about: AboutWindow::new().map_err(|e| e.to_string())?,
        store: store.clone(),
        settings: RefCell::new(settings.clone()),
        sender: sender.clone(),
        ignored_sequence: ignored_sequence.clone(),
        hotkeys: GlobalHotKeyManager::new().map_err(|e| e.to_string())?,
        hotkey: Cell::new(None),
        monitor: Cell::new(None),
        hwnd: Cell::new(HWND::default()),
        hit_test: Cell::new(false),
        configured: Cell::new(false),
        suppress: Cell::new(false),
        pinned_until: Cell::new(Instant::now()),
        last_inside: Cell::new(Instant::now()),
        status_until: Cell::new(Instant::now()),
        page: Cell::new(0),
        refresh_pending: Cell::new(false),
        gesture: RefCell::new(None),
        edit: RefCell::new(None),
        preview_id: RefCell::new(None),
        preview_path: RefCell::new(None),
        tray: RefCell::new(None),
        tray_items: RefCell::new(Vec::new()),
    });
    app.overlay
        .set_shots(ModelRc::from(app.shots_model.clone()));
    app.drop_window
        .set_shots(ModelRc::from(app.drop_model.clone()));
    bind(&app);
    app.overlay
        .set_reduced_motion(!native::animations_enabled());
    app.build_tray()?;
    match parse_hotkey(&settings.hotkey) {
        Ok(h) => match app.hotkeys.register(h) {
            Ok(()) => app.hotkey.set(Some(h)),
            Err(e) => app.status(format!("快捷键不可用 / Shortcut unavailable: {e}")),
        },
        Err(e) => app.status(e.to_string()),
    }
    app.show_line(true);
    app.refresh();
    if let Some(directory) = edit_check_directory {
        return edit_check::run(app, directory);
    }
    if let Some(directory) = overflow_check_directory {
        return overflow_check::run(app, directory);
    }
    if args.iter().any(|a| a == "--settings") {
        app.show_settings();
    }
    let worker = monitor::start(store, sender, stop.clone(), ignored_sequence);
    let timer = slint::Timer::default();
    let cloned = app.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(60),
        move || {
            if instance.take_settings_request() {
                cloned.show_settings();
            }
            while let Ok(event) = receiver.try_recv() {
                match event {
                    monitor::Event::Changed { id, capture } => {
                        if capture {
                            cloned.new_capture(id);
                        } else {
                            cloned.cancel_capture_motion();
                            cloned.refresh_pending.set(true);
                            cloned.page.set(usize::MAX);
                            cloned.show_line(true);
                        }
                    }
                    monitor::Event::CaptureReady { id, image } => {
                        if cloned.capture_pending.borrow().as_deref() == Some(id.as_str()) {
                            match image {
                                Ok(image) => {
                                    cloned.capture_image.replace(Some(image));
                                }
                                Err(_) => cloned.cancel_capture_motion(),
                            }
                        }
                    }
                    monitor::Event::Error(e) => cloned.status(e),
                }
            }
            cloned.tick();
        },
    );
    let result = slint::run_event_loop_until_quit().map_err(|e| e.to_string());
    stop.store(true, Ordering::Relaxed);
    let _ = worker.join();
    result
}

// Check framebuffer pixels: a full-size HWND with a fixed-width drawing must fail.
fn rope_check(directory: PathBuf) -> std::result::Result<(), String> {
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let overlay = OverlayWindow::new().map_err(|e| e.to_string())?;
    let widths = [1280, 1920, 2560, 3840];
    overlay
        .window()
        .set_size(slint::PhysicalSize::new(widths[0], 300));
    overlay.show().map_err(|e| e.to_string())?;
    let results = Rc::new(RefCell::new(Vec::new()));
    let collected = results.clone();
    let captured = overlay.clone_strong();
    let timer = slint::Timer::default();
    let output = directory.clone();
    let mut step = 0;
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(650), move || {
        let window = captured.window();
        window.with_winit_window(|w| { let _ = w.set_cursor_hittest(false); });
        if let Some(h) = hwnd(window) { let _ = native::configure_overlay(h); }
        match window.take_snapshot() {
            Ok(buffer) => {
                let start = (5.0 * window.scale_factor()) as u32;
                let end = ((48.0 * window.scale_factor()) as u32).min(buffer.height());
                let pixels = buffer.as_bytes();
                let uncovered = (0..buffer.width()).filter(|&x| {
                    !(start..end).any(|y| pixels[((y * buffer.width() + x) * 4 + 3) as usize] > 8)
                }).count();
                let passed = buffer.width() == widths[step] && uncovered == 0;
                collected.borrow_mut().push(serde_json::json!({"requested_width": widths[step], "rendered_width": buffer.width(), "scale": window.scale_factor(), "uncovered_columns": uncovered, "passed": passed}));
                let _ = image::save_buffer(output.join(format!("rope-{}.png", widths[step])), pixels, buffer.width(), buffer.height(), image::ColorType::Rgba8);
            }
            Err(error) => collected.borrow_mut().push(serde_json::json!({"requested_width": widths[step], "passed": false, "error": error.to_string()})),
        }
        step += 1;
        if step == widths.len() { let _ = slint::quit_event_loop(); }
        else { window.set_size(slint::PhysicalSize::new(widths[step], 300)); }
    });
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    let results = results.borrow();
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&*results).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if results.len() != widths.len() || results.iter().any(|result| result["passed"] != true) {
        return Err("The rope did not span the resized window; see report.json".into());
    }
    Ok(())
}

fn ui_smoke(directory: PathBuf) -> std::result::Result<(), String> {
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let overlay = OverlayWindow::new().map_err(|e| e.to_string())?;
    let _capture = CaptureWindow::new().map_err(|e| e.to_string())?;
    let _drop = DropWindow::new().map_err(|e| e.to_string())?;
    let settings = SettingsWindow::new().map_err(|e| e.to_string())?;
    let preview = PreviewWindow::new().map_err(|e| e.to_string())?;
    let about = AboutWindow::new().map_err(|e| e.to_string())?;
    let mut sample = image::RgbaImage::new(600, 400);
    for (x, y, pixel) in sample.enumerate_pixels_mut() {
        *pixel = image::Rgba([(35 + x / 4) as u8, (95 + y / 5) as u8, 130, 255]);
    }
    let img = image_from_rgba(&sample);
    overlay.set_shots(ModelRc::new(VecModel::from(
        (0..5)
            .map(|i| Shot {
                id: format!("sample{i}").into(),
                title: format!("截图 Screenshot {}.png", i + 1).into(),
                thumbnail: img.clone(),
                x: 210.0 + i as f32 * 170.0 + if i == 2 { 45.0 } else { 0.0 },
                y: {
                    let t = (210.0 + i as f32 * 170.0 + 75.0 + 20.0) / 1320.0;
                    1.5 + 120.0 * t * (1.0 - t)
                },
                card_width: if i == 2 { 60.0 } else { 150.0 },
                card_height: 104.0,
                angle: if i % 2 == 0 { -1.4 } else { 1.5 },
                video: i == 4,
            })
            .collect::<Vec<_>>(),
    )));
    overlay.set_page_text("1 / 2".into());
    settings.set_watch_folder("C:\\Users\\用户\\Pictures\\Screenshots".into());
    settings.set_watch_enabled(true);
    preview.set_image(img);
    preview.set_image_title("Screenshot.png".into());
    preview.set_image_width(600);
    preview.set_image_height(400);
    let components = Rc::new((overlay, settings, preview, about));
    let timer = slint::Timer::default();
    let step = Rc::new(Cell::new(0));
    let copied = components.clone();
    let index = step.clone();
    components.0.show().map_err(|e| e.to_string())?;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(1200),
        move || {
            let n = index.get();
            if n >= 8 {
                let _ = slint::quit_event_loop();
                return;
            }
            let windows = [
                copied.0.window(),
                copied.1.window(),
                copied.2.window(),
                copied.3.window(),
            ];
            match windows[n % 4].take_snapshot() {
                Ok(buffer) => {
                    let path = directory.join(if n < 4 {
                        ["overlay.png", "settings.png", "preview.png", "about.png"][n]
                    } else {
                        [
                            "overlay-en-dark.png",
                            "settings-en-dark.png",
                            "preview-en-dark.png",
                            "about-en-dark.png",
                        ][n - 4]
                    });
                    let _ = image::save_buffer(
                        path,
                        buffer.as_bytes(),
                        buffer.width(),
                        buffer.height(),
                        image::ColorType::Rgba8,
                    );
                }
                Err(e) => {
                    let _ = std::fs::write(directory.join("snapshot-error.txt"), e.to_string());
                }
            }
            let _ = windows[n % 4].hide();
            index.set(n + 1);
            if n == 3 {
                copied.0.set_english(true);
                copied.0.set_dark(true);
                copied.1.set_english(true);
                copied.1.set_dark(true);
                copied.1.set_language_index(1);
                copied.1.set_theme_index(2);
                copied.2.set_english(true);
                copied.2.set_dark(true);
                copied.3.set_english(true);
            }
            if n + 1 < 8 {
                let _ = windows[(n + 1) % 4].show();
            }
        },
    );
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_accept_windows_key_and_reject_ambiguous_input() {
        assert_eq!(
            parse_hotkey("Win + Shift + T").unwrap(),
            "Super+Shift+T".parse::<HotKey>().unwrap()
        );
        assert!(parse_hotkey("Ctrl+Alt+T").is_ok());
        assert!(parse_hotkey("Ctrl+A+B").is_err());
    }
}
