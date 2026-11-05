# GPUI Base 0.9.0

This is the Apache-2.0 crates.io release from GPUI Kit revision
`7591a35eced761e3613c01e71ff04c10129dae1a`.

The local changes are in `src/input/base`. Notes opts into grapheme
navigation and deletion through Base's existing multicaret actions. A rejected
final IME edit rolls back its pending composition. Notes also opts into a
body byte limit checked before edits and retains spaces between title lines.
Remove the Cargo patch once the pinned upstream release supports these behaviors.
