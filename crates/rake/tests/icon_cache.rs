//! Covers: icon raster sharing, byte limits, LRU ordering, and scale keys.
//! Real: the cache implementation, without native icon lookup or a renderer.
//! Doubles: synthetic bitmap bytes and visual identities.
//! Requires: none.

#[path = "../src/platform/icon_cache.rs"]
mod icon_cache;
