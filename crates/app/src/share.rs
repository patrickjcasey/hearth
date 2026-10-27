//! Share links.
//!
//! Only when the user confirms Share does the app upload the recipe's
//! `SharedRecipe` fields (no notes, photo files, ids or favorite flag) to the
//! Hestia server, which hosts a read-only page for `SHARE_DAYS` days. That page
//! carries schema.org JSON-LD, so Hestia and other recipe apps can import it.

use std::time::Duration;

use hestia_core::{ShareLink, SharedRecipe};

/// Where shares are posted. Set at build time, e.g.
/// `HESTIA_SHARE_URL=https://example.com/api/share dx bundle ...`;
/// builds without it have no Share button.
pub const SHARE_URL: Option<&str> = option_env!("HESTIA_SHARE_URL");
const _: () = assert!(
    hestia_core::url::is_build_endpoint(SHARE_URL),
    "HESTIA_SHARE_URL must start with https:// (http://127.0.0.1 or http://localhost for local testing)"
);

/// The server's host name, to tell the user where the recipe is going.
pub fn host() -> Option<&'static str> {
    host_of(SHARE_URL?)
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().filter(|h| !h.is_empty())
}

/// Uploads `recipe` and returns its public link.
pub async fn create_link(recipe: &SharedRecipe) -> Result<ShareLink, String> {
    let url = SHARE_URL.ok_or("Sharing isn't available in this build.")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(recipe).expect("recipe serializes"))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the server: {e}"))?;
    let status = res.status();
    let body = res.bytes().await.map_err(|e| format!("Couldn't reach the server: {e}"))?;
    if status.is_success() {
        return serde_json::from_slice(&body).map_err(|e| format!("Unexpected reply from the server: {e}"));
    }
    Err(match status.as_u16() {
        413 => "This recipe is too long to share.".into(),
        422 => format!("This recipe can't be shared: {}", String::from_utf8_lossy(&body)),
        429 => "Too many recipes are being shared right now. Try again later.".into(),
        _ => format!("Sharing failed ({status})."),
    })
}

/// Puts `text` on the system clipboard. Returns false where unsupported, so
/// the caller can leave the link on screen for the user to copy by hand.
pub fn copy_to_clipboard(text: &str) -> bool {
    imp::copy(text)
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    pub fn copy(text: &str) -> bool {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        pasteboard.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2_foundation::NSString;
    use objc2_ui_kit::UIPasteboard;

    pub fn copy(text: &str) -> bool {
        unsafe { UIPasteboard::generalPasteboard().setString(Some(&NSString::from_str(text))) };
        true
    }
}

// TODO(android, linux, windows): use the platform clipboard.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
mod imp {
    pub fn copy(_text: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_the_server() {
        assert_eq!(host_of("https://hestia.example.com/api/share"), Some("hestia.example.com"));
        assert_eq!(host_of("http://127.0.0.1:8000/api/share"), Some("127.0.0.1:8000"));
        assert_eq!(host_of("https://"), None);
    }
}
