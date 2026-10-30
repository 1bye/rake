// Session-local notes contain captured Rust context, never native entities or video.
export const MAX_ANNOTATIONS = 32;
export const MAX_COMMENT = 2000;

export function contextFor(element) {
  const {id, stable_id, kind, semantic_id, source, view, text, layout, content_mask, metadata_truncated} = element;
  return {id, stable_id, kind, semantic_id, source, view, text, layout, content_mask, metadata_truncated};
}

export function targetKey(element, generation) {
  return element.stable_id || `${generation}:${element.id}`;
}

export function visibleBounds(element, window) {
  const bounds = element?.layout, mask = element?.content_mask;
  if (!bounds || !mask || !window) return null;
  const x = Math.max(0, bounds.x, mask.x), y = Math.max(0, bounds.y, mask.y);
  const right = Math.min(window.content.width, bounds.x + bounds.width, mask.x + mask.width);
  const bottom = Math.min(window.content.height, bounds.y + bounds.height, mask.y + mask.height);
  return right > x && bottom > y ? {x, y, width:right - x, height:bottom - y} : null;
}

export class Annotations {
  constructor(storage) { this.storage = storage; this.items = []; this.key = null; }
  activate(runtime, window) {
    const key = `rake-preview-annotations:${runtime}:${window}`;
    if (key === this.key) return false;
    this.key = key; this.runtime = runtime; this.window = window; this.items = [];
    try {
      const saved = JSON.parse(this.storage.getItem(key) || '[]');
      if (Array.isArray(saved) && saved.length <= MAX_ANNOTATIONS) {
        this.items = saved.filter(note => Number.isSafeInteger(note?.number) && note.number > 0
          && typeof note.id === 'string' && note.id.length <= 128
          && typeof note.comment === 'string' && note.comment.length <= MAX_COMMENT
          && typeof note.generation === 'string' && note.generation.length <= 128
          && typeof note.element?.id === 'string' && typeof note.element?.kind === 'string'
          && JSON.stringify(note.element).length <= 65536)
          .map(note => ({id:note.id, number:note.number, comment:note.comment,
            generation:note.generation, element:contextFor(note.element)}));
        if (new Set(this.items.map(note => note.id)).size !== this.items.length) this.items = [];
      }
    } catch { /* Storage may be unavailable; notes still work in memory. */ }
    return true;
  }
  save(comment, element, generation, editing) {
    comment = comment.trim();
    if (!comment || comment.length > MAX_COMMENT) throw new Error(`Write a note of up to ${MAX_COMMENT} characters.`);
    if (!this.key) throw new Error('Wait for the native application to connect.');
    const index = this.items.findIndex(note => note.id === editing);
    if (index < 0 && this.items.length >= MAX_ANNOTATIONS) throw new Error(`This window supports up to ${MAX_ANNOTATIONS} annotations.`);
    const note = index < 0 ? {id:crypto.randomUUID(), number:Math.max(0, ...this.items.map(note => note.number)) + 1} : this.items[index];
    const next = {...note, comment, element:contextFor(element), generation};
    if (JSON.stringify(next.element).length > 65536) throw new Error('Element context is too large to annotate.');
    if (index < 0) this.items.push(next); else this.items[index] = next;
    this.persist();
    return next;
  }
  remove(id) { this.items = this.items.filter(note => note.id !== id); this.persist(); }
  persist() {
    this.warning = null;
    try { this.storage.setItem(this.key, JSON.stringify(this.items)); }
    catch { this.warning = 'Notes are kept in memory; session storage is unavailable. Copy them before closing.'; }
  }
  export() {
    return {runtime_id:this.runtime, window:this.window, annotations:this.items};
  }
}
