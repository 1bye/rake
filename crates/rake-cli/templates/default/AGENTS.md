<!-- rake-lint:begin -->
## Rake agent rules

After editing, run `rake format` then `rake lint`.
Use `rake lint --fix` for machine-safe fixes.

- Always prefer exact class utilities to equivalent pure style closures.
- Never hard-code colors, typography sizes or radii in themed views; use theme values.
- Never enable debug assertions in release profiles; use a separate diagnostics profile.
- Always give icon-only or dynamic buttons an aria-label, preserving any visible words.
- Always supply img alt text; use alt="" only for decorative images.
- Never derive loop IDs from positions; use stable data keys.
- Always explain suppressions; use allow-file only for a whole-file decision.

Full guide: `.agents/skills/rake-gpui/SKILL.md` (`rake lint --guide`).
<!-- rake-lint:end -->
