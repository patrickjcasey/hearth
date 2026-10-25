{
  description = "Hestia dev shell. Rust itself comes from rustup; Apple builds also need Xcode.";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      devShells = forAll (pkgs: {
        default =
          if pkgs.stdenv.hostPlatform.isDarwin then
            # No Nix C toolchain here: Xcode's clang and SDKs are needed for
            # the macOS and iOS builds, and Nix's would shadow them.
            pkgs.mkShellNoCC {
              # Must match the dioxus version in Cargo.lock.
              packages = [ pkgs.dioxus-cli ];
            }
          else
            pkgs.mkShell {
              packages = [ pkgs.dioxus-cli ];
              # Libraries needed by the Dioxus desktop (webview) target on Linux.
              nativeBuildInputs = [ pkgs.pkg-config ];
              buildInputs = with pkgs; [
                webkitgtk_4_1 gtk3 libsoup_3 glib cairo pango gdk-pixbuf atk openssl xdotool
              ];
              # WebKit loads GStreamer plugins from the desktop session's path,
              # which come from the system's nixpkgs and need a newer glibc
              # than this shell's. Use plugins built against this nixpkgs.
              GST_PLUGIN_SYSTEM_PATH_1_0 = pkgs.lib.makeSearchPath "lib/gstreamer-1.0" (with pkgs.gst_all_1; [
                gst-plugins-base gst-plugins-good
              ]);
              # WebKit's DMA-BUF renderer fails on NVIDIA ("Failed to create GBM buffer").
              WEBKIT_DISABLE_DMABUF_RENDERER = "1";
              # TLS for the webview, so recipe photos load over https.
              GIO_EXTRA_MODULES = "${pkgs.glib-networking}/lib/gio/modules";
              # Nix binaries find their own libraries; a session-wide
              # LD_LIBRARY_PATH (e.g. the system's alsa-lib) would override them.
              shellHook = ''
                unset GST_PLUGIN_PATH GST_PLUGIN_PATH_1_0 LD_LIBRARY_PATH
              '';
            };
      });
    };
}
