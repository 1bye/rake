# Native file icons

Implementation in progress for #66. Native images must be rasterized at the
requested logical size multiplied by the current window scale. Never retain
`TIFFRepresentation` or full-resolution multi-representation icons in file rows.

The cache identity must distinguish plain folders, file types, bundle identifiers,
and custom per-path icons. Raster dimensions belong in the key so changing display
scale requests a fresh raster. Bitmap storage has a 4 MiB default budget with
least-recently-used eviction. Renderer allocations and externally retained handles
must be measured separately from this storage budget.

The current implementation exports `rake::Icon` and
`rake::platform::icons::for_path(path, size, window, cx)`. The handle converts to
GPUI's image source and delegates loading to a shared `ImageCache`. It reserves no
intrinsic layout size while loading: use an explicit image size. The cache is
installed during Rake application startup, before rendering.

macOS uses NSWorkspace and an exact-size bitmap graphics context on blocking
workers. The key includes the physical pixel size; scale changes request another
raster. File identities use content-type identifiers, app identities use bundle
identifiers, and Finder custom icons and symlinks retain path identity. Both raw
bitmap storage and rendered storage currently have independent 4 MiB budgets.
Path metadata is bounded to 2,048 entries and pending loads to sixteen.

Windows and Linux currently use generic fallback bitmaps; native shell and icon
theme loading remain outstanding. Browser paths use generic folder/file bitmaps
without filesystem access. Individual raster sizes are limited to 256 physical
pixels. Inspection metrics, native platform validation, the large-directory
example, and memory measurements remain outstanding.
