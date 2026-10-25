# Hestia

A free recipe box written in Rust from end to end. You can save recipes from any website or write your own, scale servings, cook step by step with the screen kept awake, and build a grocery list. It runs on macOS and iOS from a single codebase, and Android support is possible later.

## Architecture

```
crates/
  core/    hestia-core: shared domain logic, no UI
           - Recipe / GroceryItem / MealPlanEntry models
           - Ingredient parsing + serving scaling ("1 1/2 cups")
           - Recipe import from schema.org JSON-LD (works on most recipe sites)
           - UsageReport: the only data the app sends home on its own
           - SharedRecipe: what a share link uploads, and its JSON-LD form
  app/     hestia-app: Dioxus UI, one codebase for macOS, iOS, (Android)
           - Local-first: data lives in a JSON file on the device
           - Imports recipes directly from the device; no server required
           - Cook mode keeps the screen awake (see src/wake.rs):
             iOS idleTimerDisabled, macOS NSProcessInfo activity,
             Android TODO
           - Opt-in usage statistics (src/analytics.rs), see below
           - Share links (src/share.rs), see below
  server/  hestia-server: the website (Axum + SQLite)
           - Landing, privacy and support pages (site/)
           - POST /api/events receives opt-in usage reports
           - POST /api/share + GET /r/<id>: recipes shared as links
```

The app is local-first, so it works offline and costs nothing to run. Recipes stay on the device; the only recipe that ever reaches the server is one the user explicitly shares as a link (see Sharing).

The library is a folder with one file per recipe (`recipes/<id>.json`) plus `groceries.json`, so it can live in a synced folder. A deleted recipe leaves a small tombstone file, so a device that was offline during the delete doesn't bring the recipe back. When two versions of a recipe meet, including conflicting copies left by a sync service, the one with the newest `updated_at` wins (`core/src/library.rs`). Libraries saved in the older single-file format (`data.json`) are moved over automatically the first time the app opens.

## Usage statistics

Settings has two switches, and both are off by default:

- **Collect usage statistics** counts events such as app launches, imports and cook-mode sessions on the device. Turning it off deletes the counters and the anonymous install id. Turning it back on creates a new id.
- **Send statistics to Hestia** is only available while collecting. It uploads the counters at most once a day and deletes the sent counts once the upload is accepted.

The Settings screen shows the exact JSON that would be sent. The schema is `hestia_core::UsageReport`, which contains an install id, the app version, the platform, and counts per event. The server rejects any unknown fields or events. These settings are stored in `usage.json` in the local app folder, separate from recipe data, and are never synced.

The upload endpoint is set when the app is built. Builds without it never send anything:

```sh
HESTIA_EVENTS_URL=https://hestia.example.com/api/events dx bundle --platform ios --release
```

Example totals query on the server:

```sql
SELECT event, SUM(count) AS total, COUNT(DISTINCT install_id) AS installs
FROM usage_events WHERE received_at > datetime('now', '-30 days')
GROUP BY event ORDER BY total DESC;
```

## Sharing

A recipe's **Share** button creates a link anyone can open in a browser. Nothing is uploaded until the user confirms **Create link** in the panel, which says what will be sent and where.

- **Sent:** one recipe as `hestia_core::SharedRecipe`: title, description, author, servings, yield, prep/cook/total minutes, ingredient lines, steps, tags, and the source and photo links if they are web links. The server rejects unknown fields.
- **Not sent:** notes, the favorite flag, ids and timestamps, photo files, other recipes, the usage-statistics id, or anything identifying the device.
- **Stored:** in the server's `shares` table under a random 10-character id, with the creation and expiry time. Links expire after 90 days, and expired rows are deleted whenever a new share is created. A link is a snapshot, so later edits don't change it.
- **Served:** `GET /r/<id>` is a readable page with schema.org Recipe JSON-LD embedded, so pasting the link into Hestia's import box (or most other recipe apps) saves a copy. Pages send `noindex`.

When the link is created the app copies it to the clipboard (macOS and iOS; other platforms show it to copy by hand). With usage statistics on, it counts a `recipe_shared` event, never the recipe.

Like the statistics endpoint, the share endpoint is set when the app is built, and builds without it have no Share button:

```sh
HESTIA_SHARE_URL=https://hestia.example.com/api/share dx bundle --platform ios --release
```

## Development

Prerequisites: Rust (stable, via rustup) and the Dioxus CLI (`dx`). Its version must match the `dioxus` crate in `Cargo.lock`.

`nix develop` provides `dx` on Linux and Apple Silicon Macs. On Linux it also provides the webview libraries for the desktop build. Without Nix, install `dx` with `cargo install dioxus-cli --version 0.7.10`.

```sh
cargo test --workspace --all-features   # core + server tests
cargo run -p hestia-server              # site on http://127.0.0.1:8000
cd crates/app && dx serve               # desktop app with hot reload
```

### macOS and iOS

These builds need a Mac with Xcode installed. On the Mac, the Nix shell adds only `dx`, so Xcode's compilers and SDKs still get used. For iOS, also install the Rust targets:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
nix develop
cd crates/app
dx serve --platform desktop                     # native macOS app
dx serve --platform ios                         # iOS simulator
dx bundle --platform ios --release              # .app / .ipa for distribution
dx bundle --platform desktop --release          # .app / .dmg
```

Before submitting to the App Store, set your bundle identifier in `crates/app/Dioxus.toml`.

### Android (later)

```sh
rustup target add aarch64-linux-android
cd crates/app && dx serve --platform android
```

TODO: `store.rs` needs to use the app's internal files directory on Android.

## Website

| Method | Path          | Description                                              |
|--------|---------------|----------------------------------------------------------|
| GET    | `/`           | Landing page                                             |
| GET    | `/privacy`    | Privacy policy (the App Store requires one)              |
| GET    | `/support`    | Support / FAQ (the App Store requires one)               |
| GET    | `/style.css`  | Stylesheet                                               |
| GET    | `/health`     | Liveness                                                 |
| POST   | `/api/events` | Usage report: 204 accepted, 422 invalid, 429 already sent within the last hour, 413 over 4 KB |
| POST   | `/api/share`  | Share a recipe: 201 `{"url", "expires_at"}`, 422 invalid, 429 over 600 shares in the last hour (all users), 413 over 64 KB |
| GET    | `/r/<id>`     | Shared recipe page with JSON-LD; 404 once expired         |

Page content lives in `crates/server/site/` and is compiled into the binary. Before launch, replace the placeholder `support@example.com`.

Configuration: `DATABASE_URL` (default `sqlite://hestia.db`), `HESTIA_ADDR` (default `127.0.0.1:8000`), and `HESTIA_PUBLIC_URL`, the origin used in share links (default `http://<HESTIA_ADDR>`). In production, put a TLS-terminating reverse proxy in front of it.

## Roadmap

- Meal planner (the model already exists in `core`)
- iCloud Drive sync on the Mac: a "Data folder" setting that can point at `~/Library/Mobile Documents/com~apple~CloudDocs/Hestia`, and reloading when another device changes files there. This works without a paid Apple Developer account.
- Mac release: a `.dmg` built with `dx bundle` on a GitHub Actions macOS runner
- With a paid Apple Developer account: notarization, the App Store, and the iPhone app, which syncs through an iCloud container
- Export recipes
- Open `/r/<id>` share links straight in the app (iOS universal links / macOS URL handling) instead of pasting them into import
- Android release

## License

MIT. See [LICENSE](LICENSE).
