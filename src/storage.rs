use image::{GenericImageView, ImageReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub type Result<T> = std::result::Result<T, String>;
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    pub clipboard_enabled: bool,
    pub capture_sound: bool,
    pub watch_enabled: bool,
    pub watch_folder: Option<PathBuf>,
    pub size: String,
    pub max_items: usize,
    pub show_over_fullscreen: bool,
    pub theme: String,
    pub language: String,
    pub autostart: bool,
    pub hotkey: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            clipboard_enabled: true,
            capture_sound: true,
            watch_enabled: false,
            watch_folder: None,
            size: "medium".into(),
            max_items: 30,
            show_over_fullscreen: false,
            theme: "system".into(),
            language: "zh-CN".into(),
            autostart: false,
            hotkey: "Ctrl+Alt+T".into(),
        }
    }
}
impl Settings {
    pub fn validate(&mut self) {
        self.max_items = self.max_items.clamp(5, 100);
        if !["small", "medium", "large"].contains(&self.size.as_str()) {
            self.size = "medium".into();
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            self.theme = "system".into();
        }
        if !["zh-CN", "en"].contains(&self.language.as_str()) {
            self.language = "zh-CN".into();
        }
        if self.hotkey.is_empty() {
            self.hotkey = "Ctrl+Alt+T".into();
        }
    }
    pub fn english(&self) -> bool {
        self.language == "en"
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub width: u32,
    pub height: u32,
    pub video: bool,
    pub created_at: u64,
}
pub struct Store {
    pub root: PathBuf,
    pub settings: Settings,
    pub items: Vec<Item>,
    evicted: Vec<String>,
}
impl Store {
    pub fn load(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("inbox")).map_err(err)?;
        fs::create_dir_all(root.join("thumbs")).map_err(err)?;
        let mut settings: Settings = fs::read(root.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        settings.validate();
        let items: Vec<Item> = fs::read(root.join("items.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut s = Self {
            root,
            settings,
            items,
            evicted: Vec::new(),
        };
        s.items.retain(|i| {
            valid_item(i)
                && s.root
                    .join("inbox")
                    .join(format!("{}.{}", i.id, i.extension))
                    .is_file()
        });
        s.trim()?;
        s.evicted.clear();
        Ok(s)
    }
    pub fn path(&self, item: &Item) -> PathBuf {
        self.root
            .join("inbox")
            .join(format!("{}.{}", item.id, item.extension))
    }
    pub fn thumb(&self, item: &Item) -> PathBuf {
        self.root.join("thumbs").join(format!("{}.png", item.id))
    }
    pub fn find(&self, id: &str) -> Option<Item> {
        self.items.iter().find(|i| i.id == id).cloned()
    }
    pub fn take_evicted(&mut self) -> Vec<String> {
        std::mem::take(&mut self.evicted)
    }
    pub fn persist(&self) -> Result<()> {
        atomic_json(&self.root.join("items.json"), &self.items)
    }
    pub fn save_settings(&mut self, mut settings: Settings) -> Result<()> {
        settings.validate();
        atomic_json(&self.root.join("settings.json"), &settings)?;
        self.settings = settings;
        self.trim()
    }
    pub fn import(&mut self, source: &Path) -> Result<Option<Item>> {
        let extension = source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let video = ["mp4", "mov", "m4v", "webm"].contains(&extension.as_str());
        if !video
            && !["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"]
                .contains(&extension.as_str())
        {
            return Err("不支持此文件格式 / Unsupported file format".into());
        }
        let meta = fs::metadata(source).map_err(err)?;
        if !meta.is_file() || meta.len() == 0 {
            return Err("文件为空或不是普通文件 / Empty or invalid file".into());
        }
        if !video && meta.len() > 64 * 1024 * 1024 {
            return Err("图片超过 64 MB / Image exceeds 64 MB".into());
        }
        // Copy and hash the same stream so a changing watched file cannot get the wrong content id.
        let stage = stage_path(&self.root.join("inbox"), "import");
        let mut input = fs::File::open(source).map_err(err)?;
        let mut output = fs::File::create(&stage).map_err(err)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let copied = (|| -> Result<()> {
            loop {
                let count = input.read(&mut buffer).map_err(err)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
                output.write_all(&buffer[..count]).map_err(err)?;
            }
            output.sync_all().map_err(err)
        })();
        drop(output);
        if let Err(e) = copied {
            let _ = fs::remove_file(&stage);
            return Err(e);
        }
        let id = format!("{:x}", hash.finalize());
        if self.items.iter().any(|i| i.id == id) {
            let _ = fs::remove_file(&stage);
            return Ok(None);
        }
        // Validate the exact bytes we copied; source contents may change during copying.
        let image = if video {
            None
        } else {
            Some(load_image(&stage).inspect_err(|_| {
                let _ = fs::remove_file(&stage);
            })?)
        };
        let (width, height) = image.as_ref().map_or((0, 0), |i| i.dimensions());
        let item = Item {
            id,
            name: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            extension,
            width,
            height,
            video,
            created_at: now(),
        };
        let destination = self.path(&item);
        fs::rename(&stage, &destination).map_err(err)?;
        if let Some(image) = image {
            if let Err(e) = image
                .thumbnail(360, 260)
                .save_with_format(self.thumb(&item), image::ImageFormat::Png)
            {
                let _ = fs::remove_file(&destination);
                return Err(err(e));
            }
        }
        self.items.insert(0, item.clone());
        self.trim()?;
        Ok(Some(item))
    }
    pub fn import_rgba(&mut self, width: u32, height: u32, pixels: &[u8]) -> Result<Option<Item>> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 80_000_000 {
            return Err("剪贴板图片尺寸过大 / Clipboard image too large".into());
        }
        let image = image::RgbaImage::from_raw(width, height, pixels.to_vec())
            .ok_or("Invalid clipboard image")?;
        let path = self.root.join(format!(
            "Screenshot-{}-{}.png",
            now(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        image.save(&path).map_err(err)?;
        let result = self.import(&path);
        let _ = fs::remove_file(path);
        result
    }
    pub fn replace_image(&mut self, id: &str, image: &image::RgbaImage) -> Result<Item> {
        let previous = self
            .find(id)
            .ok_or("Image is no longer on the clothesline")?;
        if !valid_item(&previous) || previous.video {
            return Err("Invalid image cache item".into());
        }
        let (width, height) = image.dimensions();
        if width == 0
            || height == 0
            || width > 32_768
            || height > 32_768
            || u64::from(width) * u64::from(height) > 80_000_000
        {
            return Err("Image exceeds 80 million pixels".into());
        }
        let staged_image = stage_path(&self.root.join("inbox"), "edit");
        let staged_thumb = stage_path(&self.root.join("thumbs"), "edit");
        let mut installed = Vec::new();
        let result = (|| -> Result<Item> {
            image
                .save_with_format(&staged_image, image::ImageFormat::Png)
                .map_err(err)?;
            let mut input = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&staged_image)
                .map_err(err)?;
            input.sync_all().map_err(err)?;
            if input.metadata().map_err(err)?.len() > 64 * 1024 * 1024 {
                return Err("Image exceeds 64 MB".into());
            }
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = input.read(&mut buffer).map_err(err)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            drop(input);
            let id = format!("{:x}", hash.finalize());
            if id == previous.id && previous.extension == "png" {
                return Ok(previous.clone());
            }
            let item = Item {
                id,
                name: Path::new(&previous.name)
                    .with_extension("png")
                    .to_string_lossy()
                    .into_owned(),
                extension: "png".into(),
                width,
                height,
                video: false,
                created_at: previous.created_at,
            };
            let scale = (360.0 / f64::from(width))
                .min(260.0 / f64::from(height))
                .min(1.0);
            image::imageops::thumbnail(
                image,
                (f64::from(width) * scale).round().max(1.0) as u32,
                (f64::from(height) * scale).round().max(1.0) as u32,
            )
            .save_with_format(&staged_thumb, image::ImageFormat::Png)
            .map_err(err)?;
            fs::OpenOptions::new()
                .write(true)
                .open(&staged_thumb)
                .map_err(err)?
                .sync_all()
                .map_err(err)?;
            // Existing content-addressed files may belong to another matching shelf item.
            // Keep them intact until the replacement index is committed.
            for (stage, destination) in [
                (&staged_image, self.path(&item)),
                (&staged_thumb, self.thumb(&item)),
            ] {
                if !destination.exists() {
                    fs::rename(stage, &destination).map_err(err)?;
                    installed.push(destination);
                }
            }
            let next = self
                .items
                .iter()
                .filter_map(|existing| {
                    if existing.id == previous.id {
                        Some(item.clone())
                    } else if existing.id == item.id {
                        None
                    } else {
                        Some(existing.clone())
                    }
                })
                .collect::<Vec<_>>();
            atomic_json(&self.root.join("items.json"), &next)?;
            let old_items = std::mem::replace(&mut self.items, next);
            // Only owned cache files unreferenced by the committed index are removed.
            for old in old_items {
                if valid_item(&old) {
                    if !self
                        .items
                        .iter()
                        .any(|current| self.path(current) == self.path(&old))
                    {
                        let _ = fs::remove_file(self.path(&old));
                    }
                    if !self.items.iter().any(|current| current.id == old.id) {
                        let _ = fs::remove_file(self.thumb(&old));
                    }
                }
            }
            Ok(item)
        })();
        let _ = fs::remove_file(staged_image);
        let _ = fs::remove_file(staged_thumb);
        if result.is_err() {
            for path in installed {
                let _ = fs::remove_file(path);
            }
        }
        result
    }
    pub fn remove(&mut self, id: &str, recycle: bool) -> Result<()> {
        let Some(item) = self.find(id) else {
            return Ok(());
        };
        if !valid_item(&item) {
            return Err("Invalid cache path".into());
        }
        let path = self.path(&item);
        if path.exists() {
            if recycle {
                trash::delete(&path).map_err(err)?;
            } else {
                fs::remove_file(&path).map_err(err)?;
            }
        }
        let _ = fs::remove_file(self.thumb(&item));
        self.items.retain(|i| i.id != id);
        self.persist()
    }
    pub fn forget_moved(&mut self, id: &str) -> Result<()> {
        if self.find(id).is_some_and(|item| !self.path(&item).exists()) {
            self.remove(id, false)?;
        }
        Ok(())
    }
    pub fn clear(&mut self) -> Result<()> {
        for id in self.items.iter().map(|i| i.id.clone()).collect::<Vec<_>>() {
            self.remove(&id, true)?;
        }
        Ok(())
    }
    fn trim(&mut self) -> Result<()> {
        while self.items.len() > self.settings.max_items {
            let item = self.items.last().unwrap().clone();
            self.remove(&item.id, false)?;
            self.evicted.push(item.id);
        }
        self.persist()
    }
}
pub fn data_root() -> PathBuf {
    std::env::var_os("SNAPLINE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
                .join("app.snapline.desktop")
        })
}
fn valid_item(item: &Item) -> bool {
    item.id.len() == 64
        && item.id.bytes().all(|b| b.is_ascii_hexdigit())
        && [
            "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "mp4", "mov", "m4v", "webm",
        ]
        .contains(&item.extension.as_str())
}
fn stage_path(directory: &Path, prefix: &str) -> PathBuf {
    directory.join(format!(
        ".{prefix}-{}-{}-{}.part",
        std::process::id(),
        now(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ))
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(err)?;
    let stage = stage_path(path.parent().unwrap(), "json");
    let result = (|| -> Result<()> {
        let mut file = fs::File::create(&stage).map_err(err)?;
        file.write_all(&bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        drop(file);
        fs::rename(&stage, path).map_err(err)
    })();
    if result.is_err() {
        let _ = fs::remove_file(stage);
    }
    result
}
pub fn load_image(path: &Path) -> Result<image::DynamicImage> {
    if fs::metadata(path).map_err(err)?.len() > 64 * 1024 * 1024 {
        return Err("Image exceeds 64 MB".into());
    }
    let mut reader = ImageReader::open(path)
        .map_err(err)?
        .with_guessed_format()
        .map_err(err)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    limits.max_alloc = Some(384 * 1024 * 1024);
    reader.limits(limits);
    let (w, h) = ImageReader::open(path)
        .map_err(err)?
        .with_guessed_format()
        .map_err(err)?
        .into_dimensions()
        .map_err(err)?;
    if u64::from(w) * u64::from(h) > 80_000_000 {
        return Err("Image exceeds 80 million pixels".into());
    }
    reader.decode().map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit_test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "snapline-edit-{label}-{}-{}-{}",
            std::process::id(),
            now(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn remove_test_root(root: PathBuf) {
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn capacity_evictions_are_oldest_first_and_consumed_once() {
        let root = edit_test_root("capacity");
        let mut store = Store::load(root.join("data")).unwrap();
        let mut settings = store.settings.clone();
        settings.max_items = 5;
        store.save_settings(settings.clone()).unwrap();
        let mut imported = Vec::new();
        for value in 1..=6 {
            imported.push(
                store
                    .import_rgba(1, 1, &[value, 0, 0, 255])
                    .unwrap()
                    .unwrap(),
            );
        }
        assert_eq!(
            store.items.iter().map(|i| &i.id).collect::<Vec<_>>(),
            imported[1..]
                .iter()
                .rev()
                .map(|i| &i.id)
                .collect::<Vec<_>>()
        );
        assert!(!store.path(&imported[0]).exists());
        assert!(!store.thumb(&imported[0]).exists());
        assert_eq!(store.take_evicted(), vec![imported[0].id.clone()]);
        assert!(store.take_evicted().is_empty());
        assert!(store.import_rgba(1, 1, &[6, 0, 0, 255]).unwrap().is_none());
        assert!(store.take_evicted().is_empty());
        store.remove(&imported[1].id, false).unwrap();
        assert!(store.take_evicted().is_empty());

        settings.max_items = 8;
        store.save_settings(settings.clone()).unwrap();
        for value in 7..=10 {
            imported.push(
                store
                    .import_rgba(1, 1, &[value, 0, 0, 255])
                    .unwrap()
                    .unwrap(),
            );
        }
        settings.max_items = 5;
        store.save_settings(settings.clone()).unwrap();
        assert_eq!(
            store.take_evicted(),
            imported[2..5]
                .iter()
                .map(|i| i.id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(store.items.len(), 5);

        settings.max_items = 8;
        store.save_settings(settings.clone()).unwrap();
        for value in 11..=13 {
            store.import_rgba(1, 1, &[value, 0, 0, 255]).unwrap();
        }
        // Trimming an existing session at startup should not animate old evictions.
        settings.max_items = 5;
        atomic_json(&store.root.join("settings.json"), &settings).unwrap();
        let mut reloaded = Store::load(root.join("data")).unwrap();
        assert_eq!(reloaded.items.len(), 5);
        assert!(reloaded.take_evicted().is_empty());
        remove_test_root(root);
    }
    #[test]
    fn replace_image_preserves_slot_source_and_reloads_edited_pixels() {
        let root = edit_test_root("save");
        let source = root.join("original.jpg");
        image::RgbImage::from_pixel(13, 9, image::Rgb([10, 20, 30]))
            .save(&source)
            .unwrap();
        let source_bytes = fs::read(&source).unwrap();
        let mut store = Store::load(root.join("data")).unwrap();
        let older = store.import_rgba(1, 1, &[255, 0, 0, 255]).unwrap().unwrap();
        let original = store.import(&source).unwrap().unwrap();
        let newer = store.import_rgba(1, 1, &[0, 255, 0, 255]).unwrap().unwrap();
        let old_path = store.path(&original);
        let old_thumb = store.thumb(&original);
        let edited = image::RgbaImage::from_pixel(23, 17, image::Rgba([44, 55, 66, 255]));
        let saved = store.replace_image(&original.id, &edited).unwrap();
        assert_ne!(saved.id, original.id);
        assert_eq!(saved.name, "original.png");
        assert_eq!(saved.extension, "png");
        assert_eq!(saved.created_at, original.created_at);
        assert_eq!((saved.width, saved.height), edited.dimensions());
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        assert!(!old_path.exists());
        assert!(!old_thumb.exists());
        assert_eq!(load_image(&store.path(&saved)).unwrap().to_rgba8(), edited);
        assert_eq!(load_image(&store.thumb(&saved)).unwrap().to_rgba8(), edited);
        let expected = vec![newer.id, saved.id.clone(), older.id];
        assert_eq!(
            store.items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            store.replace_image(&saved.id, &edited).unwrap().id,
            saved.id
        );
        assert!(
            store
                .import_rgba(23, 17, edited.as_raw())
                .unwrap()
                .is_none()
        );
        let reloaded = Store::load(root.join("data")).unwrap();
        assert_eq!(
            reloaded
                .items
                .iter()
                .map(|i| i.id.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            load_image(&reloaded.path(&saved)).unwrap().to_rgba8(),
            edited
        );
        remove_test_root(root);
    }
    #[test]
    fn replace_image_deduplicates_at_the_edited_slot() {
        let root = edit_test_root("dedup");
        let mut store = Store::load(root.join("data")).unwrap();
        let duplicate = store.import_rgba(1, 1, &[8, 9, 10, 255]).unwrap().unwrap();
        let middle = store.import_rgba(1, 1, &[3, 4, 5, 255]).unwrap().unwrap();
        let original = store.import_rgba(1, 1, &[0, 1, 2, 255]).unwrap().unwrap();
        let edited = image::RgbaImage::from_pixel(1, 1, image::Rgba([8, 9, 10, 255]));
        let saved = store.replace_image(&original.id, &edited).unwrap();
        assert_eq!(saved.id, duplicate.id);
        assert_eq!(saved.name, original.name);
        assert_eq!(
            store.items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            vec![saved.id.clone(), middle.id]
        );
        assert_eq!(load_image(&store.path(&saved)).unwrap().to_rgba8(), edited);
        assert!(store.thumb(&saved).is_file());
        assert_eq!(Store::load(root.join("data")).unwrap().items.len(), 2);
        remove_test_root(root);
    }
    #[test]
    fn replace_image_failed_index_commit_preserves_original() {
        let root = edit_test_root("rollback");
        let mut store = Store::load(root.join("data")).unwrap();
        let original = store
            .import_rgba(1, 1, &[10, 20, 30, 255])
            .unwrap()
            .unwrap();
        let original_bytes = fs::read(store.path(&original)).unwrap();
        let thumb_bytes = fs::read(store.thumb(&original)).unwrap();
        let index = store.root.join("items.json");
        let index_backup = store.root.join("items.backup");
        let index_bytes = fs::read(&index).unwrap();
        fs::rename(&index, &index_backup).unwrap();
        fs::create_dir(&index).unwrap();
        let edited = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 50, 255]));
        assert!(store.replace_image(&original.id, &edited).is_err());
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.items[0].id, original.id);
        assert_eq!(fs::read(store.path(&original)).unwrap(), original_bytes);
        assert_eq!(fs::read(store.thumb(&original)).unwrap(), thumb_bytes);
        assert_eq!(fs::read(&index_backup).unwrap(), index_bytes);
        assert_eq!(fs::read_dir(store.root.join("inbox")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(store.root.join("thumbs")).unwrap().count(), 1);
        assert!(!fs::read_dir(&store.root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".json-")
        }));
        fs::remove_dir(&index).unwrap();
        fs::rename(&index_backup, &index).unwrap();
        assert_eq!(
            Store::load(root.join("data")).unwrap().items[0].id,
            original.id
        );
        remove_test_root(root);
    }
    #[test]
    fn source_preserved_deduplicated_and_paths_validated() {
        let root =
            std::env::temp_dir().join(format!("snapline-test-{}-{}", std::process::id(), now()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("测试 screenshot.png");
        image::RgbaImage::from_pixel(12, 8, image::Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let mut store = Store::load(root.join("data")).unwrap();
        let item = store.import(&source).unwrap().unwrap();
        assert!(store.path(&item).exists());
        assert!(store.import(&source).unwrap().is_none());
        assert_eq!((item.width, item.height), (12, 8));
        let mut invalid = item.clone();
        invalid.id = "../outside".into();
        assert!(!valid_item(&invalid));
        store.remove(&item.id, false).unwrap();
        assert!(source.exists());
        let mut config = Settings::default();
        config.max_items = usize::MAX;
        config.language = "???".into();
        config.validate();
        assert_eq!(config.max_items, 100);
        assert_eq!(config.language, "zh-CN");
        fs::write(root.join("bad.png"), b"not an image").unwrap();
        assert!(store.import(&root.join("bad.png")).is_err());
        // Only remove our uniquely named test directory after validating its parent.
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }
}
