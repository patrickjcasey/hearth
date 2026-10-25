//! Local copies of recipe photos. Imported recipes link to a photo on the
//! web; this module downloads a shrunk copy beside the recipe, serves it to
//! the page at `/photos/<stem>.jpg`, and tells views which copies exist.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use dioxus::prelude::*;
use hestia_core::photo::{self, photo_stem};
use uuid::Uuid;

use crate::store::{AppData, Library, photo_path};

#[cfg(feature = "desktop")]
use dioxus::desktop as platform;
#[cfg(all(feature = "mobile", not(feature = "desktop")))]
use dioxus::mobile as platform;

use platform::wry::http::Response;

/// Stems (see [`photo_stem`]) of the photos stored on disk.
pub type Photos = Signal<HashSet<String>>;

/// Page path the photo handler answers under.
const ROUTE: &str = "photos";

/// The page address of a recipe's stored photo, once it has been saved.
pub fn use_photo(id: Uuid, image_url: Option<&str>) -> Option<String> {
    let photos: Photos = use_context();
    let stem = photo_stem(id, image_url?);
    photos.read().contains(&stem).then(|| format!("/{ROUTE}/{stem}.jpg"))
}

/// Serve stored photos to the page and fetch any that are missing.
pub fn use_photo_cache(library: Rc<RefCell<Library>>, data: Signal<AppData>) {
    let mut photos: Photos = use_context_provider(|| Signal::new(library.borrow().cached_photos()));

    let recipes_dir = library.borrow().recipes_dir();
    platform::use_asset_handler(ROUTE, move |request, responder| {
        let file = request
            .uri()
            .path()
            .strip_prefix(&format!("/{ROUTE}/"))
            .and_then(stem_of)
            .map(|stem| photo_path(&recipes_dir, stem));
        let Some(file) = file else {
            responder.respond(not_found());
            return;
        };
        // Off the UI thread: the grid can ask for dozens at once.
        std::thread::spawn(move || {
            responder.respond(match std::fs::read(&file) {
                Ok(bytes) => Response::builder()
                    .header("Content-Type", "image/jpeg")
                    // The stem changes with the link, so a file never changes
                    // meaning; the page can keep it for good.
                    .header("Cache-Control", "max-age=31536000, immutable")
                    .body(bytes)
                    .expect("valid response"),
                Err(_) => not_found(),
            })
        });
    });

    // Each photo is tried once per launch, so a dead link doesn't loop.
    let attempted: Rc<RefCell<HashSet<String>>> = use_hook(Default::default);
    let recipes_dir = library.borrow().recipes_dir();
    use_effect(move || {
        let wanted: Vec<(Uuid, String, String)> = data
            .read()
            .recipes
            .iter()
            .filter_map(|r| r.image_url.as_deref().map(|url| (r.id, photo_stem(r.id, url), url.to_owned())))
            .collect();
        for (id, stem, url) in wanted {
            if photos.peek().contains(&stem) {
                // Saving the library prunes photos of old links, so a link
                // changed A -> B -> A has lost A's file: fetch it again.
                if photo_path(&recipes_dir, &stem).exists() {
                    continue;
                }
                photos.write().remove(&stem);
                attempted.borrow_mut().insert(stem.clone());
            } else if !attempted.borrow_mut().insert(stem.clone()) {
                continue;
            }
            let library = library.clone();
            spawn(async move {
                let jpeg = match download(&url).await {
                    Ok(jpeg) => jpeg,
                    Err(e) => {
                        eprintln!("photo for {url}: {e}");
                        return;
                    }
                };
                // Skip if the recipe went away or changed its link meanwhile.
                let still_wanted = data.peek().recipes.iter().any(|r| r.id == id && r.image_url.as_deref() == Some(&url));
                if !still_wanted {
                    return;
                }
                match library.borrow().save_photo(&stem, &jpeg) {
                    Ok(()) => {
                        photos.write().insert(stem);
                    }
                    Err(e) => eprintln!("could not save photo for {url}: {e}"),
                }
            });
        }
    });
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let bytes = photo::fetch_photo(url).await.map_err(|e| e.to_string())?;
    // Decoding and resizing a large photo takes a noticeable moment.
    tokio::task::spawn_blocking(move || photo::shrink_photo(&bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// The stem in a `<stem>.jpg` request, if it can only name a photo of ours.
fn stem_of(file: &str) -> Option<&str> {
    file.strip_suffix(".jpg")
        .filter(|stem| !stem.is_empty() && stem.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
}

fn not_found() -> Response<Vec<u8>> {
    Response::builder().status(404).body(Vec::new()).expect("valid response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_serves_photo_names() {
        assert_eq!(stem_of("0a1b-2c3d.jpg"), Some("0a1b-2c3d"));
        assert_eq!(stem_of(".jpg"), None);
        assert_eq!(stem_of("../groceries.json"), None);
        assert_eq!(stem_of("x/y.jpg"), None);
        assert_eq!(stem_of("photo.png"), None);
    }
}
