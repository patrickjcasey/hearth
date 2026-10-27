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
//! A damaged file (empty, cut short, not JSON) is moved aside as
//! `<name>.corrupt-<time>` so nothing overwrites it. A file that is valid JSON
//! but in a shape this version doesn't know, say from a newer app, is left
//! exactly where it is and not touched; renaming it would delete it on every
//! synced device. Once a day
//! the app also keeps a copy of the library under `backups/` (see
//! [`snapshot`]).
//!
//! [`photo`]: hestia_core::photo

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};

use hestia_core::library::{self, Entry};
use hestia_core::photo::{photo_stem, stem_belongs_to};
use hestia_core::{GroceryItem, Recipe};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

/// The whole library in memory. Also the format of exports and of the old
/// single-file library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
    /// Recipes with a file this version can't read; never written or removed.
    held: HashSet<Uuid>,
    /// Whether `groceries.json` is in a shape this version can't read.
    groceries_held: bool,
}

impl Library {
    /// Load the library in `dir`, moving an old single-file library over
    /// and tidying up any conflicting copies a sync service left behind.
    pub fn open(dir: impl Into<PathBuf>) -> (Self, AppData) {
        let dir = dir.into();
        let recipes_dir = dir.join("recipes");
        let legacy: Option<AppData> = load(&dir.join(LEGACY_FILE)).ok();

        // Every readable entry and the file it came from, plus the ids of
        // files in a shape this version doesn't know. Those ids are left
        // alone entirely: hidden here, and never written or removed.
        let mut found: Vec<(PathBuf, Entry)> = Vec::new();
        let mut held = HashSet::new();
        let files = std::fs::read_dir(&recipes_dir).into_iter().flatten().filter_map(|f| f.ok());
        for path in files.map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "json")) {
            match load::<Entry>(&path) {
                Ok(entry) => found.push((path, entry)),
                Err(Unreadable::Unknown(json)) => held.extend(held_id(&path, &json)),
                Err(_) => {}
            }
        }
        found.retain(|(_, entry)| !held.contains(&entry.id()));
        let legacy_recipes = legacy
            .iter()
            .flat_map(|d| d.recipes.iter().cloned().map(Entry::Recipe))
            .filter(|e| !held.contains(&e.id()));
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

        let (groceries, groceries_held) = match load(&dir.join(GROCERIES_FILE)) {
            Ok(groceries) => (Some(groceries), false),
            Err(e) => (None, matches!(e, Unreadable::Unknown(_))),
        };
        let groceries = groceries
            .or_else(|| legacy.as_ref().map(|d| d.groceries.clone()).filter(|_| !groceries_held))
            .unwrap_or_default();
        if legacy.is_some() {
            if !groceries_held {
                write(&dir.join(GROCERIES_FILE), &groceries);
            }
            let _ = std::fs::rename(dir.join(LEGACY_FILE), dir.join("data.json.migrated"));
        }

        let data = AppData { recipes: library::live_recipes(&merged), groceries };
        let library = Library {
            dir,
            on_disk: merged,
            groceries_on_disk: data.groceries.clone(),
            held,
            groceries_held,
        };
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
            if self.groceries_held {
                eprintln!("not saving groceries over a list this version can't read");
            } else {
                write(&self.dir.join(GROCERIES_FILE), &data.groceries);
            }
            self.groceries_on_disk = data.groceries.clone();
        }
    }

    /// Merge an export (or an old single-file library) into `current`. The
    /// usual rules apply, so a recipe deleted after the export was made stays
    /// deleted and a newer local edit beats the exported copy. Grocery items
    /// not already on the list are added.
    pub fn import(&self, current: &AppData, incoming: AppData) -> (AppData, Imported) {
        let tombstones = self.on_disk.values().filter(|e| e.recipe().is_none()).cloned();
        let mine = current.recipes.iter().cloned().map(Entry::Recipe);
        // Recipes this version can't read on disk stay as they are.
        let theirs = incoming.recipes.into_iter().filter(|r| !self.held.contains(&r.id)).map(Entry::Recipe);
        let recipes = library::live_recipes(&library::merge(tombstones.chain(mine).chain(theirs)));

        let mut summary = Imported::default();
        for r in &recipes {
            match current.recipe(r.id) {
                None => summary.added += 1,
                Some(before) if before != r => summary.updated += 1,
                Some(_) => {}
            }
        }
        let mut groceries = current.groceries.clone();
        let known: HashSet<Uuid> = groceries.iter().map(|g| g.id).collect();
        groceries.extend(incoming.groceries.into_iter().filter(|g| !known.contains(&g.id)));
        (AppData { recipes, groceries }, summary)
    }

    fn store(&mut self, recipes_dir: &Path, entry: Entry) {
        if self.held.contains(&entry.id()) {
            eprintln!("not saving {} over a file this version can't read", entry.id());
            return;
        }
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

/// What [`Library::import`] changed.
#[derive(Debug, Default, PartialEq)]
pub struct Imported {
    pub added: usize,
    pub updated: usize,
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

/// Why [`load`] returned nothing.
#[derive(Debug)]
enum Unreadable {
    /// Missing, or the OS wouldn't read it.
    Missing,
    /// Empty or not JSON; it has been moved aside.
    Damaged,
    /// Valid JSON in a shape this version doesn't know; left untouched.
    Unknown(serde_json::Value),
}

/// Like [`read`] for library files. A damaged file is moved aside so a later
/// save can't overwrite it; one that is valid JSON but not a `T` (perhaps
/// from a newer version) is left in place for the caller to protect.
fn load<T: DeserializeOwned>(path: &Path) -> Result<T, Unreadable> {
    let bytes = std::fs::read(path).map_err(|_| Unreadable::Missing)?;
    let json = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(json) => json,
        Err(e) => {
            let mut aside = path.as_os_str().to_owned();
            aside.push(format!(".corrupt-{}", Utc::now().format("%Y%m%dT%H%M%S%.3f")));
            match std::fs::rename(path, &aside) {
                Ok(()) => eprintln!("moved damaged {} aside ({e})", path.display()),
                Err(re) => eprintln!("damaged {} ({e}) could not be moved: {re}", path.display()),
            }
            return Err(Unreadable::Damaged);
        }
    };
    T::deserialize(&json).map_err(|e| {
        eprintln!("leaving {} alone, written in an unknown format: {e}", path.display());
        Unreadable::Unknown(json)
    })
}

/// The recipe an unrecognized file belongs to: its `id` field, or else the
/// id its name starts with.
fn held_id(path: &Path, json: &serde_json::Value) -> Option<Uuid> {
    let from_json = json.get("id").and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
    let name = path.file_name()?.to_str()?;
    from_json.or_else(|| name.get(..36)?.parse().ok())
}

fn write(path: &Path, value: &impl Serialize) {
    if let Err(e) = try_write(path, value) {
        eprintln!("failed to save {}: {e}", path.display());
    }
}

/// Write-then-rename so a crash mid-write leaves the old file intact. The
/// data is flushed before the rename so a power cut can't leave the new name
/// pointing at an empty file.
fn try_write(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)
}

/// How many daily snapshots [`snapshot`] keeps.
pub const KEEP_SNAPSHOTS: usize = 7;

/// Copy the library's recipe files and grocery list into
/// `backups/<today>/`, at most once a day and only if something changed
/// since the newest snapshot, then drop all but the newest
/// [`KEEP_SNAPSHOTS`]. Photos are left out; they can be downloaded again.
/// Returns whether a snapshot was taken.
///
/// To restore, quit the app and copy a snapshot's files over the library's.
pub fn snapshot(library_dir: &Path, backups: &Path, today: NaiveDate) -> std::io::Result<bool> {
    let target = backups.join(today.to_string());
    if target.exists() {
        return Ok(false);
    }
    let files = library_files(library_dir);
    let newest = snapshots(backups).pop();
    if files.is_empty() || newest.is_some_and(|(_, dir)| library_files(&dir) == files) {
        return Ok(false);
    }

    // Built beside the target and renamed into place, so a half-written
    // snapshot never looks like a real one.
    let tmp = backups.join(format!(".{today}.tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    for (name, bytes) in &files {
        let path = tmp.join(name);
        std::fs::create_dir_all(path.parent().expect("snapshot files sit in a folder"))?;
        std::fs::write(path, bytes)?;
    }
    std::fs::rename(&tmp, &target)?;

    let all = snapshots(backups);
    for (_, old) in &all[..all.len().saturating_sub(KEEP_SNAPSHOTS)] {
        if let Err(e) = std::fs::remove_dir_all(old) {
            eprintln!("failed to remove {}: {e}", old.display());
        }
    }
    Ok(true)
}

/// Snapshot folders in `backups`, oldest first.
fn snapshots(backups: &Path) -> Vec<(NaiveDate, PathBuf)> {
    let mut found: Vec<(NaiveDate, PathBuf)> = std::fs::read_dir(backups)
        .into_iter()
        .flatten()
        .filter_map(|f| f.ok().map(|f| f.path()))
        .filter_map(|p| Some((p.file_name()?.to_str()?.parse().ok()?, p)))
        .collect();
    found.sort();
    found
}

/// The files a snapshot holds, by path relative to the library, sorted.
fn library_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let recipes = std::fs::read_dir(dir.join("recipes"))
        .into_iter()
        .flatten()
        .filter_map(|f| f.ok().map(|f| f.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| Some(format!("recipes/{}", p.file_name()?.to_str()?)));
    let mut files: Vec<(String, Vec<u8>)> = recipes
        .chain([GROCERIES_FILE.to_owned()])
        .filter_map(|name| Some((std::fs::read(dir.join(&name)).ok()?, name)))
        .map(|(bytes, name)| (name, bytes))
        .collect();
    files.sort();
    files
}

/// Where exports go: Downloads on the Mac, and on iOS the app's Documents
/// folder, which the Files app can show.
pub fn export_dir() -> PathBuf {
    #[cfg(target_os = "ios")]
    let dir = dirs::document_dir();
    #[cfg(not(target_os = "ios"))]
    let dir = dirs::download_dir();
    dir.unwrap_or_else(app_dir)
}

/// Save the whole library as one `hestia-export-<today>.json` in `dir`. The
/// file has the old single-file layout, so [`Library::import`] (or moving it
/// in as `data.json`) reads it back.
pub fn export(data: &AppData, dir: &Path, today: NaiveDate) -> std::io::Result<PathBuf> {
    let path = dir.join(format!("hestia-export-{today}.json"));
    try_write(&path, data)?;
    Ok(path)
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

    /// Names in `dir` starting with `prefix`.
    fn names_like(dir: &Path, prefix: &str) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|f| f.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.starts_with(prefix))
            .collect()
    }

    #[test]
    fn writes_leave_no_temporary_files() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        library.save(&sample());
        assert!(tmp.files().iter().all(|f| f.ends_with(".json")), "{:?}", tmp.files());
        assert!(!tmp.0.join("groceries.json.tmp").exists());
    }

    #[test]
    fn an_unreadable_recipe_is_set_aside_not_overwritten() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let data = sample();
        library.save(&data);

        // The file is damaged, and a sync service left an older copy beside it.
        let recipes_dir = tmp.0.join("recipes");
        let soup = &data.recipes[0];
        let path = recipe_path(&recipes_dir, soup.id);
        let older = recipes_dir.join(format!("{} 2.json", soup.id));
        std::fs::rename(&path, &older).unwrap();
        std::fs::write(&path, "{\"id\": \"trunc").unwrap();

        let (_, reopened) = Library::open(&tmp.0);
        assert_eq!(reopened, data, "the readable copy is used");
        let aside = names_like(&recipes_dir, &format!("{}.json.corrupt-", soup.id));
        assert_eq!(aside.len(), 1, "{:?}", tmp.files());
        assert_eq!(std::fs::read_to_string(recipes_dir.join(&aside[0])).unwrap(), "{\"id\": \"trunc");

        // With no other copy, the recipe is skipped but neither deleted nor lost.
        let bread = &data.recipes[1];
        std::fs::write(recipe_path(&recipes_dir, bread.id), "garbage").unwrap();
        let (mut library, reopened) = Library::open(&tmp.0);
        assert_eq!(reopened.recipes, vec![soup.clone()]);
        library.save(&reopened);
        assert!(!recipe_path(&recipes_dir, bread.id).exists(), "no tombstone was written");
        assert_eq!(names_like(&recipes_dir, &format!("{}.json.corrupt-", bread.id)).len(), 1);
    }

    #[test]
    fn an_unreadable_grocery_list_is_set_aside() {
        let tmp = TempDir::new();
        std::fs::write(tmp.0.join(GROCERIES_FILE), "[{\"text\":").unwrap();
        let (mut library, data) = Library::open(&tmp.0);
        assert!(data.groceries.is_empty());
        library.save(&AppData { groceries: vec![GroceryItem::new("Eggs", None)], ..data });

        let aside = names_like(&tmp.0, "groceries.json.corrupt-");
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read_to_string(tmp.0.join(&aside[0])).unwrap(), "[{\"text\":");
    }

    #[test]
    fn a_file_from_a_newer_version_is_left_alone() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let mut data = sample();
        library.save(&data);
        let recipes_dir = tmp.0.join("recipes");

        // Valid JSON this version can't read, beside an older readable copy,
        // next to a file that was cut short.
        let soup = data.recipes[0].clone();
        let newer = format!(r#"{{"id": "{}", "format": 2, "title": {{"en": "Soup"}}}}"#, soup.id);
        let path = recipe_path(&recipes_dir, soup.id);
        let copy = recipes_dir.join(format!("{} 2.json", soup.id));
        std::fs::rename(&path, &copy).unwrap();
        std::fs::write(&path, &newer).unwrap();
        let bread = &data.recipes[1];
        std::fs::write(recipe_path(&recipes_dir, bread.id), "{\"id\": \"tru").unwrap();
        let groceries = r#"{"lists": []}"#;
        std::fs::write(tmp.0.join(GROCERIES_FILE), groceries).unwrap();

        let (mut library, mut reopened) = Library::open(&tmp.0);
        assert!(reopened.recipes.is_empty(), "both are hidden: {reopened:?}");
        assert!(reopened.groceries.is_empty());
        // Saving, even a recipe with the held id, writes nothing over it.
        reopened.recipes.push(soup.clone());
        reopened.groceries.push(GroceryItem::new("Eggs", None));
        library.save(&reopened);
        library.save(&AppData::default());

        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer, "untouched byte for byte");
        assert!(copy.exists(), "its conflicting copy is kept");
        assert!(names_like(&recipes_dir, &format!("{}.json.corrupt-", soup.id)).is_empty());
        assert_eq!(std::fs::read_to_string(tmp.0.join(GROCERIES_FILE)).unwrap(), groceries);
        assert!(names_like(&tmp.0, "groceries.json.corrupt-").is_empty());
        assert_eq!(names_like(&recipes_dir, &format!("{}.json.corrupt-", bread.id)).len(), 1);

        // Once the newer version is gone, the readable copy is used again.
        std::fs::remove_file(&path).unwrap();
        data.recipes.remove(1);
        data.groceries.clear();
        assert_eq!(Library::open(&tmp.0).1, data);
    }

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
    }

    #[test]
    fn snapshots_daily_only_when_changed() {
        let tmp = TempDir::new();
        let backups = tmp.0.join("backups");
        assert!(!snapshot(&tmp.0, &backups, day(1)).unwrap(), "nothing to back up yet");

        let (mut library, _) = Library::open(&tmp.0);
        let mut data = sample();
        library.save(&data);
        assert!(snapshot(&tmp.0, &backups, day(1)).unwrap());
        let first = backups.join("2026-10-01");
        assert_eq!(library_files(&first), library_files(&tmp.0));
        assert_eq!(library_files(&first).len(), 3);

        data.recipes[0].title = "Onion soup".into();
        data.recipes[0].touch();
        library.save(&data);
        assert!(!snapshot(&tmp.0, &backups, day(1)).unwrap(), "once a day");
        assert!(snapshot(&tmp.0, &backups, day(2)).unwrap());
        assert!(!snapshot(&tmp.0, &backups, day(3)).unwrap(), "unchanged since day 2");

        // A snapshot restores the library as it was, deleted recipes included.
        let gone = data.recipes.remove(1);
        library.save(&data);
        std::fs::copy(
            first.join(format!("recipes/{}.json", gone.id)),
            recipe_path(&tmp.0.join("recipes"), gone.id),
        )
        .unwrap();
        assert!(Library::open(&tmp.0).1.recipe(gone.id).is_some());
    }

    #[test]
    fn keeps_the_newest_snapshots() {
        let tmp = TempDir::new();
        let backups = tmp.0.join("backups");
        let (mut library, _) = Library::open(&tmp.0);
        let mut data = sample();
        for n in 1..=10 {
            data.groceries.push(GroceryItem::new(format!("item {n}"), None));
            library.save(&data);
            assert!(snapshot(&tmp.0, &backups, day(n)).unwrap());
        }
        let kept: Vec<NaiveDate> = snapshots(&backups).into_iter().map(|(d, _)| d).collect();
        assert_eq!(kept, (4..=10).map(day).collect::<Vec<_>>());
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), KEEP_SNAPSHOTS, "no leftovers");
    }

    #[test]
    fn export_round_trips_through_import() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let data = sample();
        library.save(&data);
        let path = export(&data, &tmp.0, day(24)).unwrap();
        assert!(path.ends_with("hestia-export-2026-10-24.json"));

        // On a fresh install, importing brings everything back.
        let fresh = TempDir::new();
        let (mut other, empty) = Library::open(&fresh.0);
        let incoming: AppData = read(&path).unwrap();
        let (restored, summary) = other.import(&empty, incoming);
        assert_eq!(restored, data);
        assert_eq!(summary, Imported { added: 2, updated: 0 });
        other.save(&restored);
        assert_eq!(Library::open(&fresh.0).1, data);
    }

    #[test]
    fn import_never_undoes_newer_changes() {
        let tmp = TempDir::new();
        let (mut library, _) = Library::open(&tmp.0);
        let exported = sample();
        library.save(&exported);

        // After the export, one recipe was deleted and the other edited.
        let mut now = exported.clone();
        let deleted = now.recipes.remove(1);
        now.recipes[0].title = "Onion soup".into();
        now.recipes[0].updated_at += Duration::minutes(1);
        now.groceries.clear();
        library.save(&now);

        let mut extra = Recipe::new("Pie");
        extra.created_at = deleted.created_at - Duration::days(1);
        let mut incoming = exported.clone();
        incoming.recipes.push(extra.clone());
        let (merged, summary) = library.import(&now, incoming);
        assert_eq!(merged.recipes, vec![now.recipes[0].clone(), extra]);
        assert!(merged.recipe(deleted.id).is_none(), "the delete stands");
        assert_eq!(merged.groceries, exported.groceries, "missing items come back");
        assert_eq!(summary, Imported { added: 1, updated: 0 });
    }

    #[test]
    fn reads_an_export_missing_fields() {
        let data: AppData = serde_json::from_str(r#"{"recipes": []}"#).unwrap();
        assert_eq!(data, AppData::default());
    }
}
