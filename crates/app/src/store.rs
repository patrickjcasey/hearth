//! On-device storage. Everything lives locally so the app works offline and
//! costs nothing to run.
//!
//! The library is a folder with one file per recipe, so it can sit in a
//! synced folder such as iCloud Drive: two devices editing different recipes
//! never touch the same file, and [`hestia_core::library`] settles edits to
//! the same one.
//!
//! ```text
//! recipes/<id>.json          a recipe, or a tombstone once it is deleted
//! recipes/<id>-<hash>.jpg    its photo, shrunk for local use (see [`photo`])
//! groceries.json
//! ```
//!
//! [`photo`]: hestia_core::photo

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use hestia_core::library::{self, Entry};
use hestia_core::photo::{photo_stem, stem_belongs_to};
use hestia_core::{GroceryItem, Recipe};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppData {
    pub recipes: Vec<Recipe>,
    pub groceries: Vec<GroceryItem>,
}

impl AppData {
    pub fn recipe(&self, id: Uuid) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.id == id)
    }
}

/// The app's private folder. Per-device files such as settings always stay
/// here, even once the library can live elsewhere.
pub fn app_dir() -> PathBuf {
    // On iOS, HOME is the app's sandbox container.
    let base = dirs::data_dir()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("hestia")
}

/// Before the per-recipe layout, everything was saved in this one file.
const LEGACY_FILE: &str = "data.json";
const GROCERIES_FILE: &str = "groceries.json";

/// A library folder, plus what was last written to it so saves only touch
/// recipes that changed.
pub struct Library {
    dir: PathBuf,
    on_disk: HashMap<Uuid, Entry>,
    groceries_on_disk: Vec<GroceryItem>,
}

impl Library {
    /// Load the library in `dir`, moving an old single-file library over
    /// and tidying up any conflicting copies a sync service left behind.
    pub fn open(dir: impl Into<PathBuf>) -> (Self, AppData) {
        let dir = dir.into();
        let recipes_dir = dir.join("recipes");
        let legacy: Option<AppData> = read(&dir.join(LEGACY_FILE));

        // Every readable entry, and the file it came from.
        let mut found: Vec<(PathBuf, Entry)> = std::fs::read_dir(&recipes_dir)
            .into_iter()
            .flatten()
            .filter_map(|f| f.ok().map(|f| f.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .filter_map(|p| read::<Entry>(&p).map(|e| (p, e)))
            .collect();
        let legacy_recipes = legacy.iter().flat_map(|d| d.recipes.iter().cloned().map(Entry::Recipe));
        let merged = library::merge(found.iter().map(|(_, e)| e.clone()).chain(legacy_recipes));

        // Write each winner to its own file, then drop the extra copies.
        found.retain(|(path, entry)| *path != recipe_path(&recipes_dir, entry.id()));
        for (id, entry) in &merged {
            let path = recipe_path(&recipes_dir, *id);
            if read::<Entry>(&path).as_ref() != Some(entry) {
                write(&path, entry);
            }
        }
        for (copy, _) in found {
            if let Err(e) = std::fs::remove_file(&copy) {
                eprintln!("failed to remove {}: {e}", copy.display());
            }
        }

        let groceries = read(&dir.join(GROCERIES_FILE))
            .or_else(|| legacy.as_ref().map(|d| d.groceries.clone()))
            .unwrap_or_default();
        if legacy.is_some() {
            write(&dir.join(GROCERIES_FILE), &groceries);
            let _ = std::fs::rename(dir.join(LEGACY_FILE), dir.join("data.json.migrated"));
        }

        let data = AppData { recipes: library::live_recipes(&merged), groceries };
        let library = Library { dir, on_disk: merged, groceries_on_disk: data.groceries.clone() };
        (library, data)
    }

    /// Write whatever changed since the last save. Recipes missing from
    /// `data` were deleted, so they get a tombstone.
    pub fn save(&mut self, data: &AppData) {
        let recipes_dir = self.dir.join("recipes");
        let mut live = HashSet::with_capacity(data.recipes.len());
        for recipe in &data.recipes {
            live.insert(recipe.id);
            let before = self.on_disk.get(&recipe.id).and_then(Entry::recipe);
            if before != Some(recipe) {
                if before.is_none_or(|b| b.image_url != recipe.image_url) {
                    let keep = recipe.image_url.as_deref().map(|url| photo_stem(recipe.id, url));
                    prune_photos(&recipes_dir, recipe.id, keep.as_deref());
                }
                self.store(&recipes_dir, Entry::Recipe(recipe.clone()));
            }
        }
        let deleted: Vec<Uuid> = self
            .on_disk
            .iter()
            .filter(|(id, entry)| entry.recipe().is_some() && !live.contains(*id))
            .map(|(id, _)| *id)
            .collect();
        for id in deleted {
            prune_photos(&recipes_dir, id, None);
            self.store(&recipes_dir, Entry::deleted(id));
        }

        if data.groceries != self.groceries_on_disk {
            write(&self.dir.join(GROCERIES_FILE), &data.groceries);
            self.groceries_on_disk = data.groceries.clone();
        }
    }

    fn store(&mut self, recipes_dir: &Path, entry: Entry) {
        write(&recipe_path(recipes_dir, entry.id()), &entry);
        self.on_disk.insert(entry.id(), entry);
    }

    /// The folder holding recipes and their photos.
    pub fn recipes_dir(&self) -> PathBuf {
        self.dir.join("recipes")
    }

    /// Stems (see [`photo_stem`]) of every photo already on disk.
    pub fn cached_photos(&self) -> HashSet<String> {
        std::fs::read_dir(self.recipes_dir())
            .into_iter()
            .flatten()
            .filter_map(|f| f.ok().map(|f| f.path()))
            .filter(|p| p.extension().is_some_and(|e| e == PHOTO_EXT))
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
            .collect()
    }

    /// Store a shrunk photo, replacing any earlier photo of the same recipe.
    pub fn save_photo(&self, stem: &str, jpeg: &[u8]) -> std::io::Result<()> {
        let recipes_dir = self.recipes_dir();
        if let Some(id) = recipe_id(stem) {
            prune_photos(&recipes_dir, id, Some(stem));
        }
        std::fs::create_dir_all(&recipes_dir)?;
        let path = photo_path(&recipes_dir, stem);
        let tmp = path.with_extension("jpg.tmp");
        std::fs::write(&tmp, jpeg)?;
        std::fs::rename(tmp, path)
    }
}

const PHOTO_EXT: &str = "jpg";

fn recipe_path(recipes_dir: &Path, id: Uuid) -> PathBuf {
    recipes_dir.join(format!("{id}.json"))
}

pub fn photo_path(recipes_dir: &Path, stem: &str) -> PathBuf {
    recipes_dir.join(format!("{stem}.{PHOTO_EXT}"))
}

/// The recipe a photo stem belongs to.
fn recipe_id(stem: &str) -> Option<Uuid> {
    stem.get(..36).and_then(|s| s.parse().ok()).filter(|id| stem_belongs_to(stem, *id))
}

/// Remove every photo of recipe `id` except `keep`.
fn prune_photos(recipes_dir: &Path, id: Uuid, keep: Option<&str>) {
    let stale = std::fs::read_dir(recipes_dir)
        .into_iter()
        .flatten()
        .filter_map(|f| f.ok().map(|f| f.path()))
        .filter(|p| p.extension().is_some_and(|e| e == PHOTO_EXT))
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem_belongs_to(stem, id) && Some(stem) != keep)
        });
    for path in stale {
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("failed to remove {}: {e}", path.display());
        }
    }
}

/// Read `name` from the app folder, falling back to the default if it is
/// missing or unreadable.
pub fn read_json<T: DeserializeOwned + Default>(name: &str) -> T {
    read(&app_dir().join(name)).unwrap_or_default()
}

pub fn write_json(name: &str, value: &impl Serialize) {
    write(&app_dir().join(name), value);
}

fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text)
        .inspect_err(|e| eprintln!("skipping unreadable {}: {e}", path.display()))
        .ok()
}

fn write(path: &Path, value: &impl Serialize) {
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write-then-rename so a crash mid-write can't corrupt the file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
        std::fs::rename(tmp, path)
    })();
    if let Err(e) = result {
        eprintln!("failed to save {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    /// A fresh folder, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("hestia-test-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.0.join("recipes"))
                .unwrap()
                .map(|f| f.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> AppData {
        let mut soup = Recipe::new("Soup");
        let mut bread = Recipe::new("Bread");
        bread.created_at = soup.created_at - Duration::days(1);
        soup.favorite = true;
        AppData { recipes: vec![soup, bread], groceries: vec![GroceryItem::new("Salt", None)] }
    }

    #[test]
    fn saves_and_reopens() {
        let tmp = TempDir::new();
        let (mut library, empty) = Library::open(&tmp.0);
        assert_eq!(empty, AppData::default());

        let data = sample();
        library.save(&data);
        assert_eq!(tmp.files().len(), 2);
        assert_eq!(Library::open(&tmp.0).1, data);
    }

    #[test]
    fn only_changed_recipes_are_written() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let mut data = sample();
        library.save(&data);

        // Make the untouched recipe's file unreadable; a save that rewrote
        // it would repair it.
        let untouched = recipe_path(&tmp.0.join("recipes"), data.recipes[1].id);
        std::fs::write(&untouched, "sentinel").unwrap();
        data.recipes[0].title = "Onion soup".into();
        data.recipes[0].touch();
        library.save(&data);

        assert_eq!(std::fs::read_to_string(untouched).unwrap(), "sentinel");
        assert_eq!(Library::open(&tmp.0).1.recipes[0].title, "Onion soup");
    }

    #[test]
    fn deleting_leaves_a_tombstone() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let mut data = sample();
        library.save(&data);

        let gone = data.recipes.remove(0);
        library.save(&data);

        let stored: Entry = read(&recipe_path(&tmp.0.join("recipes"), gone.id)).unwrap();
        assert!(matches!(stored, Entry::Deleted(_)));
        assert_eq!(Library::open(&tmp.0).1, data);
    }

    #[test]
    fn photos_follow_their_recipe() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let mut data = sample();
        data.recipes[0].image_url = Some("https://example.com/soup.jpg".into());
        library.save(&data);

        let old = photo_stem(data.recipes[0].id, "https://example.com/soup.jpg");
        library.save_photo(&old, b"jpeg bytes").unwrap();
        assert_eq!(library.cached_photos(), HashSet::from([old.clone()]));
        assert_eq!(Library::open(&tmp.0).0.cached_photos(), HashSet::from([old.clone()]));

        // A new link makes the old photo stale, even before the new one arrives.
        data.recipes[0].image_url = Some("https://example.com/better-soup.jpg".into());
        data.recipes[0].touch();
        library.save(&data);
        assert!(library.cached_photos().is_empty());
        let new = photo_stem(data.recipes[0].id, "https://example.com/better-soup.jpg");
        library.save_photo(&new, b"jpeg bytes").unwrap();

        // Deleting the recipe removes its photo too.
        data.recipes.remove(0);
        library.save(&data);
        assert!(library.cached_photos().is_empty());
        assert_eq!(tmp.files().len(), 2, "{:?}", tmp.files());
    }

    #[test]
    fn keeps_the_newest_conflicting_copy() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let data = sample();
        library.save(&data);

        // A sync service saved another device's newer edit beside ours.
        let mut theirs = data.recipes[0].clone();
        theirs.title = "Their soup".into();
        theirs.updated_at += Duration::minutes(1);
        let recipes_dir = tmp.0.join("recipes");
        write(&recipes_dir.join(format!("{} 2.json", theirs.id)), &Entry::Recipe(theirs.clone()));

        let (_, reopened) = Library::open(&tmp.0);
        assert_eq!(reopened.recipes[0], theirs);
        assert_eq!(tmp.files().len(), 2, "the extra copy is removed: {:?}", tmp.files());
    }

    #[test]
    fn moves_an_old_single_file_library() {
        let tmp = TempDir::new();
        let data = sample();
        write(&tmp.0.join(LEGACY_FILE), &data);

        let (_, migrated) = Library::open(&tmp.0);
        assert_eq!(migrated, data);
        assert!(!tmp.0.join(LEGACY_FILE).exists());
        assert!(tmp.0.join("data.json.migrated").exists());
        assert_eq!(tmp.files().len(), 2);
        assert_eq!(Library::open(&tmp.0).1, data);
    }
}
