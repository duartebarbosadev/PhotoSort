'use strict';
// What the intro films share: the frame, the palette, the drawn app window, the
// photos, the pointer and the camera. Each film (film.html, organize.html) loads
// this, then says what happens when.
const W = 1920, H = 1080, CX = 960, CY = 432;

// The drawn app window, and the area inside it between its title bar and step tabs.
const WX = 330, WY = 64, WW = 1260, WH = 736, TOP = WY + 46, FOOT = WY + WH - 58;
const TABS = ['Organize', 'Easy Delete', 'Fix Rotation', 'Pick Best', 'Cull', 'Review'];
const TAB = k => ({ x: CX + (k - 2.5) * 152, y: FOOT + 29 });

// The app's own colours (src/ui/theme.rs).
const BG = '#181818', PANEL = '#1F1E1D', CARD = '#2B2A28', LINE = '#3A3835';
const INK = '#E9E7E3', MUTED = '#A8A49E', FAINT = '#6E6A65';
const ACCENT = '#E8873A', ON_ACCENT = '#17120E', RED = '#E5554F', GREEN = '#5DBE7E';
const FONT = 'Inter, sans-serif';

// A "photo" is a sky, a sun and two hills in one of these moods.
const PALS = [
  { sky: ['#F8C47C', '#EE7B4D'], sun: '#FFF0CC', hills: ['#B9483B', '#7C2D33'] }, // sunset
  { sky: ['#9BE0E2', '#3F8FC4'], sun: '#F6FDFF', hills: ['#2C6A9E', '#1C4370'] }, // sea
  { sky: ['#DCEBAE', '#7DB877'], sun: '#FDF9DA', hills: ['#3F8558', '#255A43'] }, // forest
];

const canvas = document.getElementById('film');
const c = canvas.getContext('2d');
const icon = new Image();

// ---- maths ---------------------------------------------------------------
const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
const seg = (t, a, b) => clamp((t - a) / (b - a));
const lerp = (a, b, p) => a + (b - a) * p;
const eoc = t => 1 - Math.pow(1 - t, 3);
const eic = t => t * t * t;
const eio = t => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
function rng(seed) {
  let a = seed * 2654435761 >>> 0;
  return () => {
    a = a + 0x6D2B79F5 | 0;
    let t = Math.imul(a ^ a >>> 15, 1 | a);
    t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t;
    return ((t ^ t >>> 14) >>> 0) / 4294967296;
  };
}

// ---- drawing helpers -----------------------------------------------------
function rr(x, y, w, h, r) { c.beginPath(); c.roundRect(x, y, w, h, r); }
function font(size, weight = 400, spacing = 0) {
  c.font = `${weight} ${size}px ${FONT}`;
  c.letterSpacing = `${spacing}px`;
}
function text(s, x, y, color, align = 'left') {
  c.fillStyle = color; c.textAlign = align; c.textBaseline = 'alphabetic';
  c.fillText(s, x, y);
}
// Words that slide up into place behind a mask, one after another, and leave
// the same way. `enter(k)` and `leave` run 0..1.
function slideWords(str, cx, baseline, size, color, enter, leave = 0) {
  const words = str.split(/(?<= )/), full = c.measureText(str).width, x0 = cx - full / 2;
  c.save(); c.beginPath(); c.rect(x0 - 40, baseline - size * 0.98, full + 80, size * 1.3); c.clip();
  c.textAlign = 'left'; c.textBaseline = 'alphabetic'; c.fillStyle = color;
  words.forEach((word, k) => {
    const dy = (1 - eoc(enter(k))) * size * 1.35 - eic(leave) * size * 1.35;
    c.fillText(word, x0 + c.measureText(words.slice(0, k).join('')).width, baseline + dy);
  });
  c.restore();
}
// One photo. `focus` 1 is sharp and in front; 0 is far behind, soft and dim.
function photo(o) {
  const { x, y, w, rot = 0, scale = 1, focus = 1, dim = 0, pal = 0, seed = 1, soft = 0, sun = 0, rack = 0, alpha = 1 } = o;
  if (scale <= 0.004) return;
  const h = w * 0.75, P = PALS[pal], rnd = rng(seed), r = h * 0.11, far = 1 - focus * (1 - 0.72 * rack);
  c.save();
  c.translate(x, y); c.rotate(rot); c.scale(scale, scale); c.globalAlpha *= alpha * lerp(1, 0.5, far);
  if (far > 0.02) c.filter = `blur(${(26 * Math.pow(far, 1.3)).toFixed(2)}px)`;
  c.save();
  c.shadowColor = `rgba(0,0,0,${0.5 * (1 - far)})`; c.shadowBlur = h * 0.3 * scale; c.shadowOffsetY = h * 0.09 * scale;
  c.fillStyle = P.hills[1]; rr(-w / 2, -h / 2, w, h, r); c.fill();
  c.restore();
  rr(-w / 2, -h / 2, w, h, r); c.clip();
  if (soft > 0) c.filter = `blur(${(soft * w / 220 + 26 * Math.pow(far, 1.3)).toFixed(2)}px)`;
  const sky = c.createLinearGradient(0, -h / 2, 0, h / 2);
  sky.addColorStop(0, P.sky[0]); sky.addColorStop(1, P.sky[1]);
  c.fillStyle = sky; c.fillRect(-w, -h, 2 * w, 2 * h);
  const sx = -w / 2 + w * (0.26 + 0.48 * rnd() + sun), sy = -h / 2 + h * (0.26 + 0.1 * rnd());
  c.fillStyle = P.sun; c.beginPath(); c.arc(sx, sy, h * 0.115, 0, 7); c.fill();
  const hill = (base, amp, color) => {
    const p1 = rnd() * 7, p2 = rnd() * 7, f1 = 2 + rnd() * 2, f2 = 5 + rnd() * 3;
    c.fillStyle = color; c.beginPath(); c.moveTo(-w, h);
    for (let i = -4; i <= 28; i++) {
      const u = i / 24;
      c.lineTo(-w / 2 + u * w, -h / 2 + h * (base + amp * Math.sin(u * f1 + p1) + amp * 0.45 * Math.sin(u * f2 + p2)));
    }
    c.lineTo(w, h); c.fill();
  };
  hill(0.6, 0.09, P.hills[0]);
  hill(0.8, 0.055, P.hills[1]);
  const shade = Math.max(dim, 0.4 * far, 0.55 * rack);
  if (shade > 0) { c.fillStyle = `rgba(22,22,22,${shade})`; c.fillRect(-w, -h, 2 * w, 2 * h); }
  c.restore();
}
function check(x, y, s, color, lw = 0.2) {
  c.save(); c.translate(x, y); c.scale(s, s);
  c.strokeStyle = color; c.lineWidth = lw; c.lineCap = 'round'; c.lineJoin = 'round';
  c.beginPath(); c.moveTo(-0.42, 0.02); c.lineTo(-0.13, 0.3); c.lineTo(0.44, -0.3); c.stroke();
  c.restore();
}
function cross(x, y, s, color, lw = 0.2) {
  c.save(); c.translate(x, y); c.scale(s, s);
  c.strokeStyle = color; c.lineWidth = lw; c.lineCap = 'round';
  c.beginPath(); c.moveTo(-0.32, -0.32); c.lineTo(0.32, 0.32); c.moveTo(0.32, -0.32); c.lineTo(-0.32, 0.32); c.stroke();
  c.restore();
}
function star(x, y, r, color) {
  c.fillStyle = color; c.beginPath();
  for (let i = 0; i < 10; i++) {
    const a = -Math.PI / 2 + i * Math.PI / 5, q = i % 2 ? r * 0.46 : r;
    c.lineTo(x + Math.cos(a) * q, y + Math.sin(a) * q);
  }
  c.closePath(); c.lineJoin = 'round'; c.fill();
}
// Icons on a 24-unit grid, centred on (x, y).
function glyph(kind, x, y, s, color) {
  c.save(); c.translate(x, y); c.scale(s, s);
  c.strokeStyle = color; c.lineWidth = 2; c.lineCap = 'round'; c.lineJoin = 'round';
  c.beginPath();
  if (kind === 'folder') {
    c.moveTo(-10, -7); c.lineTo(-10, 8); c.lineTo(10, 8); c.lineTo(10, -4.5); c.lineTo(0, -4.5); c.lineTo(-3, -8); c.lineTo(-10, -8); c.closePath();
  } else if (kind === 'trash') {
    c.moveTo(-8, -6); c.lineTo(8, -6); c.moveTo(-3, -6); c.lineTo(-3, -9); c.lineTo(3, -9); c.lineTo(3, -6);
    c.moveTo(-6, -6); c.lineTo(-5, 9); c.lineTo(5, 9); c.lineTo(6, -6);
    c.moveTo(-2, -1.5); c.lineTo(-2, 4.5); c.moveTo(2, -1.5); c.lineTo(2, 4.5);
  } else if (kind === 'left' || kind === 'right') {
    const d = kind === 'left' ? 1 : -1;
    c.moveTo(3 * d, -6); c.lineTo(-3 * d, 0); c.lineTo(3 * d, 6);
  } else if (kind === 'rotate') {
    // Three quarters of a circle, ending in an arrowhead that points anticlockwise.
    c.arc(0, 0, 7.5, -Math.PI * 0.35, Math.PI * 1.1, false); c.stroke();
    const a = -Math.PI * 0.35, px = 7.5 * Math.cos(a), py = 7.5 * Math.sin(a);
    c.beginPath(); c.moveTo(px + 5.4 * Math.sin(a), py - 5.4 * Math.cos(a));
    c.lineTo(px + 4.3 * Math.cos(a), py + 4.3 * Math.sin(a)); c.lineTo(px - 4.3 * Math.cos(a), py - 4.3 * Math.sin(a)); c.closePath();
    c.fillStyle = color; c.fill();
  }
  c.stroke(); c.restore();
}
function badge(x, y, r, color, kind, grow = 1) {
  if (grow <= 0.004) return;
  c.save(); c.translate(x, y); c.scale(grow, grow);
  c.shadowColor = 'rgba(0,0,0,0.35)'; c.shadowBlur = 14; c.shadowOffsetY = 4;
  c.fillStyle = color; c.beginPath(); c.arc(0, 0, r, 0, 7); c.fill(); c.shadowColor = 'transparent';
  if (kind === 'check') check(0, 0, r * 1.25, ON_ACCENT);
  else if (kind === 'cross') cross(0, 0, r * 1.25, ON_ACCENT);
  else star(0, 1, r * 0.58, ON_ACCENT);
  c.restore();
}

// ---- where a photo is ---------------------------------------------------------
// Where a photo rests when nothing is using it: far behind, drifting round `home`.
function drift(home, k, t) {
  return { x: home.x + Math.sin(t * 0.31 + k) * 26, y: home.y + Math.cos(t * 0.27 + 2 * k) * 18, w: home.w, rot: home.rot + Math.sin(t * 0.2 + k) * 0.04, focus: 0, scale: 1, dim: 0, z: -1 };
}
// Below RESTING a photo is behind the window; by CROSSED it is fully in front.
const RESTING = 0.03, CROSSED = 0.7;
const front = (x, y, w, more = {}) => ({ x, y, w, rot: 0, focus: 1, scale: 1, dim: 0, z: 0, ...more });
function mix(a, b, p) {
  const out = {};
  for (const key of ['x', 'y', 'w', 'rot', 'focus', 'scale', 'dim']) out[key] = lerp(a[key], b[key], p);
  out.z = p < 0.5 ? a.z : b.z;
  return out;
}

// Runs `body` in a photo's own space: centred on it, turned and scaled with it.
function onPhoto(pose, body) {
  if (pose.scale <= 0.004) return;
  c.save(); c.translate(pose.x, pose.y); c.rotate(pose.rot); c.scale(pose.scale, pose.scale); body(pose.w, pose.w * 0.75); c.restore();
}

// ---- backdrop -----------------------------------------------------------------
let noise;
function makeNoise() {
  noise = document.createElement('canvas'); noise.width = W; noise.height = H;
  const n = noise.getContext('2d'), img = n.createImageData(W, H), rnd = rng(7);
  for (let i = 0; i < img.data.length; i += 4) {
    const v = rnd() * 255 | 0;
    img.data[i] = img.data[i + 1] = img.data[i + 2] = v; img.data[i + 3] = 7;
  }
  n.putImageData(img, 0, 0);
}
function backdrop(t) {
  c.fillStyle = BG; c.fillRect(0, 0, W, H);
  const gx = CX + Math.sin(t * 0.23) * 90, gy = CY + 40 + Math.cos(t * 0.19) * 50;
  const glow = c.createRadialGradient(gx, gy, 0, gx, gy, 1000);
  glow.addColorStop(0, 'rgba(232,135,58,0.15)'); glow.addColorStop(0.45, 'rgba(232,135,58,0.05)'); glow.addColorStop(1, 'rgba(232,135,58,0)');
  c.fillStyle = glow; c.fillRect(0, 0, W, H);
}
function vignette() {
  const edge = c.createRadialGradient(CX, 540, 520, CX, 540, 1250);
  edge.addColorStop(0, 'rgba(0,0,0,0)'); edge.addColorStop(1, 'rgba(0,0,0,0.5)');
  c.fillStyle = edge; c.fillRect(0, 0, W, H);
}

// ---- the window -------------------------------------------------------------
const wire = a => `rgba(233,231,227,${a})`;
// A rounded outline drawn on from its top-left corner: `p` 0 is none of it, 1 all.
function trace(x, y, w, h, r, p, color = wire(0.3), lw = 2) {
  if (p <= 0.004) return;
  const around = 2 * (w + h);
  c.strokeStyle = color; c.lineWidth = lw; c.lineCap = 'round';
  if (p < 1) c.setLineDash([around * p, around]);
  rr(x, y, w, h, r); c.stroke(); c.setLineDash([]);
}
// The window: its outline, traced on as `drawn` goes 0 to 1, and inside it, as
// `fit` does, the title bar's line and the step tabs. `pos` is the lit tab (null
// for none) and `press` how far it is pressed; `rise(k)` is 0 once tab k's name
// is in place.
function appFrame(drawn, fit, pos, press, rise) {
  if (drawn <= 0.004) return;
  if (fit > 0) {
    c.save(); rr(WX, WY, WW, WH, 22); c.clip();
    c.fillStyle = `rgba(22,21,20,${0.8 * fit})`; c.fillRect(WX, WY, WW, WH);
    // The title bar and the step tabs open out from the middle.
    c.fillStyle = wire(0.14); c.fillRect(CX - WW / 2 * fit, TOP, WW * fit, 2); c.fillRect(CX - WW / 2 * fit, FOOT, WW * fit, 2);
    if (pos !== null) {
      const x = TAB(pos).x;
      c.fillStyle = wire(0.09 + 0.1 * press); rr(x - 68, FOOT + 9, 136, 40, 12); c.fill();
      c.fillStyle = ACCENT; rr(x - 18, FOOT + 44, 36, 4, 2); c.fill();
    }
    TABS.forEach((name, k) => {
      font(20, 500, -0.2); text(name, TAB(k).x, FOOT + 36 + 60 * rise(k), pos !== null && Math.abs(pos - k) < 0.5 ? INK : wire(0.42), 'center');
    });
    c.restore();
  }
  // The outline runs both ways round from the top-left corner and closes at the far one.
  const around = 2 * (WW + WH), half = around * drawn / 2;
  c.strokeStyle = wire(0.45); c.lineWidth = 2.5; c.lineCap = 'round';
  rr(WX, WY, WW, WH, 22);
  if (drawn < 1) {
    c.setLineDash([half, around - half]); c.stroke();
    c.lineDashOffset = half - around; c.stroke();
    // A bright tip leads each end while it is still drawing.
    const tip = Math.min(70, half), glow = Math.sin(Math.PI * drawn);
    c.strokeStyle = `rgba(232,135,58,${glow})`; c.lineWidth = 4;
    c.setLineDash([tip, around - tip]); c.lineDashOffset = tip - half; c.stroke();
    c.lineDashOffset = half - around; c.stroke();
    c.setLineDash([]); c.lineDashOffset = 0;
  } else c.stroke();
}
// The icon and name: large in the middle (`fly` 0) or in the window's title bar (1).
function wordmark(fly, grow, enter, y = 312) {
  const big = 120;
  font(big, 600, -4);
  const width = c.measureText('PhotoSort').width, size = lerp(170, 26, fly) * grow;
  if (size > 1) c.drawImage(icon, lerp(CX, WX + 30, fly) - size / 2, lerp(y, WY + 23, fly) - size / 2, size, size);
  c.save();
  c.translate(lerp(CX - 2, WX + 52 + width * 19 / big / 2, fly), lerp(y + 196, WY + 30, fly)); c.scale(lerp(1, 19 / big, fly), lerp(1, 19 / big, fly));
  font(big, 600, -4 * (1 - fly));
  slideWords('PhotoSort', 0, 0, big, INK, () => enter);
  c.restore();
}
function chip(label, x, y, color, grow, outline = false) {
  if (grow <= 0.004) return;
  font(20, 600, 0);
  const w = c.measureText(label).width + 32;
  c.save(); c.translate(x - w / 2, y); c.scale(grow, grow);
  rr(-w / 2, -19, w, 38, 19);
  if (outline) { c.fillStyle = '#1B1A19'; c.fill(); c.strokeStyle = color; c.lineWidth = 2; c.stroke(); } else { c.fillStyle = color; c.fill(); }
  text(label, 0, 7, outline ? color : ON_ACCENT, 'center');
  c.restore();
}
const pressAt = (s, at) => Math.sin(Math.PI * seg(s, at, at + 0.4));

// ---- the pointer ---------------------------------------------------------------
// `stops` say where it goes and what it presses; it sets off before each stop to
// arrive on time (`travel` seconds before, 0.6 unless the stop says). A stop may
// `click`, and may `hold` the button down until the next one.
function drawPointer(stops, t, show) {
  const next = stops.findIndex(stop => stop.t > t), to = stops[next] || stops[stops.length - 1], from = stops[Math.max(0, (next < 0 ? stops.length : next) - 1)];
  const p = eio(seg(t, Math.max(from.t + 0.12, to.t - (to.travel || 0.6)), to.t - 0.03));
  const x = lerp(from.x, to.x, p), y = lerp(from.y, to.y, p);
  if (show <= 0.004) return;
  stops.forEach(stop => {
    const ring = seg(t, stop.t, stop.t + 0.5);
    if (!stop.click || ring <= 0 || ring >= 1) return;
    c.strokeStyle = `rgba(232,135,58,${1 - ring})`; c.lineWidth = 4 * (1 - ring) + 1; c.beginPath(); c.arc(stop.x, stop.y - 4, 12 + 44 * eoc(ring), 0, 7); c.stroke();
  });
  const press = Math.max(from.hold && t >= from.t ? 0.8 : 0, ...stops.map(stop => (stop.click ? Math.sin(Math.PI * seg(t, stop.t - 0.08, stop.t + 0.22)) : 0)));
  c.save(); c.translate(x, y); c.scale(show * (1.25 - 0.2 * press), show * (1.25 - 0.2 * press));
  c.shadowColor = 'rgba(0,0,0,0.5)'; c.shadowBlur = 12; c.shadowOffsetY = 4;
  c.beginPath(); c.moveTo(0, 0); c.lineTo(0, 26); c.lineTo(7, 20); c.lineTo(12, 31); c.lineTo(16.5, 29); c.lineTo(11.5, 18.5); c.lineTo(20, 18.5); c.closePath();
  c.fillStyle = '#F4F1EC'; c.fill(); c.shadowColor = 'transparent';
  c.strokeStyle = '#141414'; c.lineWidth = 2.2; c.lineJoin = 'round'; c.stroke();
  c.restore();
}

// ---- the camera ----------------------------------------------------------------
// The view at `t`. The window fills the frame; a slow push and drift keeps every
// frame moving, what is far moves more, and the view leans in on `lean` (a point
// in the window and an `amount`). `window()` sets the view for what is drawn on
// the window; `photo(p)` draws one photo, whose distance follows its focus so
// that it glides between the far layer and the window.
function viewAt(t, lean) {
  const zoom = 1.13 * (1 + 0.012 * Math.sin(t * 0.33 - 1.2)), dx = 12 * Math.sin(t * 0.29), dy = 7 * Math.cos(t * 0.24);
  const camera = (depth, scale, midY) => {
    const tx = CX + (lean.x - CX) * 1.13, ty = 540 + (lean.y - CY) * 1.13, push = 1 + 0.17 * lean.amount / depth;
    c.translate(tx, ty); c.scale(push, push); c.translate(-tx, -ty);
    c.translate(CX + dx * depth, 540 + dy * depth); c.scale(scale, scale); c.translate(-CX, -midY);
  };
  return {
    window: () => camera(1, zoom, CY),
    photo: (p, more, paint = photo) => { c.save(); camera(lerp(2.4, 1, p.focus), lerp(zoom / 1.1, zoom, p.focus), lerp(540, CY, p.focus)); paint({ ...p, ...more }); c.restore(); },
  };
}

// ---- what the films of the steps share ----------------------------------------------
// Where the `n`th of a step film's photos drifts when it is far behind the window.
function farHome(n) {
  const rnd = rng(300 + n), b = lerp(-3.55, 0.4, (n * 8 % 21) / 20), reach = n % 2 ? 1.12 : 0.94;
  return { x: CX + Math.cos(b) * 860 * reach, y: 520 + Math.sin(b) * 470 * reach, w: 280 + rnd() * 120, rot: (rnd() - 0.5) * 0.5 };
}
// Words that slide up into place like slideWords, from a left edge.
function slideLeft(str, x, baseline, size, color, enter, leave = 0) {
  c.save(); c.translate(x + c.measureText(str).width / 2, 0); slideWords(str, 0, baseline, size, color, () => enter, leave); c.restore();
}
// The same, up to a right edge.
function slideRight(str, x, baseline, size, color, enter, leave = 0) {
  slideLeft(str, x - c.measureText(str).width, baseline, size, color, enter, leave);
}
// More icons on the kit's 24-unit grid.
function symbol(kind, x, y, s, color) {
  c.save(); c.translate(x, y); c.scale(s, s);
  c.strokeStyle = color; c.fillStyle = color; c.lineWidth = 2; c.lineCap = 'round'; c.lineJoin = 'round';
  c.beginPath();
  if (kind === 'drive') {
    c.roundRect(-9, -7, 18, 6, 2); c.roundRect(-9, 1, 18, 6, 2); c.stroke();
    c.beginPath(); c.arc(-5, -4, 1.2, 0, 7); c.arc(-5, 4, 1.2, 0, 7); c.fill();
  } else if (kind === 'caret') {
    c.moveTo(-5, -2.5); c.lineTo(0, 2.5); c.lineTo(5, -2.5); c.stroke();
  } else if (kind === 'search') {
    c.arc(-1.5, -1.5, 6, 0, 7); c.moveTo(3, 3); c.lineTo(8, 8); c.stroke();
  } else if (kind === 'plus') {
    c.moveTo(-5, 0); c.lineTo(5, 0); c.moveTo(0, -5); c.lineTo(0, 5); c.stroke();
  } else if (kind === 'check') {
    c.moveTo(-5, 0); c.lineTo(-1.5, 3.5); c.lineTo(5, -4); c.stroke();
  }
  c.restore();
}
// What is said, over the middle of the window, before each thing is shown.
// `cards` lists when, for how long, and the two lines; the first is the step's name.
function stepCards(cards, t) {
  cards.forEach(([at, length, big, small], n) => {
    const s = t - at, leave = seg(s, length - 0.5, length - 0.15), size = n ? 64 : 88;
    if (s < 0 || s > length) return;
    font(size, 600, n ? -2 : -3);
    slideWords(big, CX, CY + 4, size, INK, k => seg(s, 0.3 + 0.1 * k, 0.75 + 0.1 * k), leave);
    font(31, 400, -0.1);
    slideWords(small, CX, CY + 62, 31, MUTED, k => seg(s, 0.5 + 0.05 * k, 0.95 + 0.05 * k), leave);
  });
}
// How far the window's contents have slipped out of focus behind a card.
const rackOf = (cards, t) => Math.max(...cards.map(([at, length]) => eoc(seg(t - at, 0, 0.4)) * (1 - eio(seg(t - at, length - 0.5, length)))));
// The push in on what is being pressed. `leans` say from when (`t`) to when
// (`until`), where, and how far (`amount`).
function leanOf(leans, t) {
  let best = { amount: 0, x: CX, y: CY };
  leans.forEach(lean => {
    const amount = lean.amount * eio(seg(t, lean.t - 0.45, lean.t)) * (1 - eio(seg(t, (lean.until || lean.t) + 0.3, (lean.until || lean.t) + 0.85)));
    if (amount > best.amount) best = { ...lean, amount };
  });
  return best;
}
// The window of a step's film: drawn on in its first second with tab `step`
// lit, Review's tab pressed at `tabAt`, and drawn away from `end`.
function stepFrame(t, step, tabAt, end) {
  const e = t - end;
  appFrame(eio(seg(t, 0.1, 1.0)) * (1 - eio(seg(e, 0.3, 1.1))), eoc(seg(t, 0.6, 1.2)) * (1 - eic(seg(e, 0.1, 0.6))),
    lerp(step, 5, eio(seg(t, tabAt, tabAt + 0.4))), Math.sin(Math.PI * seg(t, tabAt, tabAt + 0.5)),
    k => 1 - eoc(seg(t, 0.75 + 0.07 * k, 1.2 + 0.07 * k)) + eic(seg(e, 0.04 * k, 0.4 + 0.04 * k)));
  // The title bar: the icon and name, and the menu at its far end.
  const g = eoc(seg(t, 0.95, 1.3)) * (1 - eic(seg(e, 0, 0.3)));
  if (g > 0.04) c.drawImage(icon, WX + 30 - 13 * g, WY + 23 - 13 * g, 26 * g, 26 * g);
  font(19, 600, 0); slideLeft('PhotoSort', WX + 52, WY + 30, 19, INK, seg(t, 0.95, 1.4), seg(e, 0, 0.3));
  if (g > 0.04) {
    c.save(); c.translate(WX + WW - 52, WY + 23); c.scale(g, g);
    c.fillStyle = wire(0.09); rr(-36, -14, 72, 28, 9); c.fill();
    font(16, 500, 0); text('Menu', 0, 6, wire(0.8), 'center');
    c.restore();
  }
}
// Review, from `tabAt` until `end`: one kind of change (its icon, what it is
// called and how many), what it is made of (`lines` of name, count, colour and,
// when it is not the kind's, the line's own icon),
// and Apply, waiting to be pressed.
const APPLY_Y = TOP + 130 + 96 + 150 + 60;
function reviewList(t, tabAt, end, kind, label, total, lines) {
  const s = t - tabAt, out = eic(seg(t, end, end + 0.4)), into = eoc(seg(s, 0.7, 1.2));
  if (into <= 0.004 || out >= 1) return;
  const x = CX - 360, w = 720, y = TOP + 130;
  // The row rises into its own slot and later sinks out of it.
  c.save(); c.beginPath(); c.rect(x - 30, y - 56, w + 60, 112); c.clip(); c.translate(0, 118 * (1 - into) + 118 * out);
  c.fillStyle = PANEL; c.strokeStyle = LINE; c.lineWidth = 2; rr(x, y - 44, w, 88, 22); c.fill(); c.stroke();
  c.fillStyle = CARD; c.beginPath(); c.arc(x + 54, y, 27, 0, 7); c.fill();
  glyph(kind, x + 54, y, 1.3, INK);
  font(30, 500, -0.3); text(label, x + 102, y + 10, INK);
  font(30, 600, 0); text(String(Math.round(total * eoc(seg(s, 0.9, 1.5)))), x + w - 40, y + 10, INK, 'right');
  c.restore();
  lines.forEach(([name, count, color, own], i) => {
    const yy = y + 96 + 50 * i, enter = seg(s, 1.15 + 0.12 * i, 1.55 + 0.12 * i), g = eoc(enter) * (1 - out);
    if (g > 0.01 && own === 'star') star(x + 54, yy, 11 * g, color);
    else if (g > 0.01) glyph(own || kind, x + 54, yy, g, color);
    font(26, 500, -0.2); slideLeft(name, x + 102, yy + 9, 26, INK, enter, out);
    slideRight(String(count), x + w - 40, yy + 9, 26, MUTED, enter, out);
  });
  const g = eoc(seg(s, 1.7, 2.1)) * (1 - out);
  if (g <= 0.004) return;
  c.save(); c.translate(CX, APPLY_Y); c.scale(g, g);
  c.shadowColor = `rgba(232,135,58,${0.3 + 0.2 * Math.sin(t * 3)})`; c.shadowBlur = 40;
  c.fillStyle = ACCENT; rr(-120, -38, 240, 76, 38); c.fill(); c.shadowColor = 'transparent';
  font(30, 600, -0.2); text('Apply', 0, 10.5, ON_ACCENT, 'center');
  c.restore();
}
// Paints a step film's frame at `t`: the backdrop, the photos that are far, the
// window with what is `under` the photos, the photos in focus (inside `clip`, a
// rectangle of the window, when there is one), what is `over` them, and on `top`
// what never steps back: the cards and the pointer. `rack` is how far the
// window's contents have stepped back behind a card.
function compose(t, { poses, lean, rack, frame, under, over, top, clip }) {
  c.setTransform(1, 0, 0, 1, 0, 0); c.globalAlpha = 1; c.filter = 'none';
  backdrop(t);
  const view = viewAt(t, lean);
  poses.sort((a, b) => a.focus - b.focus || a.z - b.z);
  // A photo stepping back is in front of the window and, less and less, behind
  // it too, so it never pops across the window's edge.
  poses.filter(p => p.focus < CROSSED).forEach(p => view.photo(p));
  vignette();
  c.save(); view.window();
  frame(t);
  c.globalAlpha = 1 - 0.8 * rack;
  under(t);
  c.globalAlpha = 1;
  c.restore();
  c.save();
  if (clip) { c.save(); view.window(); c.beginPath(); c.rect(...clip); c.restore(); c.clip(); }
  poses.filter(p => p.focus > RESTING).forEach(p => view.photo(p, { rack, alpha: eio(seg(p.focus, RESTING, CROSSED)) }, p.paint));
  c.restore();
  c.save(); view.window();
  c.globalAlpha = 1 - 0.8 * rack;
  over(t);
  c.globalAlpha = 1;
  top(t);
  c.restore();
  c.drawImage(noise, 0, 0);
}

// Loads the icon and the fonts; a film waits for this before its first frame.
async function loadKit() {
  icon.src = '../../resources/icons/app_icon.png';
  const faces = [[400, 'Regular'], [500, 'Medium'], [600, 'SemiBold']].map(([weight, name]) =>
    new FontFace('Inter', `url(../../resources/fonts/Inter-${name}.ttf)`, { weight: String(weight) }).load());
  (await Promise.all(faces)).forEach(face => document.fonts.add(face));
  await icon.decode();
  makeNoise();
}
