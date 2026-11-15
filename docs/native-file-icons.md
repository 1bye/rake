# Native file icons

Use the shared service when displaying paths in a Rake application:

```rust,ignore
let icon = rake::platform::icons::for_path(&path, rake::gpui::px(22.), window, cx);
rake::view! { <img src={icon} alt="" class="size-[22px]" /> }
```

`rake::Icon` is a cheap-clone path-and-cache handle. Use empty alt text when the
adjacent filename already describes the item; otherwise supply a useful label.
Give the image an explicit layout size. A generic placeholder appears until the
background lookup finishes. Rake initializes the shared GPUI `ImageCache` before
opening its windows; views retain handles rather than full bitmap allocations.

The raster key combines visual identity and physical pixel size. A 22-point icon
on a 2× display is a 44×44 bitmap. Moving between displays requests a new size.
Sizes are clamped to 1–256 physical pixels. Plain folders share a bitmap, file
icons share by type, app icons share by bundle identifier on macOS, and custom
icons use path identity. Special system folders and symlinks use separate macOS
identities to preserve their distinct appearance.

Never retain `TIFFRepresentation` or full-resolution multi-representation native
icons in file rows. One native icon can contain representations far larger than
its displayed size. The macOS backend draws NSWorkspace images into an exact-size
bitmap context on blocking workers and releases the original representation.

Rendered storage has a 4 MiB LRU budget, excluding renderer/GPU allocations and
a shared 1 KiB loading placeholder. macOS raw raster storage has a separate 4 MiB
LRU budget. Each raster cache also caps entries at 512 so tiny icons cannot
accumulate unbounded key metadata. Path metadata is bounded to 2,048 entries, with at most sixteen pending
loads. `rake::platform::icons::inspect(cx)` exposes rendered entries, pixel bytes,
hits, misses, and pending loads. Filesystem/theme changes become visible after
cache eviction; this service does not watch files or desktop theme settings.

Windows uses SHGetFileInfo and draws its shell icon into a display-size DIB,
releasing icon, DC and bitmap handles after copying the pixels. Linux queries
MIME types with the installed `xdg-mime` utility, then searches PNG MIME/place
icons under XDG icon directories in the configured GTK/KDE/desktop icon theme, Adwaita and hicolor.
Linux SVG-only themes, unavailable MIME tools and native lookup failures use
built-in generic folder/file bitmaps. Full desktop-theme inheritance and custom
Linux icon metadata are not supported. The browser uses generic folder/file
bitmaps without native filesystem access. These fallbacks have the same API.

Run the virtualized large-directory example with:

```sh
cargo run -p rake-file-icons -- /path/to/large/directory
```

The header reports the retained rendered bitmap count and bytes. Capture process
footprint before scrolling and again after scrolling through the listing; GPU,
font, window and runtime memory must be measured separately from cached pixels.
The native macOS regression creates 10,000 real folders and checks that the raw
cache contains one 44×44 raster (7,744 bytes) with 9,999 hits. Eviction tests check
that unrelated identities cannot exceed a fixed byte budget.

## Measured macOS run

On this Mac, the native example listed 10,000 newly created plain folders on a
2× display. The `footprint -p PID` report measured **103 MB** before scrolling and
**126 MB** after scrolling/jumping to approximately row 4,880 (peak **135 MB**).
The visible cache counter remained **one bitmap, 7,744 bytes** throughout. This
is a whole-process measurement, including a large window's graphics surfaces,
fonts and runtime allocations; it does not establish a 4 MiB process footprint.
Only visible rows are rasterized by the virtualized list. The separate native
regression exercises all 10,000 actual paths through the raw raster cache.
