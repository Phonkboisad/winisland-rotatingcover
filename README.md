# Rotating CD

Rotating CD is a native WinIsland plugin that presents the current album artwork as a circular disc in the compact island. The artwork and subtle concentric grooves rotate together once every 2.5 seconds during playback; there is no center hub or white arc. The artwork is cropped to a circle and slightly overlaid beyond the native cover bounds to conceal its square corners. The disc holds still when playback is paused or stopped. If album art is not yet available through the host API, the plugin leaves the host's native cover visible instead of covering it with a black placeholder.

The visual treatment follows the supplied example's circular crop and uses the album-art position as an overlay, preserving WinIsland's native track title and visualizer. The plugin is a native WinIsland DLL using the official ABI v2 SDK; it does not load the CSS or depend on Spicetify at runtime.

## Requirements

- Windows 10 version 2004 or later, or Windows 11
- Stable Rust and the `x86_64-pc-windows-msvc` target
- Visual Studio C++ Build Tools and Windows SDK
- A WinIsland host with the required ABI v2 Surface, Events, Host State, and Image interfaces

## Build and test

```powershell
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo test --locked
cargo run --locked --example pack
```

The package example creates a WinIsland ZIP in `target`. Drop that ZIP onto the island while WinIsland is running to install or update it. For a quick local development build, the DLL is at `target\release\rotating_cd.dll`; close WinIsland before copying a root-level DLL into `%APPDATA%\WinIsland\plugins`, then restart WinIsland.

## Test the compact overlay

1. Build the ZIP with `cargo run --locked --example pack`.
2. While WinIsland is running, drop `target\Rotating CD-1.0.0.zip` onto the island to update/install the plugin. Check the Plugins settings page if needed to confirm it is enabled.
3. Play a track with album art. The native track title and visualizer should remain visible, with the album art at the left drawn as a small CD. Pause and resume to verify the disc stops and starts.
4. Expand the island. The CD overlay should disappear while expanded and return over the cover when collapsed.

The whole disc completes a full revolution in 2.5 seconds. WinIsland updates animated surfaces at roughly 60 frames per second, keeping the rotation smooth during playback.

## Release

This plugin is intended for the public repository [`phonkboisad/winisland-rotatingcover`](https://github.com/phonkboisad/winisland-rotatingcover). Push a version tag matching `Cargo.toml` (currently `v1.0.0`) to run `.github/workflows/release.yml`. The pinned official PluginMarketplace workflow checks, packages, attests, and publishes a `*.winisland-plugin.zip` asset to a GitHub Release. Never replace an asset for an existing version; increment `version` in `Cargo.toml` and publish a new tag.

The author and repository values in `Cargo.toml` are embedded in the package metadata and must remain consistent with the plugin descriptor and marketplace registration.

## Marketplace submission

After the source is public and a valid tagged release is available, use [`marketplace-registration.toml`](https://github.com/phonkboisad/winisland-rotatingcover/blob/main/marketplace-registration.toml) as the single file `plugins/rotating-cd.toml` in a pull request to [WinIsland PluginMarketplace](https://github.com/WinIslandProject/PluginMarketplace). The Marketplace contribution pull request must contain exactly that registration file; do not include plugin-source, workflow, or catalog changes in it. See the [contribution guide](https://github.com/WinIslandProject/PluginMarketplace/blob/main/CONTRIBUTING.md).
