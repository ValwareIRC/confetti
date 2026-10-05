/* Confetti — the icon set.
 *
 * Drawn here rather than borrowed from the emoji table. An emoji is somebody
 * else's illustration at somebody else's weight, in a colour we do not choose
 * and a shape that changes with the operating system; a row of them next to
 * each other never looks like one set of anything.
 *
 * These are all the same drawing: a 20×20 box with two units of air around
 * the edge, one stroke weight throughout, round caps and joins, and no fill
 * except where a dot is genuinely a dot. They take their colour from whatever
 * they sit in, so a category icon is the category's own colour and an icon in
 * a menu is the colour of the text beside it.
 */

const ICON_NS = "http://www.w3.org/2000/svg";

const ICONS = {
  /* ---- categories ---- */
  // Who this server says it is: a card with a face and a couple of lines.
  identity: '<rect x="3" y="5" width="14" height="10" rx="2"/>' +
            '<circle cx="7.2" cy="10" r="1.9"/><path d="M11.5 8.6h3.2M11.5 11.4h3.2"/>',
  // A door in.
  door: '<path d="M5.5 3.5h9v13h-9z"/><path d="M12 10h.01"/>' +
        '<circle cx="12" cy="10" r="0.85" fill="currentColor" stroke="none"/>',
  // Operators.
  staff: '<circle cx="10" cy="7" r="2.7"/>' +
         '<path d="M4.5 16.5c0-3 2.5-4.9 5.5-4.9s5.5 1.9 5.5 4.9"/>',
  // Two servers, joined.
  link: '<rect x="2.5" y="3.5" width="6" height="5" rx="1.2"/>' +
        '<rect x="11.5" y="11.5" width="6" height="5" rx="1.2"/>' +
        '<path d="M8.5 6h3a2.5 2.5 0 0 1 2.5 2.5V11.5"/>',
  // Keeping order.
  shield: '<path d="M10 3l6 2.1v4.7c0 3.6-2.4 6.3-6 7.7-3.6-1.4-6-4.1-6-7.7V5.1z"/>',
  // The set block.
  sliders: '<path d="M3 6.6h14M3 13.4h14"/><circle cx="7.6" cy="6.6" r="2.1"/>' +
           '<circle cx="13" cy="13.4" r="2.1"/>',
  // What it loads.
  blocks: '<rect x="2.5" y="2.5" width="7" height="7" rx="1.3"/>' +
          '<rect x="10.5" y="2.5" width="7" height="7" rx="1.3"/>' +
          '<rect x="6.5" y="10.5" width="7" height="7" rx="1.3"/>',
  // What people read.
  text: '<path d="M5.5 3.5h6l4 4v9h-10z"/><path d="M11.5 3.5V7.5h4"/>' +
        '<path d="M7.5 10.5h6M7.5 13.3h4"/>',
  // A command shortcut.
  shortcut: '<path d="M4 15.5V9.5A3.5 3.5 0 0 1 7.5 6H15"/>' +
            '<path d="M12 3l3.2 3-3.2 3"/>',
  // What gets written down.
  ledger: '<rect x="3.5" y="3" width="13" height="14" rx="1.5"/><path d="M7 3v14"/>' +
          '<path d="M9.5 7h4.4M9.5 10h4.4M9.5 13h3"/>',
  // Secrets.
  key: '<circle cx="6.2" cy="10" r="3.2"/><path d="M9.4 10h7.1"/>' +
       '<path d="M13 10v2.7M15.6 10v2"/>',
  // Driving the server from outside.
  exchange: '<path d="M6.2 15.5v-11M6.2 4.5 3.7 7M6.2 4.5 8.7 7"/>' +
            '<path d="M13.8 4.5v11M13.8 15.5 11.3 13M13.8 15.5 16.3 13"/>',
  // The include lines.
  fileIn: '<path d="M9 3.5h4l3.5 3.5v9.5H9"/><path d="M12.6 3.5V7.4h3.9"/>' +
          '<path d="M2.5 10h6"/><path d="M6.2 7.5 8.7 10l-2.5 2.5"/>',
  // Whatever is left.
  dots: '<circle cx="4.8" cy="10" r="1.4" fill="currentColor" stroke="none"/>' +
        '<circle cx="10" cy="10" r="1.4" fill="currentColor" stroke="none"/>' +
        '<circle cx="15.2" cy="10" r="1.4" fill="currentColor" stroke="none"/>',

  /* ---- viewers ---- */
  tree: '<rect x="7" y="2.5" width="6" height="4.2" rx="1.2"/>' +
        '<rect x="2" y="13.3" width="6" height="4.2" rx="1.2"/>' +
        '<rect x="12" y="13.3" width="6" height="4.2" rx="1.2"/>' +
        '<path d="M10 6.7v3.3M5 13.3V10h10v3.3"/>',
  grid: '<rect x="3" y="3" width="14" height="14" rx="1.6"/>' +
        '<path d="M3 8h14M3 12.8h14M8.4 3v14"/>',
  balance: '<path d="M10 4.2v11.6M6.4 15.8h7.2M3 8h14"/>' +
           '<path d="M3 8 1.4 11.6h3.2z"/><path d="M17 8l1.6 3.6h-3.2z"/>' +
           '<circle cx="10" cy="4.2" r="1.1"/>',
  funnel: '<path d="M3 4.5h14l-5 5.8v6.2l-4-2.1v-4.1z"/>',
  web: '<circle cx="10" cy="4.4" r="2"/><circle cx="4.4" cy="14.2" r="2"/>' +
       '<circle cx="15.6" cy="14.2" r="2"/>' +
       '<path d="M8.6 6.2 5.5 12.3M11.4 6.2l3.1 6.1M6.4 14.2h7.2"/>',
  plug: '<path d="M7 2.8v4.2M13 2.8v4.2"/>' +
        '<path d="M4.6 7h10.8v2.2a5.4 5.4 0 0 1-10.8 0z"/><path d="M10 14.6v2.6"/>',
  noentry: '<circle cx="10" cy="10" r="7"/><path d="M5.5 5.5 14.5 14.5"/>',
  people: '<circle cx="7.2" cy="7.4" r="2.5"/><circle cx="14" cy="8.4" r="2"/>' +
          '<path d="M2.6 16.2c0-2.7 2.1-4.3 4.6-4.3s4.6 1.6 4.6 4.3"/>' +
          '<path d="M13.2 12.1c2.4 0 4.2 1.4 4.2 4.1"/>',
  list: '<path d="M7.2 5.6h9.8M7.2 10h9.8M7.2 14.4h6.8"/>' +
        '<circle cx="3.6" cy="5.6" r="1.3" fill="currentColor" stroke="none"/>' +
        '<circle cx="3.6" cy="10" r="1.3" fill="currentColor" stroke="none"/>' +
        '<circle cx="3.6" cy="14.4" r="1.3" fill="currentColor" stroke="none"/>',
  box: '<path d="M10 2.8 17 6.4v7.2L10 17.2 3 13.6V6.4z"/>' +
       '<path d="M3 6.4 10 10l7-3.6M10 10v7.2"/>',
  signal: '<circle cx="10" cy="14.2" r="1.8" fill="currentColor" stroke="none"/>' +
          '<path d="M6.6 11.2a4.9 4.9 0 0 1 6.8 0"/><path d="M4 8.6a8.4 8.4 0 0 1 12 0"/>',
  receipt: '<path d="M4.5 3.2h11v12.2l-2 1.4-1.8-1.3-1.7 1.3-1.8-1.3-1.7 1.3-2-1.4z"/>' +
           '<path d="M7.6 6.8h4.8M7.6 9.8h4.8"/>',

  /* ---- small ones, used in the interface itself ---- */
  refresh: '<path d="M16.4 10a6.4 6.4 0 1 1-2.4-5"/><path d="M16.8 2.6v3.9H13"/>',
  alert: '<path d="M10 3.2 17.2 16.4H2.8z"/><path d="M10 7.8v3.9"/>' +
         '<circle cx="10" cy="13.9" r="0.9" fill="currentColor" stroke="none"/>',
};

/* An icon, sized by the font around it and coloured by whatever it sits in. */
function icon(name, cls) {
  const svg = document.createElementNS(ICON_NS, "svg");
  svg.setAttribute("viewBox", "0 0 20 20");
  svg.setAttribute("class", "ico" + (cls ? " " + cls : ""));
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  svg.innerHTML = ICONS[name] || ICONS.dots;
  return svg;
}

const hasIcon = (name) => Object.prototype.hasOwnProperty.call(ICONS, name);

/* Icons Confetti used to ship as characters. A saved config keeps whatever
   icon it had — you are allowed to choose one — so the old glyphs are read as
   the drawing that replaced them rather than left sitting among the new set
   at the wrong weight. */
const LEGACY_ICONS = {
  "⌂": "identity", "⚿": "door", "★": "staff", "⇄": "link",
  "⚑": "shield", "⚖": "shield", "⚙": "sliders", "◈": "blocks",
  "¶": "text", "⇥": "shortcut", "▤": "ledger", "✦": "key",
  "⇅": "exchange", "↳": "fileIn", "☰": "dots", "◆": "blocks",
};

/* What to draw for a category. Its own name if it is one of ours, the drawing
   that replaced it if it is a character we used to ship, and otherwise
   whatever you typed, left exactly as you typed it. */
function catIcon(c) {
  const name = hasIcon(c.icon) ? c.icon : LEGACY_ICONS[c.icon];
  if (name) return icon(name);
  return el("span", "icoc", c.icon || "");
}

/* The icons a category may be given, in the order they are offered. */
const CATEGORY_ICONS = [
  "identity", "door", "staff", "link", "shield", "sliders", "blocks", "text",
  "shortcut", "ledger", "key", "exchange", "fileIn", "list", "box", "grid",
  "web", "plug", "noentry", "people", "funnel", "signal", "tree", "dots",
];
