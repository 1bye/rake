# Native file icons

Implementation in progress for #66. Native images must be rasterized at the
requested logical size multiplied by the current window scale. Never retain
`TIFFRepresentation` or full-resolution multi-representation icons in file rows.

The cache identity must distinguish plain folders, file types, bundle identifiers,
and custom per-path icons. Raster dimensions belong in the key so changing display
scale requests a fresh raster. Bitmap storage has a 4 MiB default budget with
least-recently-used eviction. Renderer allocations and externally retained handles
must be measured separately from this storage budget.
