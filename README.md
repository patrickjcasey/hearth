# Hestia

A free recipe box written in Rust from end to end. You can save recipes from any website or write your own, scale servings, cook step by step with the screen kept awake, and build a grocery list. It runs on macOS and iOS from a single codebase, and Android support is possible later.

## Architecture

```
crates/
  core/    hestia-core: shared domain logic, no UI
           - Recipe / GroceryItem / MealPlanEntry models
           - Ingredient parsing + serving scaling ("1 1/2 cups")
           - Recipe import from schema.org JSON-LD (works on most recipe sites)
           - UsageReport: the only data the app can ever send home
  app/     hestia-app: Dioxus UI, one codebase for macOS, iOS, (Android)
           - Local-first: data lives in a JSON file on the device
           - Imports recipes directly from the device; no server required
           - Cook mode keeps the screen awake (see src/wake.rs):
             iOS idleTimerDisabled, macOS NSProcessInfo activity,
             Android TODO
           - Opt-in usage statistics (src/analytics.rs), see below
  server/  hestia-server: the website (Axum + SQLite)
           - Landing, privacy and support pages (site/)
           - POST /api/events receives opt-in usage reports
```

The app is local-first, so it works offline and costs nothing to run. Recipes never go to the server.

The library is a folder with one file per recipe (`recipes/<id>.json`) plus `groceries.json`, so it can live in a synced folder. A deleted recipe leaves a small tombstone file, so a device that was offline during the delete doesn't bring the recipe back. When two versions of a recipe meet, including conflicting copies left by a sync service, the one with the newest `updated_at` wins (`core/src/library.rs`). Libraries saved in the older single-file format (`data.json`) are moved over automatically the first time the app opens.

Besides its standing **Notes** (edited with the rest of the recipe), each recipe has a **Cooking log**: dated notes such as "bake 5 min less", added from the recipe page or when you finish cook mode. The log is saved inside the recipe's file, so it syncs with the recipe under the same newest-wins rule; adding a note counts as an edit.

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

Page content lives in `crates/server/site/` and is compiled into the binary. Before launch, replace the placeholder `support@example.com`.

Configuration: `DATABASE_URL` (default `sqlite://hestia.db`) and `HESTIA_ADDR` (default `127.0.0.1:8000`). In production, put a TLS-terminating reverse proxy in front of it.

## Roadmap

- Meal planner (the model already exists in `core`)
- iCloud Drive sync on the Mac: a "Data folder" setting that can point at `~/Library/Mobile Documents/com~apple~CloudDocs/Hestia`, and reloading when another device changes files there. This works without a paid Apple Developer account.
- Mac release: a `.dmg` built with `dx bundle` on a GitHub Actions macOS runner
- With a paid Apple Developer account: notarization, the App Store, and the iPhone app, which syncs through an iCloud container
- Share and export recipes
- Android release

## License

MIT. See [LICENSE](LICENSE).
