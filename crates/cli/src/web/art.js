"use strict";
// What objects look like, drawn the same way on the Modeling diagram (SVG) and in
// the animation (canvas). Oblique view: seen from above, the top of an object is
// raised by half its height and its front face shows below it. Plan coordinates
// (where the object stands) don't change; only the drawing has height.
//
// Each look is drawn in its own meters: x from 0 to `w`, y from -h/2 (the raised
// top) to `d` (the front face's bottom edge); its footprint is 0..w × 0..d. The
// object's position is the look's `anchor` (its footprint center by default).
// A fill of "state" takes the server-state color; `shade` darkens it (front faces).

/** Rounded rectangle, circle and ellipse as path data. */
function artRect(x, y, w, h, r = 0) {
  const n = v => +v.toFixed(3);
  r = Math.min(r, w / 2, h / 2);
  if (!r) return `M${n(x)},${n(y)}h${n(w)}v${n(h)}h${n(-w)}Z`;
  return `M${n(x + r)},${n(y)}H${n(x + w - r)}A${n(r)},${n(r)} 0 0 1 ${n(x + w)},${n(y + r)}V${n(y + h - r)}A${n(r)},${n(r)} 0 0 1 ${n(x + w - r)},${n(y + h)}`
    + `H${n(x + r)}A${n(r)},${n(r)} 0 0 1 ${n(x)},${n(y + h - r)}V${n(y + r)}A${n(r)},${n(r)} 0 0 1 ${n(x + r)},${n(y)}Z`;
}
function artEllipse(cx, cy, rx, ry = rx) {
  const n = v => +v.toFixed(3);
  return `M${n(cx - rx)},${n(cy)}A${n(rx)},${n(ry)} 0 1 0 ${n(cx + rx)},${n(cy)}A${n(rx)},${n(ry)} 0 1 0 ${n(cx - rx)},${n(cy)}Z`;
}

const ART = (() => {
  const R = artRect, C = artEllipse;
  const INK = "#2A2F36", STEEL = "#E9EEF3", PANEL = "#23272E", SCREEN = "#5B7A99", GREEN = "#7BD88F", AMBER = "#FFB547";
  const KRAFT = "#D2AA73", KRAFT_EDGE = "#8E6A3E";

  /** A control panel: dark box, small screen, two lights. */
  const panel = (x, y, w, h) => [
    { d: R(x, y, w, h, 0.05), fill: PANEL },
    { d: R(x + 0.05, y + 0.06, w - 0.1, Math.min(0.3, h * 0.35), 0.03), fill: SCREEN },
    { d: C(x + w * 0.3, y + h - 0.13, 0.05), fill: GREEN },
    { d: C(x + w * 0.7, y + h - 0.13, 0.05), fill: AMBER },
  ];

  /** A machine housing in the state color: front face, plinth, then the top. */
  const housing = (w, d, h) => [
    { d: R(0.03, d - h / 2 - 0.4, w - 0.06, h / 2 + 0.37, 0.22), fill: "state", shade: 0.26 },
    { d: R(0.03, d - 0.16, w - 0.06, 0.13, 0.06), fill: "#1B1F24", op: 0.45 },
    { d: R(0.03, -h / 2 + 0.03, w - 0.06, d - 0.06, 0.22), fill: "state", stroke: "#14181D", sop: 0.45, sw: 0.06 },
    { d: R(0.03, -h / 2 + 0.03, w - 0.06, Math.min(0.8, d * 0.35), 0.22), fill: "#FFFFFF", op: 0.12 },
  ];

  /** A person standing at (x, y) on the floor, seen from the front and above. */
  const person = (x, y, { shirt = "#3E5C86", vest = true, helmet = true } = {}) => [
    { d: C(x + 0.05, y, 0.38, 0.13), fill: "#000000", op: 0.15 },
    { d: R(x - 0.13, y - 0.42, 0.26, 0.42, 0.07), fill: "#2E3A4D" },
    { d: R(x - 0.22, y - 0.84, 0.44, 0.5, 0.14), fill: shirt },
    ...(vest ? [{ d: R(x - 0.22, y - 0.64, 0.44, 0.06), fill: "#D9F24B" }] : []),
    ...(helmet
      ? [{ d: C(x, y - 0.9, 0.22, 0.06), fill: "#E6E9ED" }, { d: C(x, y - 1.0, 0.17), fill: "#FFFFFF", stroke: "#9AA0A8", sw: 0.03 }]
      : [{ d: C(x, y - 0.98, 0.16), fill: "#C99A72" }, { d: R(x - 0.16, y - 1.15, 0.32, 0.14, 0.07), fill: "#4A3628" }]),
  ];

  /** Staff: hidden while their server is off shift. */
  const staff = shapes => shapes.map(s => ({ ...s, staff: true }));

  /** A floor pad in the state color, for servers that aren't a machine. */
  const pad = (w, d) => [{ d: R(0, 0, w, d, 0.3), fill: "state", stroke: "#000000", sop: 0.15, sw: 0.05 }];

  const ribs = (x0, x1, y0, y1, step) => {
    let d = "";
    for (let x = x0; x <= x1 + 1e-9; x += step) d += `M${+x.toFixed(3)},${y0}V${y1}`;
    return d;
  };

  return {
    machine: {
      label: "Machine", sections: ["servers"], w: 2.4, d: 2.0, h: 1.6,
      shapes: [
        ...housing(2.4, 2.0, 1.6),
        { d: R(0.3, 1.36, 1.1, 0.42, 0.05), fill: "#FFFFFF", op: 0.18 },
        { d: R(0.25, -0.55, 1.4, 1.5, 0.12), fill: "#FFFFFF", op: 0.28 },
        { d: R(0.65, -0.2, 0.6, 0.6, 0.1), fill: STEEL, stroke: INK, sw: 0.05 },
        { d: C(0.95, 0.1, 0.13), fill: "#8A939E" },
        ...panel(1.85, -0.55, 0.36, 1.0),
      ],
    },
    lathe: {
      label: "Lathe", sections: ["servers"], w: 3.0, d: 2.2, h: 1.4,
      shapes: [
        ...housing(3.0, 2.2, 1.4),
        { d: "M1.0,1.75V2.0M2.0,1.75V2.0", stroke: "#000000", sop: 0.22, sw: 0.04 },
        { d: R(0.22, -0.48, 1.9, 1.76, 0.12), fill: "#0B0F14", op: 0.2 },
        { d: "M1.2,0.05H1.95M1.2,0.75H1.95", stroke: "#FFFFFF", sop: 0.55, sw: 0.06, cap: "round" },
        { d: C(0.75, 0.4, 0.42), fill: STEEL, stroke: INK, sw: 0.05 },
        { d: C(0.75, 0.4, 0.16), fill: "#8A939E" },
        { d: R(1.6, 0.22, 0.36, 0.36, 0.05), fill: STEEL, stroke: INK, sw: 0.04 },
        { d: R(2.3, -0.48, 0.48, 1.15, 0.07), fill: "#FFFFFF", op: 0.3 },
        ...panel(2.3, 0.78, 0.48, 0.5),
      ],
    },
    mill: {
      label: "Milling machine", sections: ["servers"], w: 2.6, d: 2.6, h: 2.0,
      shapes: [
        ...housing(2.6, 2.6, 2.0),
        { d: R(0.25, 1.8, 1.4, 0.55, 0.05), fill: "#FFFFFF", op: 0.18 },
        { d: R(0.22, -0.78, 1.7, 1.7, 0.12), fill: "#FFFFFF", op: 0.28 },
        { d: R(0.4, -0.2, 1.35, 0.6, 0.04), fill: "#0B0F14", op: 0.25 },
        { d: R(0.75, -0.35, 0.65, 0.65, 0.1), fill: STEEL, stroke: INK, sw: 0.05 },
        { d: C(1.075, -0.025, 0.13), fill: "#8A939E" },
        { d: "M0.35,1.1H1.85M0.35,1.28H1.85", stroke: "#0B0F14", sop: 0.28, sw: 0.05, cap: "round" },
        ...panel(2.05, -0.78, 0.36, 1.0),
      ],
    },
    oven: {
      label: "Oven", sections: ["servers"], w: 3.0, d: 2.4, h: 1.8, holds: true,
      shapes: [
        ...housing(3.0, 2.4, 1.8),
        { d: R(0.35, 1.62, 2.3, 0.62, 0.06), fill: "#000000", op: 0.2 },
        { d: R(0.6, 1.72, 1.8, 0.3, 0.05), fill: "#FFD27A", op: 0.9 },
        { d: "M0.8,2.14H2.2", stroke: STEEL, sw: 0.06, cap: "round" },
        ...[-0.6, -0.35, -0.1, 0.15].map(y => ({ d: R(0.3, y, 1.5, 0.1, 0.05), fill: "#000000", op: 0.25 })),
        { d: C(2.4, -0.35, 0.28), fill: "#6E7178", stroke: "#3A3D42", sw: 0.04 },
        { d: C(2.4, -0.35, 0.15), fill: "#2E3033" },
        ...panel(2.05, 0.3, 0.6, 0.5),
      ],
    },
    workstation: {
      label: "Workstation", sections: ["servers"], w: 2.6, d: 1.8, h: 0.9,
      shapes: [
        ...pad(2.6, 1.8),
        { d: R(0.25, 0.0, 2.1, 0.88, 0.04), fill: "#7E8894" },
        { d: R(0.4, 0.6, 1.8, 0.28), fill: "#5C6470" },
        { d: R(0.2, -0.35, 2.2, 0.8, 0.05), fill: "#C2C9D2", stroke: "#6D7783", sw: 0.04 },
        { d: R(0.35, -0.2, 0.3, 0.25, 0.03), fill: "#3A3D42" },
        { d: R(1.0, -0.28, 0.55, 0.55, 0.03), fill: KRAFT, stroke: KRAFT_EDGE, sw: 0.03 },
        { d: R(1.07, -0.21, 0.41, 0.41), fill: "#9C7547" },
        { d: R(1.95, -0.18, 0.3, 0.24, 0.04), fill: "#3A3D42" },
        ...staff(person(1.3, 1.62)),
      ],
    },
    counter: {
      label: "Service counter", sections: ["servers"], w: 2.6, d: 1.8, h: 1.0,
      shapes: [
        ...pad(2.6, 1.8),
        ...staff(person(1.3, 0.55, { shirt: "#4B5D73", vest: false, helmet: false })),
        { d: R(0.15, 0.8, 2.3, 0.55, 0.04), fill: "#B49A72" },
        { d: R(0.15, 0.25, 2.3, 0.6, 0.05), fill: "#D8C3A0", stroke: "#9C8463", sw: 0.04 },
        { d: R(0.95, -0.05, 0.7, 0.32, 0.03), fill: "#3A3D42" },
        { d: R(1.25, 0.25, 0.1, 0.1), fill: "#2E3033" },
        { d: R(0.35, 0.38, 0.35, 0.25, 0.02), fill: "#FFFFFF", stroke: "#C9C6BD", sw: 0.02 },
      ],
    },
    kiosk: {
      label: "Kiosk", sections: ["servers"], w: 1.0, d: 0.8, h: 1.5,
      shapes: [
        { d: C(0.55, 0.8, 0.45, 0.12), fill: "#000000", op: 0.12 },
        { d: R(0.1, -0.2, 0.8, 0.98, 0.1), fill: "state", shade: 0.26 },
        { d: R(0.22, 0.12, 0.56, 0.34, 0.04), fill: "#CFE3F7" },
        { d: R(0.38, 0.58, 0.24, 0.05), fill: "#1F2124" },
        { d: R(0.1, -0.72, 0.8, 0.75, 0.1), fill: "state", stroke: "#14181D", sop: 0.45, sw: 0.05 },
        { d: R(0.1, -0.72, 0.8, 0.3, 0.1), fill: "#FFFFFF", op: 0.15 },
      ],
    },
    // A trailer backed up to a dock leveler; the object stands on the leveler.
    dock: {
      label: "Truck dock", sections: ["sources", "sinks"], w: 4.2, d: 2.6, h: 3.0, anchor: [3.9, 1.3],
      shapes: [
        { d: R(0.25, 0.3, 3.5, 2.4, 0.1), fill: "#000000", op: 0.12 },
        { d: R(3.65, 0, 0.5, 2.6), fill: "#E8B730" },
        ...[0.2, 0.7, 1.2, 1.7, 2.2].map(y => ({ d: `M3.65,${y + 0.3}L4.15,${y}V${y + 0.2}L3.65,${y + 0.5}Z`, fill: "#2B2B2A" })),
        { d: R(2.3, 2.35, 0.55, 0.35, 0.1), fill: "#1F2124" },
        { d: R(2.95, 2.35, 0.55, 0.35, 0.1), fill: "#1F2124" },
        { d: R(0.03, 0.5, 3.54, 1.95, 0.08), fill: "#A9AFB7" },
        { d: ribs(0.45, 3.15, 1.15, 2.4, 0.45), stroke: "#979DA6", sw: 0.05 },
        { d: R(0.03, -1.47, 3.54, 2.54, 0.08), fill: "#CDD2D8", stroke: "#8F959D", sw: 0.05 },
        { d: ribs(0.45, 3.15, -1.38, 0.98, 0.45), stroke: "#B5BBC3", sw: 0.05 },
        { d: R(3.42, -1.4, 0.15, 2.4), fill: "#5D636B" },
      ],
    },
    door: {
      label: "Door", sections: ["sources", "sinks"], w: 2.2, d: 1.0, h: 2.4,
      shapes: [
        { d: R(0.35, 0.2, 1.5, 0.8, 0.08), fill: "#5C6470" },
        { d: "M0.55,0.4H1.65M0.55,0.6H1.65M0.55,0.8H1.65", stroke: "#4A515B", sw: 0.04 },
        { d: R(0, -0.9, 0.4, 1.2), fill: "#D8D4CB" },
        { d: R(1.8, -0.9, 0.4, 1.2), fill: "#D8D4CB" },
        { d: R(0.45, -0.9, 0.64, 1.15, 0.02), fill: "#CFE3F7", op: 0.75, stroke: "#8FA6BC", sw: 0.03 },
        { d: R(1.11, -0.9, 0.64, 1.15, 0.02), fill: "#CFE3F7", op: 0.75, stroke: "#8FA6BC", sw: 0.03 },
        { d: R(0, -1.2, 2.2, 0.3), fill: "#3A3A38" },
        { d: "M0.8,0.6H1.4M1.2,0.45L1.4,0.6L1.2,0.75", stroke: "#FFFFFF", sw: 0.06, cap: "round" },
      ],
    },
    bin: {
      label: "Bin", sections: ["sinks"], w: 1.4, d: 1.2, h: 1.0,
      shapes: [
        { d: R(0.2, 0.3, 1.3, 1.0, 0.1), fill: "#000000", op: 0.12 },
        { d: R(0.05, 0.2, 1.3, 0.97, 0.06), fill: "#6E7178" },
        { d: R(0.05, 0.55, 1.3, 0.12), fill: "#E8B730" },
        { d: R(0.05, -0.47, 1.3, 0.9, 0.06), fill: "#8A8F96", stroke: "#4A4D52", sw: 0.04 },
        { d: R(0.18, -0.36, 1.04, 0.68, 0.04), fill: "#2E3033" },
        { d: "M0.35,-0.1L0.6,-0.25L0.7,0.05ZM0.8,0.1L1.05,-0.05L1.1,0.2Z", fill: KRAFT },
        { d: R(0.55, -0.3, 0.3, 0.12, 0.02), fill: "#9AA0A8" },
      ],
    },
    generic: {
      label: "Generic", sections: ["sources", "sinks"], w: 1.4, d: 1.4, h: 0,
      shapes: [
        { d: R(0, 0, 1.4, 1.4, 0.35), fill: "#C9C6BD", stroke: "#9A978F", sw: 0.05 },
        { d: "M0.4,0.7H1.0M0.8,0.5L1.0,0.7L0.8,0.9", stroke: "#5C5B57", sw: 0.1, cap: "round" },
      ],
    },
    // Vehicles face right (forks first); the animation mirrors them going left.
    // `load` is where what they carry sits: [x, depth, height] in their meters.
    forklift: {
      label: "Forklift", sections: ["vehicles"], w: 2.9, d: 1.1, h: 2.2, load: [2.43, 0.53, 0.1],
      shapes: [
        { d: R(0.15, 0.3, 2.75, 0.95, 0.3), fill: "#000000", op: 0.14 },
        { d: R(1.82, -0.95, 0.14, 1.05, 0.02), fill: "#2E3136" },
        { d: R(1.96, 0.16, 0.94, 0.14, 0.03), fill: "#4A4E55" },
        { d: R(1.96, 0.72, 0.94, 0.14, 0.03), fill: "#4A4E55" },
        { d: R(0.05, 0.6, 1.77, 0.4, 0.12), fill: "#C9971F" },
        { d: R(0.05, -0.5, 1.77, 1.1, 0.14), fill: "#F2C230", stroke: "#8C6A12", sw: 0.03 },
        { d: R(0.05, -0.5, 0.42, 1.1, 0.14), fill: "#000000", op: 0.1 },
        { d: C(0.45, 0.96, 0.28, 0.14), fill: "#222326" },
        { d: C(0.45, 0.96, 0.1, 0.05), fill: "#9AA0A8" },
        { d: C(1.5, 0.96, 0.28, 0.14), fill: "#222326" },
        { d: C(1.5, 0.96, 0.1, 0.05), fill: "#9AA0A8" },
        { d: R(0.72, -0.2, 0.4, 0.5, 0.08), fill: "#2B2D31" },
        { d: R(0.68, -0.48, 0.12, 0.62, 0.04), fill: "#1F2124" },
        { d: R(0.84, -0.72, 0.3, 0.42, 0.1), fill: "#3E5C86" },
        { d: C(0.99, -0.86, 0.13), fill: "#FFFFFF", stroke: "#9AA0A8", sw: 0.03 },
        { d: "M1.3,-0.12L1.42,0.12", stroke: "#1F2124", sw: 0.05, cap: "round" },
        { d: "M0.62,0.5V-0.05M1.58,0.5V-0.05", stroke: "#2B2D31", sw: 0.07, cap: "round" },
        { d: R(0.58, -0.97, 1.04, 0.94, 0.06), fill: "#2B2D31", op: 0.3, stroke: "#2B2D31", sw: 0.07 },
        { d: "M0.58,-0.66H1.62M0.58,-0.35H1.62", stroke: "#2B2D31", sw: 0.04 },
        { d: C(1.1, -1.0, 0.07), fill: AMBER },
        { d: R(1.82, -0.15, 0.14, 1.05, 0.02), fill: "#3A3D42" },
        { d: R(1.82, -0.95, 0.14, 0.12, 0.02), fill: "#3A3D42" },
        { d: R(1.96, -0.35, 0.07, 1.3, 0.02), fill: "#55595F" },
      ],
    },
    // A low automated guided vehicle that carries loads on its deck.
    agv: {
      label: "AGV", sections: ["vehicles"], w: 1.4, d: 1.0, h: 0.4, load: [0.7, 0.5, 0.4],
      shapes: [
        { d: R(0.1, 0.25, 1.4, 0.9, 0.2), fill: "#000000", op: 0.14 },
        { d: R(0, 0.75, 1.4, 0.21, 0.08), fill: "#5E626A" },
        { d: R(0.12, 0.82, 1.16, 0.04, 0.02), fill: "#4FA3FF" },
        { d: R(0, -0.2, 1.4, 1.0, 0.15), fill: "#E3E6EA", stroke: "#8F959D", sw: 0.03 },
        { d: R(0.17, -0.07, 1.06, 0.74, 0.06), fill: "#C5CAD1" },
        { d: C(1.24, 0.3, 0.09), fill: "#2B2D31" },
        { d: C(0.15, 0.3, 0.05), fill: GREEN },
      ],
    },
    // Parts. "state" is the part type's color where a model has several types
    // (`typed` shapes are drawn only then), else `plain`. `stands`: waits in
    // front of a server rather than on it.
    box: {
      label: "Box", sections: ["entities"], w: 0.5, d: 0.5, h: 0.4,
      shapes: [
        { d: R(0.06, 0.1, 0.5, 0.48, 0.05), fill: "#000000", op: 0.14 },
        { d: R(0, 0.24, 0.5, 0.26, 0.04), fill: "#A98252" },
        { d: R(0.22, 0.3, 0.06, 0.2), fill: "#C9AE7E" },
        { d: R(0, -0.2, 0.5, 0.5, 0.05), fill: KRAFT, stroke: KRAFT_EDGE, sw: 0.03 },
        { d: R(0.22, -0.185, 0.06, 0.47), fill: "#EFDDB8" },
        { d: R(0.11, -0.1, 0.28, 0.3, 0.04), fill: "state", stroke: "#FFFFFF", sw: 0.03, sop: 0.8, typed: true },
      ],
    },
    // A wooden pallet with four cartons, shrink-wrapped.
    pallet: {
      label: "Pallet", sections: ["entities"], w: 1.2, d: 1.0, h: 1.0,
      shapes: [
        { d: R(0.1, 0.2, 1.2, 0.95, 0.05), fill: "#000000", op: 0.14 },
        { d: R(0, 0.9, 1.2, 0.1), fill: "#8E6534" },
        { d: "M0.08,0.93h0.22v0.07h-0.22ZM0.49,0.93h0.22v0.07h-0.22ZM0.9,0.93h0.22v0.07h-0.22Z", fill: "#5E4120" },
        { d: R(0.02, 0.5, 1.16, 0.42, 0.03), fill: "#A98252" },
        { d: "M0.6,0.5V0.92M0.02,0.71H1.18", stroke: "#7A5A33", sw: 0.03 },
        { d: R(0.02, -0.48, 1.16, 0.98, 0.04), fill: KRAFT, stroke: KRAFT_EDGE, sw: 0.03 },
        { d: "M0.6,-0.48V0.5M0.02,0.01H1.18", stroke: KRAFT_EDGE, sw: 0.03 },
        { d: R(0.02, -0.48, 1.16, 1.4, 0.04), fill: "#FFFFFF", op: 0.12 },
        { d: R(0.4, -0.22, 0.4, 0.44, 0.05), fill: "state", stroke: "#FFFFFF", sw: 0.04, sop: 0.8, typed: true },
      ],
    },
    // A person standing, the shirt in the type's color.
    person: {
      label: "Person", sections: ["entities"], w: 0.6, d: 0.5, h: 2.2, plain: "#5B7FA6", stands: true,
      shapes: person(0.3, 0.35, { shirt: "state", vest: false, helmet: false }),
    },
  };
})();

/** What an object looks like when its layout doesn't say. */
const ART_DEFAULT = { sources: "dock", servers: "machine", sinks: "dock", vehicles: "forklift", entities: "box" };
/** Older names, still read. */
const ART_ALIASES = { machine_small: "machine" };

/** The look of an object: { kind, art, mirror }. A sink's dock has its trailer on
 *  the right, since parts arrive from the left; an unknown look falls back to the default. */
function artFor(section, model) {
  const name = ART_ALIASES[model] || model;
  const kind = ART[name]?.sections.includes(section) ? name : ART_DEFAULT[section];
  return { kind, art: ART[kind], mirror: section === "sinks" && kind === "dock" };
}

/** The looks an object in `section` can have: [kind, label]. */
function artChoices(section) {
  return Object.entries(ART).filter(([, a]) => a.sections.includes(section)).map(([k, a]) => [k, a.label]);
}

// A server that works on several parts at once is drawn as that many units
// (`look.units`), side by side, up to four abreast, more rows behind; units in a
// row 0.3 m apart, rows 1.2 m apart, so people stand between them. A look that
// `holds` many parts (an oven) stays one drawing, and so does a server with more
// than ART_MAX_UNITS.
const ART_MAX_UNITS = 12;

/** Whether `units` of a look are drawn one by one. */
const artGroups = (look, units) => units > 1 && units <= ART_MAX_UNITS && !look.art.holds;

/** Where the units of a look go for an object at (x, z): one (x, z) each, the
 *  front row first, left to right, all centered on (x, z). */
function artUnits(look, x, z) {
  const n = look.units || 1;
  if (n < 2) return [[x, z]];
  const rows = Math.ceil(n / 4), cols = Math.ceil(n / rows);
  const [dx, dz] = [look.art.w + 0.3, look.art.d + 1.2];
  const at = [];
  for (let r = 0; r < rows; r++) {
    const k = Math.min(cols, n - r * cols);
    for (let c = 0; c < k; c++) at.push([x + (c - (k - 1) / 2) * dx, z + ((rows - 1) / 2 - r) * dz]);
  }
  return at;
}

/** One unit of a look. */
const artUnit = look => look.units ? { ...look, units: 0 } : look;

/** Where a look's drawing goes for an object at (x, z): its top-left corner and size. */
function artBox(look, x, z) {
  if (look.units > 1) {
    const boxes = artUnits(look, x, z).map(([ux, uz]) => artBox(artUnit(look), ux, uz));
    const x0 = Math.min(...boxes.map(b => b.x)), y0 = Math.min(...boxes.map(b => b.y));
    return {
      x: x0, y: y0, w: Math.max(...boxes.map(b => b.x + b.w)) - x0, h: Math.max(...boxes.map(b => b.y + b.h)) - y0,
      top: Math.min(...boxes.map(b => b.top)), mirror: look.mirror,
    };
  }
  const { art, mirror } = look;
  const [ax, az] = art.anchor || [art.w / 2, art.d / 2];
  const left = x - (mirror ? art.w - ax : ax);
  return { x: left, y: z - az - art.h / 2, w: art.w, h: art.d + art.h / 2, top: z - az, mirror };
}

/** The look's drawing as an SVG group, `fill` being the state color. */
function artSvg(look, x, z, fill) {
  if (look.units > 1) {
    const g = svg("g", { class: "art", "aria-hidden": "true" });
    artUnits(look, x, z).reverse().forEach(([ux, uz]) => g.append(artSvg(artUnit(look), ux, uz, fill)));
    return g;
  }
  const b = artBox(look, x, z);
  const t = `translate(${+b.x.toFixed(3)},${+b.top.toFixed(3)})` + (b.mirror ? ` translate(${look.art.w},0) scale(-1,1)` : "");
  const g = svg("g", { transform: t, class: "art", "aria-hidden": "true" });
  for (const s of look.art.shapes) {
    const attrs = { d: s.d, fill: s.fill === "state" ? fill : (s.fill || "none") };
    if (s.op !== undefined) attrs["fill-opacity"] = s.op;
    if (s.stroke) Object.assign(attrs, { stroke: s.stroke, "stroke-width": s.sw || 0.05 });
    if (s.sop !== undefined) attrs["stroke-opacity"] = s.sop;
    if (s.cap) attrs["stroke-linecap"] = s.cap;
    g.append(svg("path", attrs));
    if (s.fill === "state" && s.shade) g.append(svg("path", { d: s.d, fill: "#000000", "fill-opacity": s.shade }));
  }
  return g;
}

const ART_PATHS = new Map();
const artPath = d => {
  let p = ART_PATHS.get(d);
  if (!p) ART_PATHS.set(d, (p = new Path2D(d)));
  return p;
};

/** Draws the look on a canvas whose meters map to pixels by `X`, `Y` and
 *  `scale`; without its staff when not `staffed` (off shift). */
function artCanvas(ctx, look, x, z, fill, X, Y, scale, staffed = true) {
  if (look.units > 1) {
    artUnits(look, x, z).reverse().forEach(([ux, uz]) => artCanvas(ctx, artUnit(look), ux, uz, fill, X, Y, scale, staffed));
    return;
  }
  const b = artBox(look, x, z);
  ctx.save();
  ctx.translate(X(b.x), Y(b.top));
  ctx.scale(scale, scale);
  if (b.mirror) { ctx.translate(look.art.w, 0); ctx.scale(-1, 1); }
  artShapesCanvas(ctx, look.art.shapes, fill, staffed);
  ctx.restore();
}

/** An object's footprint on the floor: { x0, z0, x1, z1 }. */
function artFootprint(look, x, z) {
  if (look.units > 1) {
    const fs = artUnits(look, x, z).map(([ux, uz]) => artFootprint(artUnit(look), ux, uz));
    return {
      x0: Math.min(...fs.map(f => f.x0)), z0: Math.min(...fs.map(f => f.z0)),
      x1: Math.max(...fs.map(f => f.x1)), z1: Math.max(...fs.map(f => f.z1)),
    };
  }
  const b = artBox(look, x, z);
  return { x0: b.x, z0: b.top, x1: b.x + look.art.w, z1: b.top + look.art.d };
}

// Connections are drawn as strokes along their path, in meters: `w` wide, moved
// `dy` down (negative is up), `off` to the right of the way parts go, dashed by
// `dash`. A dashed thick stroke makes the rollers.

/** How high a belt's top is drawn above the floor: half its 0.7 m height. */
const BELT_TOP = 0.35;
const ART_BELT = [
  { dy: 0.14, w: 1.0, color: "#000000", op: 0.1 },
  { dy: 0, w: 0.9, color: "#5E626A" },
  { dy: -BELT_TOP / 2, w: 0.9, color: "#5E626A" },
  { dy: -BELT_TOP, w: 0.9, color: "#464A51" },
  { dy: -BELT_TOP, w: 0.72, color: "#80848D" },
  { dy: -BELT_TOP, w: 0.72, color: "#B0B5BD", dash: [0.08, 0.22] },
];
/** A queue: a lane painted on the floor, where parts wait in line. */
const ART_LANE = [
  { w: 0.9, color: "#E8B730", op: 0.16 },
  { off: -0.42, w: 0.06, color: "#D9A21B" },
  { off: 0.42, w: 0.06, color: "#D9A21B" },
];

/** The polyline moved `off` meters to the right of its direction (corners mitered). */
function artOffset(points, off) {
  if (!off || points.length < 2) return points;
  const normal = (a, b) => {
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
    return [-(b[1] - a[1]) / len, (b[0] - a[0]) / len];
  };
  return points.map((p, i) => {
    const n0 = normal(points[Math.max(0, i - 1)], points[Math.max(1, i)]);
    const n1 = normal(points[Math.min(i, points.length - 2)], points[Math.min(i + 1, points.length - 1)]);
    const [nx, nz] = [n0[0] + n1[0], n0[1] + n1[1]];
    const k = off / Math.max(0.3, (nx * n0[0] + nz * n0[1]));
    return [p[0] + nx * k, p[1] + nz * k];
  });
}

const artPoints = (points, s) => artOffset(points, s.off).map(([x, z]) => [x, z + (s.dy || 0)]);

/** A connection's strokes as an SVG group. */
function artLineSvg(points, strokes) {
  const g = svg("g", { class: "art", "aria-hidden": "true" });
  for (const s of strokes) {
    const attrs = {
      points: artPoints(points, s).map(([x, z]) => `${+x.toFixed(3)},${+z.toFixed(3)}`).join(" "),
      fill: "none", stroke: s.color, "stroke-width": s.w, "stroke-linejoin": "round",
    };
    if (s.op !== undefined) attrs["stroke-opacity"] = s.op;
    if (s.dash) attrs["stroke-dasharray"] = s.dash.join(" ");
    g.append(svg("polyline", attrs));
  }
  return g;
}

/** Draws a connection's strokes on a canvas whose meters map to pixels by `X`, `Y` and `scale`. */
function artLineCanvas(ctx, points, strokes, X, Y, scale) {
  ctx.save();
  ctx.translate(X(0), Y(0));
  ctx.scale(scale, scale);
  ctx.lineJoin = "round";
  ctx.lineCap = "butt";
  for (const s of strokes) {
    ctx.globalAlpha = s.op ?? 1;
    ctx.strokeStyle = s.color;
    ctx.lineWidth = s.w;
    ctx.setLineDash(s.dash || []);
    ctx.beginPath();
    artPoints(points, s).forEach(([x, z], i) => (i ? ctx.lineTo(x, z) : ctx.moveTo(x, z)));
    ctx.stroke();
  }
  ctx.restore();
}

/** Draws shapes (as in ART) at the canvas's current transform; staff only if
 *  `staffed`, shapes for a part type only if `typed`. */
function artShapesCanvas(ctx, shapes, fill, staffed = true, typed = true) {
  for (const s of shapes) {
    if ((s.staff && !staffed) || (s.typed && !typed)) continue;
    const p = artPath(s.d);
    if (s.fill) {
      ctx.globalAlpha = s.op ?? 1;
      ctx.fillStyle = s.fill === "state" ? fill : s.fill;
      ctx.fill(p);
      if (s.fill === "state" && s.shade) { ctx.globalAlpha = s.shade; ctx.fillStyle = "#000000"; ctx.fill(p); }
    }
    if (s.stroke) {
      ctx.globalAlpha = s.sop ?? 1;
      ctx.strokeStyle = s.stroke;
      ctx.lineWidth = s.sw || 0.05;
      ctx.lineCap = s.cap || "butt";
      ctx.stroke(p);
    }
  }
  ctx.globalAlpha = 1;
}

/** Draws a part (`look`, from artFor("entities", ...)) standing at (x, z), its
 *  bottom `lift` meters above the floor (on a belt or a machine), in the part
 *  type's color `type` if given. It never gets smaller than `minPx` across, so
 *  parts stay visible zoomed out. */
function artPartCanvas(ctx, look, x, z, lift, type, X, Y, scale, minPx = 7) {
  const art = look.art;
  const k = Math.max(scale, minPx / art.w);
  ctx.save();
  ctx.translate(X(x), Y(z - lift));
  ctx.scale(k, k);
  ctx.translate(-art.w / 2, -art.d / 2);
  artShapesCanvas(ctx, art.shapes, type || art.plain, true, Boolean(type));
  ctx.restore();
}
