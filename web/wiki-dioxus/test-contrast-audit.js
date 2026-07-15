// Contrast audit — run via WebDriver execute/sync. Walks every visible element
// that renders its own text (or an icon), resolves the EFFECTIVE background
// (climbing ancestors, compositing semi-transparent layers over the page base),
// and computes the real WCAG contrast ratio. Returns a JSON array of elements
// below AA (4.5:1 normal text, 3:1 large text / icons) so the harness can fail.
//
// Blind spots (skipped, not failed): text over a background-image / gradient
// (no single colour to measure), and a glyph-overlay letter — text absolutely
// positioned on top of a sibling icon glyph (e.g. a folder avatar's initial sits
// on the white folder glyph, not the tinted circle behind it), which the flat
// background walk likewise cannot measure.

function parseColor(s) {
    var m = (s || '').match(/[0-9.]+/g) || [];
    return [+m[0] || 0, +m[1] || 0, +m[2] || 0, m[3] === undefined ? 1 : +m[3]];
}
function lin(v) {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}
function luminance(c) {
    return 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
}
// Composite foreground `f` (with alpha) over opaque `b`.
function over(f, b) {
    var a = f[3];
    return [f[0] * a + b[0] * (1 - a), f[1] * a + b[1] * (1 - a), f[2] * a + b[2] * (1 - a), 1];
}
function ratio(a, b) {
    var x = luminance(a), y = luminance(b), hi = Math.max(x, y), lo = Math.min(x, y);
    return (hi + 0.05) / (lo + 0.05);
}
// Effective background behind `el`: climb parents collecting background-colours
// until one is opaque, then composite them over the document base. null if any
// ancestor paints a background-image (unmeasurable).
function effectiveBg(el) {
    var base = parseColor(getComputedStyle(document.documentElement).backgroundColor);
    if (base[3] < 1) base = [255, 255, 255, 1];
    var stack = [], e = el;
    while (e) {
        var cs = getComputedStyle(e);
        if (cs.backgroundImage && cs.backgroundImage !== 'none') return null;
        var bg = parseColor(cs.backgroundColor);
        if (bg[3] > 0) stack.push(bg);
        if (bg[3] >= 1) break;
        e = e.parentElement;
    }
    var acc = base;
    for (var i = stack.length - 1; i >= 0; i--) acc = over(stack[i], acc);
    return acc;
}
function hasOwnText(el) {
    for (var i = 0; i < el.childNodes.length; i++) {
        var n = el.childNodes[i];
        if (n.nodeType === 3 && n.textContent.trim()) return true;
    }
    return el.classList.contains('material-icons');
}
function visible(el) {
    var cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || +cs.opacity === 0) return false;
    var r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
}
function selector(el) {
    var s = el.tagName.toLowerCase();
    if (el.className && typeof el.className === 'string') {
        var c = el.className.trim().split(/\s+/).filter(Boolean);
        if (c.length) s += '.' + c.join('.');
    }
    return s;
}

var bad = [], all = document.querySelectorAll('body *');
for (var i = 0; i < all.length; i++) {
    var el = all[i];
    if (!hasOwnText(el) || !visible(el)) continue;
    var cs = getComputedStyle(el);
    // Blind spot: a letter layered on top of an icon glyph (absolutely positioned,
    // parent also directly holds a .material-icons glyph). The letter's real
    // backing is the glyph, not the tinted circle the flat-bg walk finds.
    if (cs.position === 'absolute' && el.parentElement
        && el.parentElement.querySelector(':scope > .material-icons')
        && !el.classList.contains('material-icons')) continue;
    var fg = parseColor(cs.color);
    var bg = effectiveBg(el);
    if (!bg) continue;
    if (fg[3] < 1) fg = over(fg, bg);
    var cr = ratio(fg, bg);
    var px = parseFloat(cs.fontSize), bold = (+cs.fontWeight) >= 700;
    var large = px >= 24 || (px >= 18.66 && bold) || el.classList.contains('material-icons');
    var min = large ? 3.0 : 4.5;
    if (cr < min - 0.05) {
        bad.push({ s: selector(el), t: (el.textContent || '').trim().slice(0, 24), r: Math.round(cr * 100) / 100, m: min });
    }
}
return JSON.stringify(bad.slice(0, 40));
