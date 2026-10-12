//! Polling keeps clipboard and folder ingestion independent of the UI event loop.

use crate::{native, storage::Store};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::Sender,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

#[derive(Debug)]
pub enum Event {
    Changed {
        id: String,
        capture: bool,
    },
    CaptureReady {
        id: String,
        image: Result<image::RgbaImage, String>,
    },
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: Option<SystemTime>,
}

#[derive(Debug)]
struct FileState {
    stamp: Stamp,
    stable_polls: u8,
    attempts: u8,
    imported: bool,
}

#[derive(Default)]
struct FolderWatch {
    folder: Option<PathBuf>,
    files: HashMap<PathBuf, FileState>,
    initialized: bool,
    reported_error: bool,
}

impl FolderWatch {
    fn select(&mut self, folder: Option<PathBuf>) {
        if self.folder != folder {
            self.folder = folder;
            self.files.clear();
            self.initialized = false;
            self.reported_error = false;
        }
    }

    fn observe(&mut self, path: &Path, stamp: Stamp) -> bool {
        let state = self.files.entry(path.to_owned()).or_insert(FileState {
            stamp,
            stable_polls: 0,
            attempts: 0,
            imported: !self.initialized,
        });
        if state.stamp != stamp {
            state.stamp = stamp;
            state.stable_polls = 1;
            state.attempts = 0;
            state.imported = false;
        } else {
            state.stable_polls = state.stable_polls.saturating_add(1);
        }
        !state.imported && state.stable_polls >= 2 && stamp.size != 0 && state.attempts < 4
    }
}

fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "bmp"
                    | "gif"
                    | "tif"
                    | "tiff"
                    | "mp4"
                    | "mov"
                    | "m4v"
                    | "webm"
            )
        })
}

fn folder_snapshot(folder: &Path) -> std::io::Result<Vec<(PathBuf, Stamp)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(folder)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !supported(&path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            files.push((
                path,
                Stamp {
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                },
            ));
        }
    }
    files.sort_by_key(|(_, stamp)| stamp.modified);
    Ok(files)
}

pub fn start(
    store: Arc<Mutex<Store>>,
    sender: Sender<Event>,
    stop: Arc<AtomicBool>,
    ignored_sequence: Arc<AtomicU32>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut clipboard = None;
        let mut sequence = native::clipboard_sequence();
        let mut attempts = 4u8;
        let mut clipboard_enabled = false;
        let mut watcher = FolderWatch::default();

        while !stop.load(Ordering::Acquire) {
            let settings = match store.lock() {
                Ok(store) => store.settings.clone(),
                Err(_) => {
                    let _ = sender.send(Event::Error("Screenshot store lock failed".into()));
                    break;
                }
            };
            let current = native::clipboard_sequence();
            if settings.clipboard_enabled != clipboard_enabled {
                clipboard_enabled = settings.clipboard_enabled;
                sequence = current;
                attempts = 4;
            }
            if current != sequence {
                sequence = current;
                attempts = 0;
            }
            if clipboard_enabled && attempts < 4 {
                if sequence == ignored_sequence.load(Ordering::Acquire) {
                    attempts = 4;
                } else {
                    attempts += 1;
                    if native::clipboard_has_image_only() {
                        if clipboard.is_none() {
                            match arboard::Clipboard::new() {
                                Ok(value) => clipboard = Some(value),
                                Err(error) if attempts == 4 => {
                                    let _ =
                                        sender.send(Event::Error(format!("Clipboard: {error}")));
                                }
                                Err(_) => {}
                            }
                        }
                        if let Some(clipboard) = &mut clipboard {
                            match clipboard.get_image() {
                                Ok(image) => {
                                    // The clipboard can change between checking formats and reading.
                                    if native::clipboard_sequence() != sequence
                                        || sequence == ignored_sequence.load(Ordering::Acquire)
                                    {
                                        continue;
                                    }
                                    attempts = 4;
                                    let dimensions = u32::try_from(image.width)
                                        .ok()
                                        .zip(u32::try_from(image.height).ok());
                                    let result = match dimensions {
                                        Some((width, height)) => store
                                            .lock()
                                            .map_err(|_| "Screenshot store lock failed".to_string())
                                            .and_then(|mut store| {
                                                store.import_rgba(width, height, &image.bytes)
                                            }),
                                        None => {
                                            Err("Clipboard image dimensions exceed Windows limits"
                                                .into())
                                        }
                                    };
                                    match result {
                                        Ok(Some(item)) => {
                                            let _ = sender.send(Event::Changed {
                                                id: item.id,
                                                capture: true,
                                            });
                                        }
                                        Ok(None) => {}
                                        Err(error) => {
                                            let _ = sender.send(Event::Error(error));
                                        }
                                    }
                                }
                                Err(error) if attempts == 4 => {
                                    let _ =
                                        sender.send(Event::Error(format!("Clipboard: {error}")));
                                }
                                Err(_) => {}
                            }
                        }
                    }
                }
            }

            watcher.select(if settings.watch_enabled {
                settings.watch_folder
            } else {
                None
            });
            if let Some(folder) = &watcher.folder {
                match folder_snapshot(folder) {
                    Ok(snapshot) => {
                        watcher.reported_error = false;
                        let present: HashSet<_> =
                            snapshot.iter().map(|(path, _)| path.clone()).collect();
                        watcher.files.retain(|path, _| present.contains(path));
                        for (path, stamp) in snapshot {
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            if !watcher.observe(&path, stamp) {
                                continue;
                            }
                            let state = watcher.files.get_mut(&path).unwrap();
                            state.attempts += 1;
                            let result = store
                                .lock()
                                .map_err(|_| "Screenshot store lock failed".to_string())
                                .and_then(|mut store| store.import(&path));
                            match result {
                                Ok(item) => {
                                    state.imported = true;
                                    if let Some(item) = item {
                                        let _ = sender.send(Event::Changed {
                                            id: item.id,
                                            capture: !item.video,
                                        });
                                    }
                                }
                                Err(error) if state.attempts == 4 => {
                                    let _ = sender
                                        .send(Event::Error(format!("{}: {error}", path.display())));
                                }
                                Err(_) => {}
                            }
                        }
                        watcher.initialized = true;
                    }
                    Err(error) if !watcher.reported_error => {
                        watcher.reported_error = true;
                        let _ = sender.send(Event::Error(format!("Screenshot folder: {error}")));
                    }
                    Err(_) => {}
                }
            }
            thread::sleep(Duration::from_millis(250));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_imports_new_file_and_preserves_original() {
        let root = std::env::temp_dir().join(format!(
            "snapline-monitor-test-{}-{}",
            std::process::id(),
            crate::storage::now()
        ));
        let folder = root.join("screenshots");
        fs::create_dir_all(&folder).unwrap();
        let image = image::RgbaImage::from_pixel(8, 6, image::Rgba([20, 40, 60, 255]));
        image.save(folder.join("old.png")).unwrap();
        let mut initial = Store::load(root.join("data")).unwrap();
        initial.settings.clipboard_enabled = false;
        initial.settings.watch_enabled = true;
        initial.settings.watch_folder = Some(folder.clone());
        let store = Arc::new(Mutex::new(initial));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = start(
            store.clone(),
            sender,
            stop.clone(),
            Arc::new(AtomicU32::new(0)),
        );
        let baseline_event = receiver.recv_timeout(Duration::from_millis(650));
        let source = folder.join("新截图.png");
        image.save(&source).unwrap();
        let event = receiver.recv_timeout(Duration::from_secs(3));
        stop.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(matches!(
            baseline_event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(
            matches!(event, Ok(Event::Changed { capture: true, .. })),
            "{event:?}"
        );
        assert!(source.exists());
        let store = store.lock().unwrap();
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.items[0].name, "新截图.png");
        assert!(store.path(&store.items[0]).is_file());
        drop(store);
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_watch_ignores_baseline_and_waits_for_size_and_mtime() {
        let old = Path::new("existing.png");
        let new = Path::new("new.png");
        let mut watcher = FolderWatch::default();
        watcher.select(Some(PathBuf::from("screenshots")));
        let mut stamp = Stamp {
            size: 100,
            modified: Some(SystemTime::UNIX_EPOCH),
        };
        assert!(!watcher.observe(old, stamp));
        watcher.initialized = true;
        stamp.size = 200;
        assert!(!watcher.observe(old, stamp));
        assert!(watcher.observe(old, stamp)); // An overwrite after the baseline is a new capture.
        assert!(!watcher.observe(new, stamp));
        stamp.size = 300;
        assert!(!watcher.observe(new, stamp));
        stamp.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        assert!(!watcher.observe(new, stamp));
        assert!(watcher.observe(new, stamp));
        watcher.files.get_mut(new).unwrap().attempts = 4;
        assert!(!watcher.observe(new, stamp));
        stamp.size = 400;
        assert!(!watcher.observe(new, stamp));
        assert!(watcher.observe(new, stamp));
        watcher.select(None);
        watcher.select(Some(PathBuf::from("screenshots")));
        assert!(!watcher.observe(new, stamp));
    }
}
