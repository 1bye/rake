// Rake framing: u32 metadata length, u32 AVCC/JPEG length, UTF-8 JSON, binary payload.
// Geometric buffer growth avoids repeatedly concatenating a partially received frame.
export class Packets {
  constructor() { this.buffer = new Uint8Array(65536); this.start = 0; this.end = 0; }
  push(chunk) {
    if (this.end - this.start + chunk.length > 4 * 1024 * 1024) throw new Error('Video buffer exceeded its bound');
    if (this.end + chunk.length > this.buffer.length) {
      const size = this.end - this.start + chunk.length;
      if (size <= this.buffer.length) this.buffer.copyWithin(0, this.start, this.end);
      else {
        const next = new Uint8Array(Math.min(4 * 1024 * 1024, Math.max(size, this.buffer.length * 2)));
        next.set(this.buffer.subarray(this.start, this.end)); this.buffer = next;
      }
      this.end -= this.start; this.start = 0;
    }
    this.buffer.set(chunk, this.end); this.end += chunk.length;
    const result = [];
    while (this.end - this.start >= 8) {
      const view = new DataView(this.buffer.buffer, this.start, 8);
      const h = view.getUint32(0), n = view.getUint32(4);
      if (h < 1 || h > 4096 || n < 1 || n > 2 * 1024 * 1024) throw new Error('Invalid video packet size');
      if (this.end - this.start < 8 + h + n) break;
      const metadata = JSON.parse(new TextDecoder().decode(this.buffer.subarray(this.start + 8, this.start + 8 + h)));
      const payload = this.buffer.slice(this.start + 8 + h, this.start + 8 + h + n);
      result.push({metadata, payload}); this.start += 8 + h + n;
    }
    if (this.start === this.end) this.start = this.end = 0;
    return result;
  }
}

export function position(event, rect, window) {
  const x = (event.clientX - rect.left) / rect.width * window.frame.width - window.content.x;
  const y = (event.clientY - rect.top) / rect.height * window.frame.height - window.content.y;
  return x >= 0 && y >= 0 && x < window.content.width && y < window.content.height ? {x, y} : null;
}

// Preserve subpixel trackpad movement while the runtime protocol stays integer/Eq.
export class Wheel {
  constructor() { this.x = 0; this.y = 0; this.active = false; }
  add(x, y) { this.x += x; this.y += y; }
  take(end = false) {
    const dx = Math.max(-128000, Math.min(128000, Math.trunc(this.x * 64)));
    const dy = Math.max(-128000, Math.min(128000, Math.trunc(this.y * 64)));
    this.x -= dx / 64; this.y -= dy / 64;
    if (!dx && !dy && (!end || !this.active)) return null;
    const phase = dx || dy ? this.active ? 'moved' : 'started' : 'ended';
    this.active = phase !== 'ended';
    return {type:'wheel', dx, dy, phase};
  }
}
