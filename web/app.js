// カタン Web UI。
//
// Rust 側とは「JSON を読む」「合法手を番号で返す」の 2 つだけでやり取りする。
// wasm-bindgen も npm も挟んでいないので、ここが全部。
//
// 見た目は「木の盤の再現」を狙っていない。海は端まで続き、枠は無い。

import * as sfx from "./sfx.js";

const SVG_NS = "http://www.w3.org/2000/svg";
const RES = ["WOOD", "BRICK", "SHEEP", "WHEAT", "ORE"];
const RES_JA = { WOOD: "木材", BRICK: "土", SHEEP: "羊毛", WHEAT: "小麦", ORE: "鉄鉱" };
const DEV_JA = ["騎士", "街道建設", "収穫", "独占", "勝利点"];
const PLAYER_COLORS = ["#ff5a5a", "#4f9dff", "#f2f0ea", "#ffa23d"];
// 駒の陰の色。くすませると盤の地形に埋もれて「誰がどこに建てたか」が読めない。
// 暗くするのではなく、彩度を保ったまま一段落とすだけにする
const PLAYER_DARK = ["#d13a3a", "#2f74d8", "#cdc7b8", "#e08a1e"];
// 白い欄（できごと・プレイヤー・詳細ログ）で使う色。
// 盤の上の鮮やかな色は白地では読めない（とくに P2 は白なので消える）。
// 同じ人だと分かる範囲で一段濃くした対（ink）を持つ
const PLAYER_INK = ["#d93a35", "#1f6fd0", "#7b8794", "#cf7414"];

const TERRAIN = {
  WOOD:   { a: "#3f8a51", b: "#1c4f2e", dark: "#153c22", light: "#5aa96b" },
  BRICK:  { a: "#c26a3f", b: "#8a3d1e", dark: "#6a2c12", light: "#dd8f61" },
  SHEEP:  { a: "#9ecb5e", b: "#69973a", dark: "#476f22", light: "#c4e58c" },
  WHEAT:  { a: "#e6bc57", b: "#bd8c22", dark: "#87610e", light: "#f6dc95" },
  ORE:    { a: "#8b919c", b: "#575d67", dark: "#3c4149", light: "#b9bec7" },
  DESERT: { a: "#dbc899", b: "#b59e6d", dark: "#8e7b50", light: "#efe0bb" },
};

// 紋章の色。単色で塗るより「何の資源か」が一瞬で分かる
const GLYPH = {
  WOOD:  { main: "#4cc46c", sub: "#8a5a34" },
  BRICK: { main: "#e87a50", sub: "#a8452a" },
  SHEEP: { main: "#f7f5ed", sub: "#4a5a34" },
  WHEAT: { main: "#f3c64e", sub: "#b8862a" },
  ORE:   { main: "#9aa3b1", sub: "#dbe3ee" },
};

let wasm = null;
let board = null;
let state = null;
let watching = false;
let botTimer = null;
let lastPieces = new Set();
let lastDiceKey = "";
let lastSeq = 0;
/** 盗賊が一度でも動かされたか。最初の砂漠に居るうちは動きを付けない */
let robberEverMoved = false;
let mySeat = 0;   // 人間の席（seed から決まる）

// ---------------------------------------------------------------- wasm 橋渡し

function readJson(ptr) {
  const len = wasm.out_len();
  // memory.buffer はメモリ拡張で差し替わるので毎回取り直す
  const bytes = new Uint8Array(wasm.memory.buffer, ptr, len);
  return JSON.parse(new TextDecoder("utf-8").decode(bytes));
}

async function loadWasm() {
  const res = await fetch("catan_wasm.wasm");
  const { instance } = await WebAssembly.instantiate(await res.arrayBuffer(), {});
  wasm = instance.exports;
}

// ---------------------------------------------------------------- SVG の小道具

function el(tag, attrs = {}, parent = null) {
  const n = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  if (parent) parent.appendChild(n);
  return n;
}

function raw(parent, svgText) {
  const g = document.createElementNS(SVG_NS, "g");
  g.innerHTML = svgText;
  parent.appendChild(g);
  return g;
}

/** 尖り上ヘクス（左右に隣がある向き）の頂点 */
function hexPoints(cx, cy, s) {
  const pts = [];
  for (let k = 0; k < 6; k++) {
    const a = ((60 * k - 90) * Math.PI) / 180;
    pts.push(`${(cx + s * Math.cos(a)).toFixed(2)},${(cy + s * Math.sin(a)).toFixed(2)}`);
  }
  return pts.join(" ");
}

// ---------------------------------------------------------------- 紋章
//
// 港・手札・プレイヤー欄で使う。小さくても読めることだけを狙って専用に描いてある
// （ヘクスの大きな意匠を縮小すると細部が潰れて何の絵か分からなくなる）。
// `mono` に色を渡せば単色、渡さなければ資源ごとの色で描く。

function smallGlyph(kind, mono) {
  const g = GLYPH[kind] || { main: mono || "#f2e9d6", sub: mono || "#f2e9d6" };
  const a = mono || g.main;
  const b = mono || g.sub;
  switch (kind) {
    case "WOOD":
      return `<rect x="-1.6" y="4" width="3.2" height="7" rx="1" fill="${b}"/>
              <path d="M0,-11 L6,-1 L3,-1 L8,6 L-8,6 L-3,-1 L-6,-1 Z" fill="${a}"/>`;
    case "BRICK":
      return `<rect x="-9" y="-6.5" width="18" height="6" rx="1.4" fill="${a}"/>
              <rect x="-9" y="0.5" width="8.2" height="6" rx="1.4" fill="${b}"/>
              <rect x="0.8" y="0.5" width="8.2" height="6" rx="1.4" fill="${b}"/>
              <rect x="-9" y="-6.5" width="18" height="2" rx="1" fill="#fff" opacity=".22"/>`;
    case "SHEEP":
      return `<rect x="-4" y="5" width="2.2" height="4.5" fill="${b}"/>
              <rect x="2" y="5" width="2.2" height="4.5" fill="${b}"/>
              <ellipse cx="-1" cy="0.5" rx="8" ry="6" fill="${a}"/>
              <circle cx="-7" cy="-3.4" r="3.6" fill="${a}"/>
              <circle cx="3.6" cy="-4.2" r="3.4" fill="${a}"/>
              <circle cx="7" cy="1.6" r="4.2" fill="${b}"/>
              <circle cx="8.4" cy="0.4" r="1" fill="#fff" opacity=".9"/>`;
    case "WHEAT":
      return `<path d="M0,10 L0,-9" stroke="${b}" stroke-width="2" fill="none"/>
              ${[0, 1, 2]
                .map(
                  (i) =>
                    `<ellipse cx="-3.4" cy="${-7 + i * 5.5}" rx="3.4" ry="2.1" fill="${a}" transform="rotate(-34 -3.4 ${-7 + i * 5.5})"/>
                     <ellipse cx="3.4" cy="${-7 + i * 5.5}" rx="3.4" ry="2.1" fill="${a}" transform="rotate(34 3.4 ${-7 + i * 5.5})"/>`
                )
                .join("")}`;
    case "ORE":
      return `<path d="M-10,8 L-3,-9 L2,-1 L6,-6 L10,8 Z" fill="${a}"/>
              <path d="M-3,-9 L0.6,-3.6 L-6,-3.6 Z" fill="${b}"/>
              <path d="M6,-6 L9,-1 L3.4,-1 Z" fill="${b}" opacity=".8"/>`;
    default:
      return `<circle r="6" fill="${a}"/>`;
  }
}

function glyphHtml(kind, size, mono) {
  return `<svg width="${size}" height="${size}" viewBox="-12 -12 24 24">${smallGlyph(kind, mono)}</svg>`;
}

/** プレイヤー欄の持ち物アイコン */
/**
 * 欄の小さな絵。地色が 2 系統あるので、色も 2 系統持つ。
 *
 * `dark` = 盤の上の暗い面（明るい灰色の絵に暗い抜き）、
 * `light` = 白い欄（濃い青灰の絵に白い抜き）。
 * 白い欄に `dark` の絵を置くと、白地に薄灰で沈んで読めない。
 */
function iconHtml(kind, size = 17, tone = "dark") {
  const light = tone === "light";
  const c = light ? "#2c4257" : "#7f95a8";
  const k = light ? "#ffffff" : "#0c141c";
  const ko = light ? ".92" : ".55";
  const inner = {
    hand: `<rect x="-7.5" y="-10" width="15" height="20" rx="3" fill="${c}"/>
           <rect x="-4.5" y="-7" width="9" height="5" rx="1" fill="${k}" opacity="${ko}"/>`,
    dev: `<rect x="-7.5" y="-10" width="15" height="20" rx="3" fill="${c}"/>
          <path d="M0,-6 L1.9,-1.6 L6.6,-1.2 L3,1.9 L4.1,6.5 L0,4 L-4.1,6.5 L-3,1.9 L-6.6,-1.2 L-1.9,-1.6 Z" fill="${k}" opacity="${ko}"/>`,
    knight: `<path d="M0,-10 L8,-6 V2 Q8,8 0,11 Q-8,8 -8,2 V-6 Z" fill="${c}"/>
             <path d="M0,-5 L0,6 M-4,0 L4,0" stroke="${k}" stroke-width="2.2" opacity="${ko}"/>`,
    road: `<rect x="-10" y="-3.2" width="20" height="6.4" rx="3.2" fill="${c}" transform="rotate(-20)"/>`,
    settle: `<path d="M0,-9 L8,-2 V8 H-8 V-2 Z" fill="${c}"/>`,
    city: `<path d="M-9,8 V-2 L-3,-8 L3,-2 V1 H9 V8 Z" fill="${c}"/>`,
  }[kind];
  return `<svg width="${size}" height="${size}" viewBox="-12 -12 24 24">${inner}</svg>`;
}

// ---------------------------------------------------------------- 地形の意匠

function motifWood(t) {
  const tree = (x, y, sc) => `
    <g transform="translate(${x},${y}) scale(${sc})">
      <path d="M0,-15 L7.5,-3 L4,-3 L9.5,8 L-9.5,8 L-4,-3 L-7.5,-3 Z" fill="${t.dark}"/>
      <path d="M0,-15 L7.5,-3 L4,-3 L9.5,8 L0,8 Z" fill="${t.light}" opacity=".38"/>
      <rect x="-1.7" y="8" width="3.4" height="5" fill="${t.dark}"/>
    </g>`;
  return tree(-25, 20, 1.0) + tree(25, 20, 1.0) + tree(0, 28, 0.82);
}

function motifBrick(t) {
  const brick = (x, y) => `<rect x="${x}" y="${y}" width="15" height="7" rx="1.6" fill="${t.dark}"/>`;
  return `
    <path d="M-34,30 Q-17,6 0,30 Z" fill="${t.dark}" opacity=".55"/>
    <path d="M2,30 Q19,10 34,30 Z" fill="${t.dark}" opacity=".4"/>
    ${brick(-20, 20)}${brick(-3, 20)}${brick(-12, 29)}${brick(5, 29)}
    <rect x="-20" y="20" width="15" height="3" rx="1.5" fill="${t.light}" opacity=".35"/>`;
}

function motifSheep(t) {
  return `
    <g transform="translate(0,22)">
      <path d="M-30,10 Q-15,2 0,10 Q15,18 30,10" stroke="${t.dark}" stroke-width="2.5" fill="none" opacity=".5"/>
      <g transform="translate(-2,-2)">
        <ellipse cx="0" cy="0" rx="13" ry="9.5" fill="#f3f2ec"/>
        <ellipse cx="-8" cy="-4" rx="6" ry="5" fill="#fbfaf6"/>
        <ellipse cx="7" cy="-4" rx="6" ry="5" fill="#fbfaf6"/>
        <circle cx="13" cy="-4" r="5.4" fill="${t.dark}"/>
        <circle cx="15.4" cy="-5.4" r="1.2" fill="#fff" opacity=".8"/>
        <rect x="-7" y="8" width="3" height="7" rx="1.5" fill="${t.dark}"/>
        <rect x="4" y="8" width="3" height="7" rx="1.5" fill="${t.dark}"/>
      </g>
    </g>`;
}

function motifWheat(t) {
  const stalk = (x, y, sc) => `
    <g transform="translate(${x},${y}) scale(${sc})">
      <path d="M0,16 L0,-12" stroke="${t.dark}" stroke-width="2.2" fill="none"/>
      ${[0, 1, 2, 3]
        .map(
          (i) =>
            `<ellipse cx="-4" cy="${-10 + i * 6}" rx="4" ry="2.6" fill="${t.dark}" transform="rotate(-32 -4 ${-10 + i * 6})"/>
             <ellipse cx="4" cy="${-10 + i * 6}" rx="4" ry="2.6" fill="${t.dark}" transform="rotate(32 4 ${-10 + i * 6})"/>`
        )
        .join("")}
    </g>`;
  return stalk(-22, 22, 0.95) + stalk(0, 28, 1.05) + stalk(22, 22, 0.95);
}

function motifOre(t) {
  return `
    <g transform="translate(0,24)">
      <path d="M-32,12 L-13,-16 L-1,2 L7,-9 L32,12 Z" fill="${t.dark}"/>
      <path d="M-13,-16 L-4.5,-3.5 L-13,-3.5 Z" fill="#f0f2f6" opacity=".85"/>
      <path d="M7,-9 L12,-2 L2,-2 Z" fill="#f0f2f6" opacity=".7"/>
    </g>`;
}

function motifDesert(t) {
  return `
    <g transform="translate(0,22)">
      <path d="M-32,14 Q-16,4 0,14 Q16,24 32,14" stroke="${t.dark}" stroke-width="2.6" fill="none" opacity=".55"/>
      <path d="M-26,24 Q-10,16 6,24" stroke="${t.dark}" stroke-width="2.2" fill="none" opacity=".38"/>
      <g transform="translate(16,2)">
        <rect x="-3" y="-14" width="6" height="24" rx="3" fill="${t.dark}"/>
        <path d="M-3,-4 h-6 a3,3 0 0 0 -3,3 v5" stroke="${t.dark}" stroke-width="5" fill="none" stroke-linecap="round"/>
        <path d="M3,-8 h5 a3,3 0 0 1 3,3 v7" stroke="${t.dark}" stroke-width="5" fill="none" stroke-linecap="round"/>
      </g>
    </g>`;
}

function motif(kind) {
  const t = TERRAIN[kind];
  return { WOOD: motifWood, BRICK: motifBrick, SHEEP: motifSheep, WHEAT: motifWheat, ORE: motifOre }
    [kind]?.(t) ?? motifDesert(t);
}

// ---------------------------------------------------------------- 駒

function settlementShape(color, dark) {
  return `
    <rect x="5.5" y="-14" width="4.4" height="7" rx="1" fill="${dark}" stroke="#050b11" stroke-width="1.2"/>
    <path d="M-13.5,-2.4 L0,-14.5 L13.5,-2.4 Z" fill="${dark}" stroke="#04090e" stroke-width="1.8" stroke-linejoin="round"/>
    <path d="M-10.4,-5.2 L0,-14.5 L10.4,-5.2" fill="none" stroke="#fff" stroke-width="1.1" opacity=".28"/>
    <path d="M-7.2,-8 L0,-14.5 L7.2,-8" fill="none" stroke="#fff" stroke-width="1.1" opacity=".2"/>
    <path d="M-11,-2.4 L11,-2.4 L11,11 L-11,11 Z" fill="${color}" stroke="#04090e" stroke-width="1.8" stroke-linejoin="round"/>
    <path d="M-11,-2.4 L11,-2.4 L11,1.4 L-11,1.4 Z" fill="#fff" opacity=".16"/>
    <rect x="-2.9" y="2.6" width="5.8" height="8.4" rx="1" fill="#2a1c11" stroke="#050b11" stroke-width="1"/>
    <circle cx="1.5" cy="7" r=".9" fill="#f4d489"/>
    <rect x="-8.6" y="2.8" width="4.2" height="4.2" rx=".8" fill="#ffdf9b" stroke="#050b11" stroke-width="1"/>
    <rect x="4.4" y="2.8" width="4.2" height="4.2" rx=".8" fill="#ffdf9b" stroke="#050b11" stroke-width="1"/>`;
}

function cityShape(color, dark) {
  // 胸壁つきの城。旗・灯りの入った窓・門で「格上げされた」ことを一目で
  const merlon = (x, y, w) =>
    `<rect x="${x}" y="${y}" width="${w}" height="3.6" fill="${color}" stroke="#050b11" stroke-width="1.1"/>`;
  return `
    <path d="M-4.5,-19 L8,-16.4 L-4.5,-13.8 Z" fill="#ff5d5d" stroke="#050b11" stroke-width="1"/>
    <path d="M-5,-20 L-5,-6" stroke="#0b1219" stroke-width="1.8" stroke-linecap="round"/>
    <path d="M-12,-6 L2,-6 L2,12 L-12,12 Z" fill="${color}" stroke="#04090e" stroke-width="1.8" stroke-linejoin="round"/>
    ${merlon(-12.6, -9.4, 4)}${merlon(-6.9, -9.4, 4)}${merlon(-1.2, -9.4, 4)}
    <path d="M2,0 L16,0 L16,12 L2,12 Z" fill="${dark}" stroke="#04090e" stroke-width="1.8" stroke-linejoin="round"/>
    ${merlon(1.6, -3.2, 3.6)}${merlon(6.6, -3.2, 3.6)}${merlon(11.6, -3.2, 3.6)}
    <path d="M-12,-6 L2,-6 L2,-2.4 L-12,-2.4 Z" fill="#fff" opacity=".18"/>
    <rect x="-9.6" y="-2.4" width="3.6" height="4.6" rx=".8" fill="#ffdf9b" stroke="#050b11" stroke-width=".9"/>
    <rect x="-3.6" y="-2.4" width="3.6" height="4.6" rx=".8" fill="#ffdf9b" stroke="#050b11" stroke-width=".9"/>
    <rect x="5.4" y="3.4" width="3.4" height="4.2" rx=".8" fill="#ffdf9b" stroke="#050b11" stroke-width=".9"/>
    <path d="M-8.4,12 L-8.4,5.6 a3.4,3.4 0 0 1 6.8,0 L-1.6,12 Z" fill="#2a1c11" stroke="#050b11" stroke-width="1"/>`;
}

/**
 * 新しい道を「どちら側から伸ばしたか」。
 * 自分の建物がある端 → 自分の既存の道がつながっている端、の順で探す。
 * どちらにも当たらなければ 1（初期配置の 1 本目など）。
 */
function baseEnd(r) {
  const e = board.edges[r.edge];
  const mine = (n) => state.buildings.some((b) => b.node === n && b.owner === r.owner);
  if (mine(e.n1)) return 1;
  if (mine(e.n2)) return 2;
  const touches = (n) =>
    state.roads.some((o) => {
      if (o.edge === r.edge || o.owner !== r.owner) return false;
      const oe = board.edges[o.edge];
      return oe.n1 === n || oe.n2 === n;
    });
  if (touches(e.n1)) return 1;
  if (touches(e.n2)) return 2;
  return 1;
}

/** 街道。ただの太い線ではなく、枕木を並べた敷板として描く */
function roadShape(e, color, dark, growFrom) {
  const mx = (e.x1 + e.x2) / 2;
  const my = (e.y1 + e.y2) / 2;
  const dx = e.x2 - e.x1;
  const dy = e.y2 - e.y1;
  const len = Math.hypot(dx, dy) * 0.92;
  const deg = (Math.atan2(dy, dx) * 180) / Math.PI;
  const h = 13;
  const x0 = -len / 2;

  // 枕木。長さに応じて本数を決める
  const n = Math.max(3, Math.round(len / 9));
  let ties = "";
  for (let i = 0; i < n; i++) {
    const tx = x0 + 3 + ((len - 6) * (i + 0.5)) / n - 1.3;
    ties += `<rect x="${tx.toFixed(2)}" y="${-h / 2 + 1.4}" width="2.6" height="${h - 2.8}"
             rx="1" fill="${dark}" opacity=".55"/>`;
  }
  // growFrom: 1 = e.n1 側から、2 = e.n2 側から敷く（自分の拠点のある方）。
  // 横に引き伸ばすと枕木まで伸びて「板が伸びた」ように見えるので、
  // 出来上がりの絵を **端から順に見せる**（clip-path）形にする
  const grow = growFrom ? ` class="roadgrow ${growFrom === 1 ? "from1" : "from2"}"` : "";
  return `<g transform="translate(${mx.toFixed(2)},${my.toFixed(2)}) rotate(${deg.toFixed(2)})">
    <g${grow}>
      <rect x="${x0 - 1.4}" y="${-h / 2 - 1.4}" width="${len + 2.8}" height="${h + 2.8}" rx="${h / 2 + 1.4}"
            fill="#050b11" opacity=".85"/>
      <rect x="${x0}" y="${-h / 2}" width="${len}" height="${h}" rx="${h / 2}" fill="${color}"/>
      <rect x="${x0 + 2}" y="${-h / 2 + 1}" width="${len - 4}" height="${h * 0.36}" rx="${h * 0.18}"
            fill="#fff" opacity=".42"/>
      ${ties}
      <rect x="${x0 + 1}" y="${-h / 2 + h * 0.66}" width="${len - 2}" height="${h * 0.3}" rx="${h * 0.15}"
            fill="#000" opacity=".16"/>
      <circle cx="${x0 + 2.6}" cy="0" r="2.1" fill="${dark}" stroke="#050b11" stroke-width="1"/>
      <circle cx="${-x0 - 2.6}" cy="0" r="2.1" fill="${dark}" stroke="#050b11" stroke-width="1"/>
    </g></g>`;
}

/**
 * 盗賊。駒 1 つではなく、**フードを被った一団**として描く。
 *
 * 単体の丸い駒だと「置いてあるコマ」にしか見えず、その土地が止まっていることが伝わらない。
 * 3 人は同じ絵の色違いではなく、**役ごとに別の絵**を持つ。
 * 動きだけを付け替えても、絵が同じなら「何をしているのか」は伝わらないため。
 *   jab   = そばの建物にナイフでちょっかいをかける（腕が伸びる）
 *   sleep = 足を組んで寝ている（盗んだ袋を枕元に置いている）
 *   look  = 銃を担いで見張っている（左右をゆっくり見る）
 */

/** 3 人に共通の外套と、顔の位置の黒い抜き */
function banditCloak(edge = "#93a1af") {
  return `<path d="M0,-20 C7.4,-20 11,-14.6 11,-8.2 L11,0 C11,3.2 12.2,6.6 13.4,9.6 L-13.4,9.6
             C-12.2,6.6 -11,3.2 -11,0 L-11,-8.2 C-11,-14.6 -7.4,-20 0,-20 Z"
          fill="#1b232c" stroke="${edge}" stroke-width="1.7" stroke-linejoin="round"/>
    <path d="M0,-16.6 C5.1,-16.6 7.7,-12.7 7.7,-8.4 L7.7,-4.9
             C4.7,-2.7 -4.7,-2.7 -7.7,-4.9 L-7.7,-8.4 C-7.7,-12.7 -5.1,-16.6 0,-16.6 Z"
          fill="#04080c"/>`;
}

const banditShadow = (rx = 12, y = 10.6) =>
  `<ellipse cx="0" cy="${y}" rx="${rx}" ry="3.4" fill="#04080c" opacity=".42"/>`;

const banditEyes = `<circle class="eye" cx="-3.2" cy="-8.8" r="1.55" fill="#ffd166"/>
    <circle class="eye" cx="3.2" cy="-8.8" r="1.55" fill="#ffd166"/>`;

/** フードの闇の中に浮かぶ笑み。7 で追い出される時（＝望んで移る時）だけ見える */
const banditGrin =
  `<path class="grin" d="M-3.8,-5.6 Q0,-2.6 3.8,-5.6" stroke="#ffd166" stroke-width="1.4"
         fill="none" stroke-linecap="round"/>`;

/** 汗。騎士に追い出される時だけ飛ぶ。滴ごとに向きと出る間を変える */
const banditSweat = `<g class="sweat">
    <path style="--sx:-7px" d="M-9,-15 q-1.6,2.6 0,3.8 q1.6,-1.2 0,-3.8 Z" fill="#9fe0ff"/>
    <path style="--sx:7px; animation-delay:.16s" d="M9,-17 q-1.6,2.6 0,3.8 q1.6,-1.2 0,-3.8 Z" fill="#9fe0ff"/>
    <path style="--sx:2px; animation-delay:.3s" d="M2,-19 q-1.4,2.2 0,3.2 q1.4,-1 0,-3.2 Z" fill="#9fe0ff"/>
  </g>`;

/** 突き役。腕は別の `<g>` にして、体とは別の速さで前に伸ばす */
function banditJab() {
  return `
    ${banditShadow()}
    ${banditCloak()}
    ${banditEyes}
    ${banditGrin}
    ${banditSweat}
    <g transform="translate(7,-7)"><g class="arm">
      <path d="M0,-2.8 L12,-4.2 L12,2.2 L0,2.8 Z"
            fill="#1b232c" stroke="#93a1af" stroke-width="1.3" stroke-linejoin="round"/>
      <g transform="translate(12,-1)">
        <rect x="-3.2" y="-2.4" width="3.6" height="4.8" rx="1.2" fill="#5c4a30" stroke="#2a2114" stroke-width=".9"/>
        <path d="M0.4,-1.9 L13,-0.5 L13,0.5 L0.4,1.9 Z"
              fill="#dfe7f0" stroke="#6d7c8b" stroke-width=".9" stroke-linejoin="round"/>
      </g>
    </g></g>`;
}

/** 寝役。座っているのではなく、袋を枕に**寝そべって**いる。目は閉じている */
function banditSleep() {
  return `
    <ellipse cx="1" cy="9" rx="19" ry="3.4" fill="#04080c" opacity=".42"/>
    <!-- 枕にした袋 -->
    <g transform="translate(-16,4.5) scale(.92)">
      <path d="M-4.4,0 Q0,-7.2 4.4,0 Q4.4,6.1 0,6.1 Q-4.4,6.1 -4.4,0 Z"
            fill="#5c4a30" stroke="#2a2114" stroke-width="1.3" stroke-linejoin="round"/>
      <path d="M-2.3,-2.5 L2.3,-2.5" stroke="#2a2114" stroke-width="1.3" stroke-linecap="round"/>
    </g>
    <!-- 投げ出して組んだ足 -->
    <path d="M5,2.4 L21,-1.6" stroke="#141b22" stroke-width="5.2" stroke-linecap="round"/>
    <path d="M5,6.2 L21,3" stroke="#222c36" stroke-width="5.2" stroke-linecap="round"/>
    <!-- 横になった胴。頭の側が高く、足に向かって下がる -->
    <path d="M-12,8.4 C-13.4,-2 -6,-7 1,-4.2 C7.4,-1.6 12.4,3.4 14.6,8.4 Z"
          fill="#1b232c" stroke="#93a1af" stroke-width="1.7" stroke-linejoin="round"/>
    <!-- 頭は枕の上。フードごと横倒し -->
    <g transform="translate(-9.5,-2.4) rotate(-76)">
      <path d="M0,-8.8 C5.4,-8.8 8.4,-4.8 8.4,-0.6 L8.4,3.2 C4.8,5.4 -4.8,5.4 -8.4,3.2
               L-8.4,-0.6 C-8.4,-4.8 -5.4,-8.8 0,-8.8 Z"
            fill="#1b232c" stroke="#93a1af" stroke-width="1.6" stroke-linejoin="round"/>
      <path d="M-6.6,0.2 C-3.4,2.4 3.4,2.4 6.6,0.2 L6.6,2.4 C3.4,4.2 -3.4,4.2 -6.6,2.4 Z" fill="#04080c"/>
      <path d="M-3.4,1.6 Q-2,2.9 -0.6,1.6" stroke="#ffd166" stroke-width="1.1" fill="none" stroke-linecap="round"/>
      <path class="grin" d="M0.4,-1.4 Q2.6,1.2 0.4,3.4" stroke="#ffd166" stroke-width="1.3"
            fill="none" stroke-linecap="round"/>
    </g>
    <g class="sweat">
      <path style="--sx:-6px" d="M-14,-9 q-1.6,2.6 0,3.8 q1.6,-1.2 0,-3.8 Z" fill="#9fe0ff"/>
      <path style="--sx:4px; animation-delay:.2s" d="M-4,-12 q-1.6,2.6 0,3.8 q1.6,-1.2 0,-3.8 Z" fill="#9fe0ff"/>
    </g>
    <!-- だらりと投げ出した腕 -->
    <path d="M-3,-1 C1,3.5 5,4.5 8.5,3.4" stroke="#1b232c" stroke-width="4.2" fill="none" stroke-linecap="round"/>
    <g class="zzz" fill="#eaf6ff" font-size="8" font-weight="700" font-family="system-ui">
      <text x="-14" y="-12" style="animation-delay:0s">z</text>
      <text x="-9" y="-17" style="animation-delay:.9s">z</text>
      <text x="-4" y="-21" style="animation-delay:1.8s">z</text>
    </g>`;
}

/** 見張り役。長い銃を肩に担ぐ */
function banditLook() {
  return `
    ${banditShadow()}
    <!-- 担いだ銃。外套の後ろから前に出す。
         銃身と銃床の両端が外套の輪郭より外へ出ていないと、担いでいることが分からない -->
    <g transform="translate(-1,-3) rotate(-36)">
      <rect x="-5" y="-1.8" width="30" height="3.6" rx="1.6" fill="#3c4a58" stroke="#9fb0c0" stroke-width="1"/>
      <path d="M-5,-2.8 L-17,-1.2 L-17,4 L-5,2.8 Z"
            fill="#6b5637" stroke="#2a2114" stroke-width="1.1" stroke-linejoin="round"/>
      <rect x="7" y="1.6" width="8" height="2.2" rx="1" fill="#2a3542"/>
    </g>
    ${banditCloak()}
    ${banditEyes}
    ${banditGrin}
    ${banditSweat}
    <!-- 銃身の先と銃床の端だけ外套の手前に出して、担いでいるように見せる -->
    <g transform="translate(-1,-3) rotate(-36)">
      <rect x="11" y="-1.8" width="14" height="3.6" rx="1.6" fill="#54677a" stroke="#9fb0c0" stroke-width="1"/>
      <circle cx="25" cy="0" r="2.2" fill="#2a3542" stroke="#9fb0c0" stroke-width=".9"/>
    </g>`;
}

/** 野営のテント。布を張った三角に、入口だけ黒く抜く */
function tentShape() {
  return `
    <ellipse cx="0" cy="11" rx="17" ry="3.6" fill="#04080c" opacity=".4"/>
    <path d="M0,-23 L16,11 L-16,11 Z" fill="#333d4a" stroke="#93a1af" stroke-width="1.7" stroke-linejoin="round"/>
    <path d="M0,-23 L16,11 L5,11 Z" fill="#46525f"/>
    <path d="M-4.6,11 L0,-6 L4.6,11 Z" fill="#04080c"/>
    <path d="M0,-23 L0,-28" stroke="#8a5f22" stroke-width="2.2" stroke-linecap="round"/>
    <path d="M-16,11 L-21,14 M16,11 L21,14" stroke="#6d7c8b" stroke-width="1.4" stroke-linecap="round"/>`;
}

/** 焚き火。まだ誰も襲っていないので、燃え差しと細い煙だけ */
function campfireShape() {
  const stone = (x, y) => `<circle cx="${x}" cy="${y}" r="2.6" fill="#6b7480" stroke="#39414b" stroke-width=".9"/>`;
  return `
    <ellipse cx="0" cy="3" rx="11" ry="3.2" fill="#04080c" opacity=".35"/>
    <path d="M-6,2 L6,-2 M-6,-2 L6,2" stroke="#5c4a30" stroke-width="3.4" stroke-linecap="round"/>
    <ellipse cx="0" cy="0" rx="4.6" ry="2.2" fill="#e0662c"/>
    <ellipse cx="0" cy="0" rx="2.4" ry="1.2" fill="#ffcf6b"/>
    ${stone(-8, 2)}${stone(8, 2)}${stone(0, 5)}${stone(-6, -3)}${stone(6, -3)}
    <path d="M0,-4 C-3,-9 3,-13 0,-19 C-2,-23 1,-25 2,-28"
          stroke="#cfd8e0" stroke-width="1.6" fill="none" opacity=".45" stroke-linecap="round"/>`;
}

/** 座って足を組んだ 1 人。野営で焚き火に当たっている姿 */
function banditSit() {
  return `
    ${banditShadow(14, 9)}
    <path d="M2,4 L15,1" stroke="#141b22" stroke-width="5.4" stroke-linecap="round"/>
    <path d="M2,7.4 L15,4.4" stroke="#222c36" stroke-width="5.4" stroke-linecap="round"/>
    <path d="M0,-9 C7.8,-9 12,-3.4 12,3 L12,8 L-12,8 L-12,3 C-12,-3.4 -7.8,-9 0,-9 Z"
          fill="#1b232c" stroke="#93a1af" stroke-width="1.7" stroke-linejoin="round"/>
    <g transform="translate(-1,-11) rotate(-8)">
      <path d="M0,-8.6 C5.4,-8.6 8.2,-4.6 8.2,-0.4 L8.2,3.4 C4.6,5.6 -4.6,5.6 -8.2,3.4
               L-8.2,-0.4 C-8.2,-4.6 -5.4,-8.6 0,-8.6 Z"
            fill="#1b232c" stroke="#93a1af" stroke-width="1.6" stroke-linejoin="round"/>
      <path d="M-6.4,0.4 C-3.4,2.6 3.4,2.6 6.4,0.4 L6.4,2.4 C3.4,4.2 -3.4,4.2 -6.4,2.4 Z" fill="#04080c"/>
      <circle cx="-2.6" cy="1.6" r="1.4" fill="#ffd166"/>
      <circle cx="2.6" cy="1.6" r="1.4" fill="#ffd166"/>
    </g>`;
}

/**
 * まだ一度も追い出されていない盗賊＝**砂漠の野営**。
 *
 * 開始時点の盗賊は誰の邪魔もしていない。動く一団として描くと、
 * 何も起きていないのに盤で一番目立つものになってしまう。
 * ここは「そこに居着いている」ことだけが伝わればいいので、
 * テントと焚き火のある止まった絵にする。
 */
function robberCampHtml(tile) {
  const [cx, cy] = tileXY(tile);
  return `<g id="robbercamp" transform="translate(${cx},${cy})">
    <g transform="translate(-22,-1)">${tentShape()}</g>
    <g transform="translate(-19,23) scale(.56)">${banditSleep()}</g>
    <g transform="translate(16,-4) scale(.68)">${banditLook()}</g>
    <g transform="translate(6,20) scale(1.05)">${campfireShape()}</g>
    <g transform="translate(29,12) scale(.62)">${banditSit()}</g>
  </g>`;
}

/**
 * 盗賊の一団を、そのヘクスの中に散らして置く。
 *
 * 突き役だけは「建物のある角」の隣に立たせる（つつく相手が要るので）。
 * 向きは絵を左右反転して合わせるので、腕の絵は右向きの 1 種類で足りる。
 */
function robberBandHtml(tile) {
  // 最初の砂漠にいる間は野営の絵。まだ誰も追い出していない
  if (!robberEverMoved) return robberCampHtml(tile);
  const [cx, cy] = tileXY(tile);
  const nodes = (board.tiles[tile] && board.tiles[tile].nodes) || [];
  const built = new Set(state.buildings.map((b) => b.node));

  const cand = nodes.map((n) => {
    const [x, y] = nodeXY(n);
    return { n, ang: Math.atan2(y - cy, x - cx), built: built.has(n) };
  });
  // 突き役の相手は建物のある角。**無ければ突き役は出さない**
  // （何も無い方へ突いていると、何をしているのか分からなくなる）
  const targets = [];
  const bs = cand.filter((c) => c.built);
  const hasVictim = bs.length > 0;
  if (hasVictim) targets.push(bs[0]);
  const rest = cand.filter((c) => c !== targets[0]);
  while (targets.length < 3 && rest.length) {
    let best = 0, bestGap = -1;
    rest.forEach((c, i) => {
      const gap = targets.length
        ? Math.min(...targets.map((p) => Math.abs(Math.atan2(Math.sin(c.ang - p.ang), Math.cos(c.ang - p.ang)))))
        : 9;
      if (gap > bestGap) { bestGap = gap; best = i; }
    });
    targets.push(rest.splice(best, 1)[0]);
  }

  const R = 55.16; // ヘクスの中心から角までの距離
  // 中心には数字の札（半径 21）がある。そこへ被ると出目が読めなくなるので、
  // 3 人とも外側の輪に立たせ、さらに輪に沿ってゆっくり持ち場を移らせる。
  // 止まっていると「たまたま被った 1 人」がずっと数字を隠したままになる
  const roles = [
    hasVictim
      ? { cls: "jab", art: banditJab, s: 0.78, near: 0.68, dy: 6, dur: 4.6 }
      : { cls: "look", art: banditLook, s: 0.78, near: 0.66, dy: 8, dur: 6.4 },
    { cls: "sleep", art: banditSleep, s: 0.74, near: 0.62, dy: 12, dur: 5.6 },
    { cls: "look", art: banditLook, s: 0.72, near: 0.64, dy: 8, dur: 7.2 },
  ];

  return roles
    .map((r, i) => {
      const t = targets[i] || targets[0];
      if (!t) return "";
      const dx = Math.cos(t.ang), dy = Math.sin(t.ang);
      // 数字の札は中心から少し上（0,-6）にある。上側に立つ者は特に被りやすいので、
      // その分だけ外へ出して、下へずらす
      const upper = dy < -0.2;
      const near = upper ? r.near + 0.14 : r.near;
      const x = cx + dx * R * near;
      const y = cy + dy * R * near + r.dy + (upper ? 10 : 0);
      // 突き役は相手のいる側を向く。反転は外側の scale でやるので、
      // 腕の伸びる向き（--px）は絵の中の右方向で固定してよい
      const flip = r.cls === "jab" && dx < 0 ? -1 : 1;
      const vars =
        `--dur:${r.dur}s; --delay:${(i * 0.8).toFixed(1)}s` +
        (r.cls === "jab" ? `; --px:5px; --py:${(dy * 2.5).toFixed(2)}px; --tilt:10deg` : "");
      // 輪に沿った向き（半径に直交）へゆっくり往復させる
      // 上側の者は振れ幅も大きく取る（そこが一番数字を隠す）
      const amp = 11 + i * 3 + (upper ? 13 : 0);
      const wx = (-dy * amp).toFixed(1);
      const wy = (dx * amp).toFixed(1);
      const wander = `--wx:${wx}px; --wy:${wy}px; --wdur:${(7.5 + i * 1.9).toFixed(1)}s; --wdelay:${(i * 1.4).toFixed(1)}s`;
      return `<g transform="translate(${x.toFixed(1)},${y.toFixed(1)})">
        <g class="drift" style="${wander}">
          <g transform="scale(${(r.s * flip).toFixed(2)},${r.s})">
            <g class="bandit ${r.cls}" style="${vars}">${r.art()}</g>
          </g>
        </g>
      </g>`;
    })
    .join("");
}

/**
 * 道や建物を作る人。
 * 腕（＝ハンマー）は別の `<g>` にしてある。歩いている間は担いだまま、
 * 敷き始めてから振り下ろす、という 2 段の動きを付けたいため。
 */
function workerShape(color) {
  return `
    <g class="legs">
      <path d="M-2.6,6 L-2.6,11" stroke="#2b1f14" stroke-width="2.6" stroke-linecap="round"/>
      <path d="M2.6,6 L2.6,11" stroke="#2b1f14" stroke-width="2.6" stroke-linecap="round"/>
    </g>
    <path d="M-4.5,-3 L4.5,-3 L3.5,7 L-3.5,7 Z" fill="${color}" stroke="#050b11" stroke-width="1.2" stroke-linejoin="round"/>
    <circle cx="0" cy="-8" r="4.6" fill="#f0dcc0" stroke="#050b11" stroke-width="1.2"/>
    <path d="M-5,-11 a5,5 0 0 1 10,0 Z" fill="${color}" stroke="#050b11" stroke-width="1.2"/>
    <g class="hammer">
      <path d="M4,-2 L10,-9" stroke="#8b6b3f" stroke-width="2.2" stroke-linecap="round"/>
      <rect x="8" y="-13" width="7" height="4" rx="1" fill="#a9b3bf" stroke="#050b11" stroke-width="1"/>
    </g>`;
}

function knightShape(color) {
  return `
    <path d="M0,-12 L9,-7 V2 Q9,9 0,13 Q-9,9 -9,2 V-7 Z" fill="${color}" stroke="#050b11" stroke-width="1.4"/>
    <path d="M0,-6 L0,7 M-4.5,0 L4.5,0" stroke="#fff" stroke-width="2" opacity=".85"/>
    <path d="M11,-14 L18,-21" stroke="#dfe7ef" stroke-width="3" stroke-linecap="round"/>`;
}

// ---------------------------------------------------------------- 港

/** 陸から船まで伸びる桟橋。杭・板・手すりで組む */
function dockShape(x1, y1, x2, y2, p) {
  const g = GLYPH[p.kind] || {};
  const rail = p.kind === "ANY" ? "#7fd6f0" : g.main || "#7fd6f0";
  const mx = (x1 + x2) / 2;
  const my = (y1 + y2) / 2;
  const len = Math.hypot(x2 - x1, y2 - y1);
  const deg = (Math.atan2(y2 - y1, x2 - x1) * 180) / Math.PI;
  const n = Math.max(3, Math.round(len / 13));
  let piles = "";
  for (let i = 0; i < n; i++) {
    const px = -len / 2 + 4 + ((len - 8) * i) / (n - 1);
    piles += `<rect x="${px.toFixed(2)}" y="-4.6" width="2.4" height="9.2" rx="1"
              fill="#5d3a1c" opacity=".9"/>`;
  }
  return `<g transform="translate(${mx.toFixed(2)},${my.toFixed(2)}) rotate(${deg.toFixed(2)})" class="dock">
      <rect x="${-len / 2}" y="-4.4" width="${len}" height="8.8" rx="2" fill="#a3703c"/>
      <rect x="${-len / 2}" y="-4.4" width="${len}" height="3" rx="1.5" fill="#d09a5c"/>
      ${piles}
      <rect x="${-len / 2}" y="-5.6" width="${len}" height="1.5" rx=".75" fill="${rail}" opacity=".85"/>
      <rect x="${-len / 2}" y="4.1" width="${len}" height="1.5" rx=".75" fill="${rail}" opacity=".6"/>
    </g>`;
}

/** 船 + 「紋章 + レート」の札。桟橋は陸から沖まで伸びる。
 *  2:1 の港はその資源の色を帆と旗に乗せて、遠目でも何の港か分かるようにする */
function portGraphic(p) {
  const isAny = p.kind === "ANY";
  const g = GLYPH[p.kind] || {};
  const sail = isAny ? "#f4f9fd" : g.main || "#f4f9fd";
  const sail2 = isAny ? "#cfe2ef" : g.sub || "#cfe2ef";
  const w = isAny ? 36 : 56;
  const plate = isAny
    ? `<text class="porttext" y="21.5">3:1</text>`
    : `<g transform="translate(-15,21.5) scale(.86)">${smallGlyph(p.kind)}</g>
       <text class="porttext" x="10" y="21.5">2:1</text>`;
  return `
    <g class="portg">
      <path d="M-21,12 q6,-4.5 12,0 t12,0 t12,0" stroke="#bfeaff" stroke-width="2.2" fill="none"
            opacity=".55" stroke-linecap="round"/>
      <path d="M-17,-1 L17,-1 L11.5,9.5 L-11.5,9.5 Z" fill="#8a5a2e" stroke="#3d2412"
            stroke-width="1.5" stroke-linejoin="round"/>
      <path d="M-17,-1 L17,-1 L16,1.6 L-16,1.6 Z" fill="#c08a4e"/>
      <path d="M-15,3.4 L15,3.4" stroke="#5d3a1c" stroke-width="1.3" opacity=".8"/>
      <circle cx="-7" cy="5.6" r="1.5" fill="#3d2412"/>
      <circle cx="0" cy="5.6" r="1.5" fill="#3d2412"/>
      <circle cx="7" cy="5.6" r="1.5" fill="#3d2412"/>
      <path d="M0,-1 L0,-25" stroke="#6b4423" stroke-width="2.2" stroke-linecap="round"/>
      <path d="M0.6,-25 L11,-21.4 L0.6,-19 Z" fill="${sail}" stroke="#3d2412" stroke-width="1"/>
      <path d="M1.8,-18 L16,-3.4 L1.8,-3.4 Z" fill="${sail}" stroke="#3d2412"
            stroke-width="1.2" stroke-linejoin="round"/>
      <path d="M1.8,-18 L16,-3.4 L1.8,-3.4 Z" fill="#000" opacity=".12"/>
      <path d="M-1.8,-21 L-12,-4.4 L-1.8,-4.4 Z" fill="${sail2}" stroke="#3d2412"
            stroke-width="1.2" stroke-linejoin="round"/>
      <path d="M-1.8,-21 L-12,-4.4 L-1.8,-4.4 Z" fill="#fff" opacity=".22"/>
      <rect x="${-w / 2}" y="12" width="${w}" height="19.5" rx="9.8"
            fill="rgba(6,16,24,.92)" stroke="${isAny ? "#7fb2cc" : sail}" stroke-width="1.5"/>
      ${plate}
    </g>`;
}

// ---------------------------------------------------------------- 盤の描画

function defs(svg) {
  const g = [];
  for (const [k, t] of Object.entries(TERRAIN)) {
    g.push(`<linearGradient id="g${k}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="${t.a}"/><stop offset="1" stop-color="${t.b}"/></linearGradient>`);
  }
  raw(svg, `<defs>
      ${g.join("")}
      <radialGradient id="gToken" cx="38%" cy="28%" r="82%">
        <stop offset="0" stop-color="rgba(10,20,28,.86)"/><stop offset="1" stop-color="rgba(4,10,16,.92)"/>
      </radialGradient>
      <filter id="pieceShadow" x="-70%" y="-70%" width="240%" height="240%">
        <feDropShadow dx="0" dy="3" stdDeviation="2.6" flood-color="#000" flood-opacity=".75"/>
        <feDropShadow dx="0" dy="0" stdDeviation="4" flood-color="#000" flood-opacity=".45"/>
      </filter>
      <filter id="hexShadow" x="-30%" y="-30%" width="160%" height="160%">
        <feDropShadow dx="0" dy="5" stdDeviation="6" flood-color="#02080e" flood-opacity=".65"/>
      </filter>
    </defs>`);
}

function drawBoard() {
  const svg = document.getElementById("board");
  svg.innerHTML = "";
  const [vx, vy, vw, vh] = board.viewBox;
  svg.setAttribute("viewBox", `${vx} ${vy} ${vw} ${vh}`);
  // 盤の大きさは縦で決まるので横に余白が出る。
  // その余白を CSS の右パディングで右側に寄せ、盤はその中で中央に置く。
  // ⚠ 操作の欄（右下）とは重なる。盤 922px + 欄 404px = 1326px に対して
  //   置ける幅が 1096px しかないので、動かすだけでは避けられない
  svg.setAttribute("preserveAspectRatio", "xMidYMid meet");
  defs(svg);

  // 海は SVG では描かない。
  // SVG は縦横比を保って中に収まるので、ここで塗ると「収まった矩形」の縁が見えてしまう。
  // 海はページ全体の地（CSS）に置いて、盤はその上に浮かべる。

  const gTiles = el("g", { filter: "url(#hexShadow)" }, svg);
  const gPorts = el("g", {}, svg);
  // 駒は地形の上に置かれるので、影を付けて輪郭を立てる（誰の駒かを一目で）
  const gRoads = el("g", { filter: "url(#pieceShadow)" }, svg);
  const gBuild = el("g", { filter: "url(#pieceShadow)" }, svg);
  const gHints = el("g", {}, svg);
  const gFx = el("g", { "pointer-events": "none" }, svg);
  svg._layers = { gRoads, gBuild, gHints, gFx };

  for (const t of board.tiles) {
    const kind = t.resource || "DESERT";
    const g = el("g", { transform: `translate(${t.x},${t.y})` }, gTiles);
    el("polygon", { points: hexPoints(0, 0, board.hexSize * 0.985), class: "hex", fill: `url(#g${kind})` }, g);
    const m = raw(g, `<g opacity=".8">${motif(kind)}</g>`);
    m.setAttribute("pointer-events", "none");
    el("path", {
      d: `M${-board.hexSize * 0.85},${-board.hexSize * 0.49} L0,${-board.hexSize * 0.98} L${board.hexSize * 0.85},${-board.hexSize * 0.49}`,
      stroke: "#fff", "stroke-width": 2, fill: "none", opacity: ".18",
    }, g);

    if (t.number > 0) {
      const tok = el("g", { transform: "translate(0,-6)" }, g);
      el("circle", { r: 21, fill: "url(#gToken)", stroke: "rgba(234,242,248,.35)", "stroke-width": 1.4 }, tok);
      const red = t.number === 6 || t.number === 8;
      if (red) el("circle", { r: 21, fill: "none", stroke: "#ff6b5e", "stroke-width": 2, opacity: ".8" }, tok);
      const tx = el("text", { y: -2, class: `numtext ${red ? "red" : ""}` }, tok);
      tx.textContent = t.number;
      const w = (t.pips - 1) * 4.4;
      for (let i = 0; i < t.pips; i++) {
        el("circle", { cx: -w / 2 + i * 4.4, cy: 11, r: 1.6, class: "pip" }, tok);
      }
    }
  }

  for (const p of board.ports) {
    // 桟橋。破線ではなく、杭の上に板を渡した橋として描く（色はその港の資源）
    raw(gPorts, dockShape(p.ax, p.ay, p.x, p.y, p));
    raw(gPorts, dockShape(p.bx, p.by, p.x, p.y, p));
    const g = raw(gPorts, `<g transform="translate(${p.x},${p.y})">${portGraphic(p)}</g>`);
    g.setAttribute("filter", "url(#pieceShadow)");
  }
}

// ---------------------------------------------------------------- 局面の描画

function nodeXY(n) { const p = board.nodes[n]; return [p.x, p.y]; }
function tileXY(t) { const p = board.tiles[t]; return [p.x, p.y]; }
function edgeMid(e) { const g = board.edges[e]; return [(g.x1 + g.x2) / 2, (g.y1 + g.y2) / 2]; }


function drawState() {
  const svg = document.getElementById("board");
  const { gRoads, gBuild, gHints } = svg._layers;
  gRoads.innerHTML = "";
  gBuild.innerHTML = "";
  gHints.innerHTML = "";

  const nowPieces = new Set();

  for (const r of state.roads) {
    const e = board.edges[r.edge];
    const key = `r${r.edge}`;
    nowPieces.add(key);
    const isNew = !lastPieces.has(key);
    const g = raw(gRoads, roadShape(e, PLAYER_COLORS[r.owner], PLAYER_DARK[r.owner],
      isNew ? baseEnd(r) : 0));
    g.setAttribute("class", "road");
    g.setAttribute("data-edge", r.edge);
  }

  for (const b of state.buildings) {
    const [x, y] = nodeXY(b.node);
    const key = `b${b.node}${b.kind}`;
    nowPieces.add(key);
    const isNew = !lastPieces.has(key);
    const shape = b.kind === "SETTLEMENT"
      ? settlementShape(PLAYER_COLORS[b.owner], PLAYER_DARK[b.owner])
      : cityShape(PLAYER_COLORS[b.owner], PLAYER_DARK[b.owner]);
    const g = raw(gBuild, `<g transform="translate(${x},${y})">${shape}</g>`);
    // 開拓地は地面からせり上がり、都市は足場が外れてから現れる。
    // 同じ「ポン」で出すと、建てるのと建て替えるのが同じに見える
    g.setAttribute("class", "piece" + (isNew ? (b.kind === "CITY" ? " grow" : " rise") : ""));
    g.setAttribute("data-node", b.node);
  }

  const wrap = el("g", { id: "robberwrap" }, gBuild);
  const rg = raw(wrap, `<g>${robberBandHtml(state.robber)}</g>`);
  rg.setAttribute("class", "piece");
  // 独占で取り立てに回っている間は、そちらが本体。同じ物を 2 つ出さない
  if (monopolyTrip) wrap.style.display = "none";

  lastPieces = nowPieces;
  if (isHumanTurn()) drawHints(gHints);
}

function drawHints(gHints) {
  const seenNode = new Set(), seenEdge = new Set(), seenTile = new Set();
  // 何を建てるか選んだ後は、その手の置き場所だけを光らせる。
  // 全部を一度に光らせると、どれがどの手なのか分からない。
  //
  // 🔴 絞ってよいのは **通常の建設フェーズだけ**。初期配置・街道建設カードの
  // 無償の道・盗賊は「その手しか無い」場面で、しかも FREE_ROAD は BuildRoad を
  // 出すので、ここで一緒に絞ると盤が光らず**ゲームが進まなくなる**（実際そうなった）。
  const src = pickKind
    ? actionsOf(pickKind)
    : state.prompt === "PLAY_TURN"
    ? state.actions.filter((a) => !MENU_SPATIAL.has(a.kind))
    : state.actions;
  const many = src.filter((a) => a.node !== undefined).length > 12;
  const spotR = many ? 8 : 11;
  for (const a of src) {
    if (a.node !== undefined && !seenNode.has(a.node)) {
      seenNode.add(a.node);
      const [x, y] = nodeXY(a.node);
      const c = el("circle", { cx: x, cy: y, r: spotR, class: "spot" + (many ? " faint" : " pulse") }, gHints);
      c.addEventListener("click", () => play(a.i));
      c.appendChild(title(a.label));
    } else if (a.edge !== undefined && !seenEdge.has(a.edge)) {
      seenEdge.add(a.edge);
      const e = board.edges[a.edge];
      const t = 0.26;
      const l = el("line", {
        x1: e.x1 + (e.x2 - e.x1) * t, y1: e.y1 + (e.y2 - e.y1) * t,
        x2: e.x2 - (e.x2 - e.x1) * t, y2: e.y2 - (e.y2 - e.y1) * t,
        class: "spot-edge pulse",
      }, gHints);
      l.addEventListener("click", () => play(a.i));
      l.appendChild(title(a.label));
    } else if (a.tile !== undefined && !seenTile.has(a.tile)) {
      seenTile.add(a.tile);
      const [x, y] = tileXY(a.tile);
      // 点線の輪郭は地形の絵に紛れて見づらい。
      // **数字チップを光る輪で囲って**、そこを見れば分かるようにする
      const g = el("g", { class: "tilehint" }, gHints);
      el("polygon", { points: hexPoints(x, y, board.hexSize * 0.9), class: "spot-tile" }, g);
      // 下に暗い縁を敷いてから明るい輪を重ねる。
      // 1 本だけだと、明るいヘクスの上と暗いヘクスの上で見え方が変わって
      // 全体がまばらに光っているように見える
      el("circle", { cx: x, cy: y - 6, r: 25, class: "spot-token-bg" }, g);
      el("circle", { cx: x, cy: y - 6, r: 25, class: "spot-token" }, g);
      g.addEventListener("click", () => selectTile(a.tile));
      g.appendChild(title("ここへ盗賊を動かす"));
    }
  }
}

function title(text) {
  const t = document.createElementNS(SVG_NS, "title");
  t.textContent = text;
  return t;
}

// ---------------------------------------------------------------- 演出

function fxLayer() { return document.getElementById("board")._layers.gFx; }
function fxAdd(svgText, cls) {
  const g = raw(fxLayer(), svgText);
  if (cls) g.setAttribute("class", cls);
  return g;
}
function fxRemove(g, ms) { setTimeout(() => g.remove(), ms); }

/** CSS の transform で動かす（SVG 属性の transform とは別枠なので競合しない） */
function fxMove(g, fromX, fromY, toX, toY, ms, easing) {
  g.style.transition = "none";
  g.style.transform = `translate(${fromX - toX}px, ${fromY - toY}px)`;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    g.style.transition = `transform ${ms}ms ${easing || "cubic-bezier(.35,1.25,.5,1)"}`;
    g.style.transform = "translate(0,0)";
  }));
}

/**
 * 盗賊の引っ越し。**なぜ動かされたか**で足取りが変わる。
 *
 *   騎士で追い出された → 慌てて蛇行しながら逃げる（汗をかく）
 *   7 が出て動かされた → 次の獲物へ勇んで向かう（笑みを浮かべて弾む）
 *
 * どちらも「移った」ことだけ分かればいい速さでは足りない。
 * 盤の上で一番大きな出来事なので、目で追える速さまで落とす。
 */
function animRobberMove(from, to, byKnight) {
  stopSiege();
  robberEverMoved = true;
  const wrap = document.getElementById("robberwrap");
  if (!wrap || from === to) return;
  const [fx, fy] = tileXY(from);
  const [tx, ty] = tileXY(to);
  const dx = tx - fx, dy = ty - fy;
  const L = Math.hypot(dx, dy) || 1;
  const nx = -dy / L, ny = dx / L; // 進む向きに直交＝横に振る向き

  // 距離なりに伸ばす。近所へ移るのと盤の端まで行くのが同じ時間だと嘘になる
  const base = byKnight ? 1450 : 1850;
  const MS = Math.round(base * Math.min(1.5, Math.max(0.75, L / 110)));
  const N = 14;
  const frames = [];
  for (let i = 0; i <= N; i++) {
    const t = i / N;
    let ox, oy;
    if (byKnight) {
      // 逃げ足。横に細かく振れて、着く頃に落ち着く
      ox = nx * Math.sin(t * Math.PI * 3.4) * 9 * (1 - t * 0.75);
      oy = ny * Math.sin(t * Math.PI * 3.4) * 9 * (1 - t * 0.75) - Math.sin(t * Math.PI) * 5;
    } else {
      // 勇んで弾む。横振れは浅く、代わりに 3 回跳ねる
      ox = nx * Math.sin(t * Math.PI * 1.6) * 4;
      oy = ny * Math.sin(t * Math.PI * 1.6) * 4 - Math.abs(Math.sin(t * Math.PI * 3)) * 7;
    }
    frames.push({
      transform: `translate(${(fx + dx * t + ox - tx).toFixed(1)}px, ${(fy + dy * t + oy - ty).toFixed(1)}px)`,
      offset: t,
    });
  }
  wrap.animate(frames, {
    duration: MS,
    easing: byKnight ? "cubic-bezier(.15,.75,.4,1)" : "cubic-bezier(.4,.05,.5,1)",
    fill: "both",
  });
  const mood = byKnight ? "fleeing" : "eager";
  wrap.classList.add(mood);
  setTimeout(() => wrap.classList.remove(mood), MS + 150);
}

function animSteal(L) {
  if (!L.stolen || L.victimNode === null) return;
  const [sx, sy] = nodeXY(L.victimNode);
  const [tx, ty] = tileXY(L.tile);
  const t = TERRAIN[L.stolen];
  const g = fxAdd(`
    <g transform="translate(${tx},${ty - 10})">
      <rect x="-14" y="-19" width="28" height="38" rx="6" fill="${t.b}" stroke="${t.light}" stroke-width="1.8"/>
      <g transform="scale(.85)">${smallGlyph(L.stolen)}</g>
    </g>`);
  fxMove(g, sx, sy - 14, tx, ty - 10, 700, "cubic-bezier(.3,.9,.4,1)");
  setTimeout(() => { g.style.transition = "opacity .35s"; g.style.opacity = "0"; }, 720);
  fxRemove(g, 1150);
}

/* ------------------------------------------------- 独占：盗賊が取り立てに回る */

/** 盗んだ物を入れる袋。盗賊の絵で何度も使うので切り出す */
function sackShape(s = 1) {
  return `<g transform="scale(${s})">
    <path d="M-4.4,0 Q0,-7.2 4.4,0 Q4.4,6.1 0,6.1 Q-4.4,6.1 -4.4,0 Z"
          fill="#5c4a30" stroke="#2a2114" stroke-width="1.3" stroke-linejoin="round"/>
    <path d="M-2.3,-2.5 L2.3,-2.5" stroke="#2a2114" stroke-width="1.3" stroke-linecap="round"/>
  </g>`;
}

/**
 * 取り立ての一撃。**盗んでいる**ことを、渡す場面とは別の見た目で出す。
 *
 * 火花だけだと「何かした」しか伝わらず、渡す場面と区別が付かない。
 * 建物を揺らし、赤い斬線を引き、中身が引きずり出されて袋に消える、の 3 つで見せる。
 */
function theftFx(node, x, y) {
  const b = document.querySelector(`[data-node="${node}"]`);
  if (b) {
    b.classList.add("robbedhit");
    setTimeout(() => b.classList.remove("robbedhit"), 460);
  }
  // 引っ掻いた跡
  const slash = fxAdd(`<g transform="translate(${x},${y - 6})">
    <path d="M-13,-9 L13,9" stroke="#ff6a52" stroke-width="3.4" stroke-linecap="round"/>
    <path d="M-13,4 L13,-4" stroke="#ff9d8c" stroke-width="2.4" stroke-linecap="round" opacity=".8"/>
  </g>`);
  slash.firstElementChild.animate(
    [{ opacity: 0, transform: "scale(.4)" }, { opacity: 1, transform: "scale(1.05)", offset: 0.3 }, { opacity: 0, transform: "scale(1.3)" }],
    { duration: 340, easing: "ease-out", fill: "forwards" }
  );
  fxRemove(slash, 420);

  // 中身が引きずり出されて、袋の方へ吸い込まれる
  const loot = fxAdd(`<g transform="translate(${x},${y - 4})">${sackShape(0.62)}</g>`);
  loot.firstElementChild.animate(
    [
      { transform: "translate(0px,0px) scale(.5)", opacity: 0 },
      { transform: "translate(2px,-15px) scale(1.05)", opacity: 1, offset: 0.35 },
      { transform: "translate(6px,4px) scale(.35)", opacity: 0 },
    ],
    { duration: 420, easing: "cubic-bezier(.3,.7,.4,1)", fill: "forwards" }
  );
  fxRemove(loot, 500);
}

/**
 * 引き渡し。**どっさり置いていく**ところを見せる。
 *
 * 盗む時と同じ火花で済ませると、最後の 1 軒にも押し入ったように見える。
 * 袋を実際に落として積み、金色に開く ── 向きが逆であることを絵で分ける。
 */
function deliverFx(node, x, y, count) {
  const b = document.querySelector(`[data-node="${node}"]`);
  if (b) {
    b.classList.add("received");
    setTimeout(() => b.classList.remove("received"), 700);
  }
  const n = Math.max(3, Math.min(12, count));
  for (let i = 0; i < n; i++) {
    const dx = ((i % 5) - 2) * 10 + (i % 2 ? 3 : -3);
    const dy = 12 + Math.floor(i / 5) * 5;
    const g = fxAdd(`<g transform="translate(${x},${y - 10})">${sackShape(0.8)}</g>`);
    const inner = g.firstElementChild;
    inner.animate(
      [
        { transform: "translate(0px,-6px) scale(.6) rotate(0deg)", opacity: 0 },
        { transform: `translate(${(dx * 0.5).toFixed(1)}px,-20px) scale(1) rotate(${dx > 0 ? 45 : -45}deg)`, opacity: 1, offset: 0.35 },
        { transform: `translate(${dx}px,${dy}px) scale(1) rotate(${dx > 0 ? 110 : -110}deg)`, opacity: 1, offset: 0.8 },
        { transform: `translate(${dx}px,${dy}px) scale(1.25) rotate(${dx > 0 ? 120 : -120}deg)`, opacity: 0 },
      ],
      { duration: 760, delay: i * 65, easing: "cubic-bezier(.25,.7,.35,1)", fill: "both" }
    );
    setTimeout(() => coinPop(x + dx, y + dy), 620 + i * 65);
    fxRemove(g, 900 + i * 65);
  }
  // 受け取った印。金色の輪が広がる
  const ring = fxAdd(`<g transform="translate(${x},${y})">
    <circle r="10" fill="none" stroke="#ffd166" stroke-width="3"/>
  </g>`);
  ring.firstElementChild.animate(
    [{ transform: "scale(.4)", opacity: 0.95 }, { transform: "scale(3.4)", opacity: 0 }],
    { duration: 760, delay: 260, easing: "cubic-bezier(.2,.8,.3,1)", fill: "both" }
  );
  fxRemove(ring, 1100);
}

/** 落ちた袋がはじけて金貨になる */
function coinPop(x, y) {
  const g = fxAdd(`<g transform="translate(${x},${y})"></g>`);
  const at = g.firstElementChild;
  for (let i = 0; i < 4; i++) {
    const a = (Math.PI * (i + 0.5)) / 4;
    const c = document.createElementNS(SVG_NS, "circle");
    c.setAttribute("r", "2.6");
    c.setAttribute("fill", "#ffd166");
    c.setAttribute("stroke", "#8a5a04");
    c.setAttribute("stroke-width", "0.8");
    at.appendChild(c);
    c.animate(
      [
        { transform: "translate(0px,0px) scale(.4)", opacity: 1 },
        { transform: `translate(${(-Math.cos(a) * 13).toFixed(1)}px, ${(-Math.sin(a) * 11 - 2).toFixed(1)}px) scale(1)`, opacity: 0 },
      ],
      { duration: 420, easing: "cubic-bezier(.2,.8,.3,1)", fill: "forwards" }
    );
  }
  fxRemove(g, 520);
}

/** 取り立てに回る一団。散らばらず、固まって歩く */
function marchBandHtml() {
  const hood = (x, y, sc, d) => `<g transform="translate(${x},${y}) scale(${sc})">
      <g class="march" style="--mdelay:${d}s">${banditShadow(11, 10)}${banditCloak()}${banditEyes}</g>
    </g>`;
  return hood(-13, 5, 0.64, 0.12) + hood(13, 3, 0.6, 0.24) + hood(0, 0, 0.76, 0) +
    `<g class="loot" transform="translate(0,4)"></g>`;
}

/** 取り立て中は本物の盗賊を隠す（同じ物が 2 つ盤に出ないように） */
let monopolyTrip = false;

/**
 * 独占。盗賊が **他の全員の拠点を回って取り立て**、使った人の拠点へ全部渡し、
 * 元いたマスへ帰る。
 *
 * 「全員から 1 種類を巻き上げる」という札の効きが、盤の上のどこで起きたのかを見せる。
 * 足は速くしない ── 立ち寄った先が読み取れないと、回っている意味が無くなる。
 */
function animMonopoly(actor) {
  if (monopolyTrip) return;
  const home = tileXY(state.robber);

  // 回る先。**自分以外の拠点を全部**、いま居る所から近い順に辿る
  const left = state.buildings.filter((b) => b.owner !== actor);
  const stops = [];
  let cur = home;
  while (left.length) {
    let bi = 0, bd = Infinity;
    left.forEach((b, i) => {
      const [x, y] = nodeXY(b.node);
      const d = Math.hypot(x - cur[0], y - cur[1]);
      if (d < bd) { bd = d; bi = i; }
    });
    const b = left.splice(bi, 1)[0];
    cur = nodeXY(b.node);
    stops.push({ p: cur, owner: b.owner, node: b.node });
  }
  // 渡す先。使った人の拠点のうち、最後に回った所から一番近いもの
  const mine = state.buildings.filter((b) => b.owner === actor);
  if (!mine.length) return;
  let drop = null, dropNode = null, dd = Infinity;
  for (const b of mine) {
    const [x, y] = nodeXY(b.node);
    const d = Math.hypot(x - cur[0], y - cur[1]);
    if (d < dd) { dd = d; drop = [x, y]; dropNode = b.node; }
  }
  if (!stops.length) return;

  monopolyTrip = true;
  const wrap = document.getElementById("robberwrap");
  if (wrap) wrap.style.display = "none";
  const g = fxAdd(`<g transform="translate(${home[0]},${home[1]})">${marchBandHtml()}</g>`, "trip");

  // 道のり: 元の場所 → 取り立て先 → 渡す先 → 元の場所
  const route = [home, ...stops.map((s) => s.p), drop, home];
  const SPEED = 0.16; // 単位/ms
  const HOLD = 210;        // 取り立て先で止まる時間
  const DROP_HOLD = 1000;  // 渡す所。ここは見せ場なので長く止まる
  const frames = [];
  const at = [];
  let t = 0;
  for (let i = 0; i < route.length; i++) {
    const p = route[i];
    if (i) {
      const q = route[i - 1];
      t += Math.min(1100, Math.max(220, Math.hypot(p[0] - q[0], p[1] - q[1]) / SPEED));
    }
    at.push(t);
    frames.push({ p, t });
    // 途中の立ち寄り先では立ち止まる。渡す所は「どっさり置く」ぶん長く取る
    if (i > 0 && i < route.length - 1) {
      t += i === route.length - 2 ? DROP_HOLD : HOLD;
      frames.push({ p, t });
    }
  }
  // 拠点が多いと一周が長くなりすぎる。全体で頭打ちにして、その分だけ足を速める
  const CAP = 7600;
  const k = t > CAP ? CAP / t : 1;
  if (k < 1) {
    for (const f of frames) f.t *= k;
    for (let i = 0; i < at.length; i++) at[i] *= k;
    t *= k;
  }
  const total = t;
  const kf = frames.map((f) => ({
    transform: `translate(${(f.p[0] - home[0]).toFixed(1)}px, ${(f.p[1] - home[1]).toFixed(1)}px)`,
    offset: Math.min(1, f.t / total),
  }));
  for (let i = 1; i < kf.length; i++)
    if (kf[i].offset <= kf[i - 1].offset) kf[i].offset = Math.min(1, kf[i - 1].offset + 0.0005);
  g.animate(kf, { duration: total, easing: "linear", fill: "both" });

  // 立ち寄るたびに袋が 1 つ増える
  const loot = g.querySelector(".loot");
  stops.forEach((st, i) => {
    setTimeout(() => {
      theftFx(st.node, st.p[0], st.p[1]);
      // 拠点の数だけ増えるので、4 つずつ段に積んで嵩を抑える
      const x = -13 + (i % 4) * 9;
      const y = -2 - Math.floor(i / 4) * 7;
      raw(loot, `<g class="sack" transform="translate(${x},${y})">${sackShape(0.72)}</g>`);
    }, at[i + 1] + 120);
  });

  // 渡す先で全部置いていく
  const dropAt = at[at.length - 2];
  setTimeout(() => {
    const carried = loot.childElementCount;
    loot.innerHTML = "";
    dustBurst(drop[0], drop[1] + 10, 9, 26);
    deliverFx(dropNode, drop[0], drop[1], carried);
    // 自分が使った札なら、そのまま手札へ流れ込む
    if (actor === mySeat) {
      const e = state.log && state.log[0];
      const gain = (e && e.actor === actor && e.gain) || [];
      let k = 0;
      gain.forEach((n, i) => {
        for (let j = 0; j < Math.min(n, 8); j++)
          flyCard(
            svgRect(drop[0], drop[1]),
            document.querySelector(`#handbar .hcard.${RES[i]}`) || document.getElementById("handbar"),
            `<div class="fcard ${RES[i]}">${glyphHtml(RES[i], 26)}</div>`,
            k++ * 80
          );
      });
    }
  }, dropAt + 200);

  // 帰り着いたら本物と入れ替える
  setTimeout(() => {
    monopolyTrip = false;
    const w = document.getElementById("robberwrap");
    if (w) w.style.display = "";
    g.style.transition = "opacity .3s";
    g.style.opacity = "0";
  }, total + 120);
  fxRemove(g, total + 700);
}

/* ------------------------------------------------- 騎士と盗賊の攻防 */

/** 剣を振れるように、腕（剣）を別の `<g>` に分けた騎士 */
function knightFighterShape(color) {
  return `
    <ellipse cx="0" cy="13" rx="9" ry="3" fill="#04080c" opacity=".38"/>
    <path d="M0,-12 L9,-7 V2 Q9,9 0,13 Q-9,9 -9,2 V-7 Z" fill="${color}" stroke="#050b11" stroke-width="1.4"/>
    <path d="M0,-6 L0,7 M-4.5,0 L4.5,0" stroke="#fff" stroke-width="2" opacity=".85"/>
    <g class="sword">
      <path d="M7,-6 L20,-19" stroke="#dfe7ef" stroke-width="3.2" stroke-linecap="round"/>
      <path d="M4.4,-3.4 L9.6,-8.6" stroke="#8a6a3a" stroke-width="3" stroke-linecap="round"/>
    </g>`;
}

/** 遠くの盗賊へは大砲を撃つ。走って行ける距離ではないので */
function cannonShape(color) {
  return `
    <ellipse cx="2" cy="9" rx="14" ry="3.2" fill="#04080c" opacity=".35"/>
    <path d="M-11,6 L7,6 L3,-2 L-7,-2 Z" fill="#6b4a22" stroke="#2f2010" stroke-width="1.2" stroke-linejoin="round"/>
    <g class="barrel">
      <rect x="-5" y="-5" width="27" height="9.4" rx="4.6" fill="#39434f" stroke="#8e9dad" stroke-width="1.1"/>
      <circle cx="21" cy="-0.3" r="5" fill="#2a333d" stroke="#8e9dad" stroke-width="1.1"/>
    </g>
    <circle cx="-3" cy="7" r="4.8" fill="${color}" stroke="#050b11" stroke-width="1.3"/>
    <circle cx="-3" cy="7" r="1.6" fill="#050b11"/>`;
}

/** 打ち合いの火花。線を短く散らすだけで「当たった」ことが伝わる */
function sparkBurst(x, y) {
  const g = fxAdd(`<g transform="translate(${x},${y})"></g>`);
  const at = g.firstElementChild;
  for (let i = 0; i < 5; i++) {
    const a = (Math.PI * 2 * i) / 5 + Math.random();
    const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
    p.setAttribute("d", `M0,0 L${(Math.cos(a) * 9).toFixed(1)},${(Math.sin(a) * 9).toFixed(1)}`);
    p.setAttribute("stroke", "#ffe08a");
    p.setAttribute("stroke-width", "2");
    p.setAttribute("stroke-linecap", "round");
    at.appendChild(p);
  }
  at.animate([{ opacity: 1, transform: "scale(.4)" }, { opacity: 0, transform: "scale(1.5)" }],
    { duration: 320, easing: "ease-out", fill: "forwards" });
  fxRemove(g, 400);
}

/** 白兵。拠点から走り出て、ヘクスの縁で盗賊と斬り結ぶ */
function knightCharge(actor, from, cx, cy) {
  const dx = from[0] - cx, dy = from[1] - cy;
  const L = Math.hypot(dx, dy) || 1;
  const tx = cx + (dx / L) * 55.16 * 0.66;
  const ty = cy + (dy / L) * 55.16 * 0.66;
  const flip = dx > 0 ? -1 : 1; // 常にヘクスの中（＝盗賊）を向く
  const g = fxAdd(
    `<g transform="translate(${tx.toFixed(1)},${ty.toFixed(1)}) scale(${flip},1)">${knightFighterShape(PLAYER_COLORS[actor])}</g>`,
    "knight"
  );
  fxMove(g, from[0], from[1], tx, ty, 420, "cubic-bezier(.3,.1,.4,1)");
  setTimeout(() => g.classList.add("fighting"), 420);
  // 斬り結ぶ音の代わりに火花。剣の先あたりで散らす
  const clash = setInterval(() => sparkBurst(cx + (dx / L) * 34, cy + (dy / L) * 34), 460);
  setTimeout(() => {
    clearInterval(clash);
    g.classList.remove("fighting");
    g.style.transition = "transform .45s ease-in, opacity .45s";
    g.style.transform = `translate(${(from[0] - tx).toFixed(1)}px, ${(from[1] - ty).toFixed(1)}px)`;
    g.style.opacity = "0";
  }, 2100);
  fxRemove(g, 2700);
}

/** 遠距離。拠点に大砲を据えて撃ち込む */
function cannonFire(actor, from, cx, cy) {
  const ang = (Math.atan2(cy - from[1], cx - from[0]) * 180) / Math.PI;
  const g = fxAdd(
    `<g transform="translate(${from[0]},${from[1]}) rotate(${ang.toFixed(1)})">${cannonShape(PLAYER_COLORS[actor])}</g>`,
    "cannon"
  );
  setTimeout(() => {
    g.classList.add("fire");
    // 砲口の火。砲身の先に一瞬だけ
    const fl = fxAdd(
      `<g transform="translate(${from[0]},${from[1]}) rotate(${ang.toFixed(1)})">
        <path d="M20,0 L34,-7 L30,0 L34,7 Z" fill="#ffd166"/>
        <circle cx="22" cy="0" r="5.4" fill="#fff2c4"/>
      </g>`
    );
    fl.firstElementChild.animate(
      [{ opacity: 1, transform: "scale(.6)" }, { opacity: 0, transform: "scale(1.35)" }],
      { duration: 220, easing: "ease-out", fill: "forwards" }
    );
    fxRemove(fl, 280);
    // 砲弾。着弾したら土埃と、盗賊がのけぞる
    const b = fxAdd(`<g transform="translate(${cx},${cy})"><circle r="4.4" fill="#20262e" stroke="#8e9dad" stroke-width="1.2"/></g>`);
    fxMove(b, from[0], from[1], cx, cy, 380, "cubic-bezier(.2,.6,.6,1)");
    setTimeout(() => {
      b.remove();
      dustBurst(cx, cy + 6, 9, 26);
      sparkBurst(cx, cy);
      const wrap = document.getElementById("robberwrap");
      if (wrap) { wrap.classList.add("shoved"); setTimeout(() => wrap.classList.remove("shoved"), 700); }
    }, 390);
  }, 420);
  setTimeout(() => { g.style.transition = "opacity .4s"; g.style.opacity = "0"; }, 1900);
  fxRemove(g, 2400);
}

/**
 * 騎士を出したときの攻防。
 *
 * 盗賊をどこへ動かすか決めるまで戦いは続く。**迷うほど増援が出る**ので、
 * 待ち時間そのものが演出になる。近い拠点からは騎士が走り、
 * 走って行けない距離の拠点からは大砲を撃つ。
 */
let siege = null;

function stopSiege() {
  if (!siege) return;
  clearInterval(siege.timer);
  siege = null;
  const wrap = document.getElementById("robberwrap");
  if (wrap) wrap.classList.remove("besieged");
}

function siegeWave(actor) {
  const [cx, cy] = tileXY(state.robber);
  const mine = state.buildings.filter((b) => b.owner === actor);
  if (!mine.length) return;
  const sorted = mine
    .map((b) => ({ b, p: nodeXY(b.node), d: 0 }))
    .map((o) => ({ ...o, d: Math.hypot(o.p[0] - cx, o.p[1] - cy) }))
    .sort((a, b) => a.d - b.d);
  // まだ出していない拠点を優先。全部出したら近い方から出し直す
  const next = sorted.find((o) => !siege.used.has(o.b.node)) || sorted[0];
  siege.used.add(next.b.node);
  if (next.d <= 132) knightCharge(actor, next.p, cx, cy);
  else cannonFire(actor, next.p, cx, cy);
}

function animKnight(actor) {
  stopSiege();
  const wrap = document.getElementById("robberwrap");
  if (wrap) wrap.classList.add("besieged");
  siege = { actor, used: new Set(), timer: 0 };
  siegeWave(actor);
  siege.timer = setInterval(() => {
    // 置き場所が決まった（＝勝負が付いた）ら止める
    if (!state || state.prompt !== "MOVE_ROBBER" || state.winner !== null) { stopSiege(); return; }
    siegeWave(actor);
  }, 1900);
}

/**
 * 建てる人。**自分の道を辿って**現場まで来て、しばらく打ってから帰る。
 *
 * 直線で飛んで来ると「道の先に建てた」ことが絵で分からない。
 * 戻り値は現場に着くまでの時間で、建物側の演出（土煙・せり上がり）を
 * これに合わせて遅らせる。
 */
function animBuildAt(actor, node, workMs = 520) {
  const pts = trimPts(ownRoadPath(actor, node, { exclude: node }).map(nodeXY));
  const last = pts[pts.length - 1];
  const g = fxAdd(
    `<g transform="translate(${last[0]},${last[1]})">${workerShape(PLAYER_COLORS[actor])}</g>`,
    "worker"
  );

  let walkMs = 0;
  if (pts.length >= 2) {
    const seg = [];
    for (let i = 1; i < pts.length; i++)
      seg.push(Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]));
    const walkLen = seg.reduce((a, b) => a + b, 0);
    walkMs = Math.min(1200, Math.round(walkLen / WALK_SPEED));
    let acc = 0;
    const frames = pts.map((p, i) => {
      if (i) acc += seg[i - 1];
      return {
        transform: `translate(${(p[0] - last[0]).toFixed(1)}px, ${(p[1] - last[1]).toFixed(1)}px)`,
        offset: walkLen ? Math.min(1, acc / walkLen) : i === 0 ? 0 : 1,
      };
    });
    for (let i = 1; i < frames.length; i++)
      if (frames[i].offset <= frames[i - 1].offset)
        frames[i].offset = Math.min(1, frames[i - 1].offset + 0.001);
    g.animate(frames, { duration: walkMs, easing: "linear", fill: "both" });
  }

  setTimeout(() => g.classList.add("laying"), walkMs);
  setTimeout(() => {
    g.classList.remove("laying");
    // 来た道を戻って消える
    if (pts.length >= 2) {
      const home = pts[0];
      g.style.transition = `transform ${Math.max(240, walkMs)}ms ease-in, opacity .4s ${Math.max(0, walkMs - 400)}ms`;
      g.style.transform = `translate(${(home[0] - last[0]).toFixed(1)}px, ${(home[1] - last[1]).toFixed(1)}px)`;
    } else {
      g.style.transition = "opacity .34s";
    }
    g.style.opacity = "0";
  }, walkMs + workMs);
  fxRemove(g, walkMs * 2 + workMs + 600);
  return walkMs;
}

/** 建物側の演出を、作業員が着く時刻に合わせる */
function setPieceDelay(node, ms) {
  const el = document.querySelector(`[data-node="${node}"]`);
  if (el) el.style.animationDelay = `${ms}ms`;
}

/**
 * その人の道だけを辿って、拠点から目的の地点まで来る道順（ノードの列）。
 *
 * 拠点から離れた所に道や建物を足すとき、直線で飛ばすと
 * 「道を無視して現れた」ように見える。自分の道のつながりを幅優先で辿る。
 * `skipEdge` は今から敷く道（まだ通れない）、`exclude` は出発点にしないノード
 * （都市への建て替えでは、その場に建っている自分の開拓地から出ても意味が無い）。
 */
function ownRoadPath(actor, target, { skipEdge = -1, exclude = -1 } = {}) {
  const mine = state.roads.filter((r) => r.owner === actor && r.edge !== skipEdge);
  const adj = new Map();
  for (const r of mine) {
    const e = board.edges[r.edge];
    if (!adj.has(e.n1)) adj.set(e.n1, []);
    if (!adj.has(e.n2)) adj.set(e.n2, []);
    adj.get(e.n1).push(e.n2);
    adj.get(e.n2).push(e.n1);
  }
  const home = new Set(
    state.buildings.filter((b) => b.owner === actor && b.node !== exclude).map((b) => b.node)
  );
  if (home.has(target) || !adj.has(target)) return [target];

  const prev = new Map([[target, null]]);
  const q = [target];
  let goal = null;
  while (q.length) {
    const n = q.shift();
    if (home.has(n)) { goal = n; break; }
    for (const m of adj.get(n) || []) {
      if (prev.has(m)) continue;
      prev.set(m, n);
      q.push(m);
    }
  }
  if (goal === null) return [target];
  // goal → target の順に並べ直す（歩く向き）
  const path = [];
  for (let n = goal; n !== null; n = prev.get(n)) path.push(n);
  return path;
}

/** 同じ点が続くとキーフレームの時刻が重なるので落とす */
function trimPts(pts) {
  return pts.filter((p, i) => i === 0 || Math.hypot(p[0] - pts[i - 1][0], p[1] - pts[i - 1][1]) > 0.5);
}

/** 歩く速さ。単位/ms（ヘクスの一辺およそ 55 を 290ms で歩く） */
const WALK_SPEED = 0.19;

/**
 * 道づくり。
 *
 * ①自分の道を辿って現場まで歩く → ②歩いた分だけ道が敷かれていく → ③土埃を上げて帰る。
 * 道が出来始めるのは作業員が着いてからなので、道の側のアニメーションには
 * 歩いた時間ぶんの待ちを入れる。
 */
function animRoadBuild(L) {
  const e = board.edges[L.edge];
  const be = baseEnd({ edge: L.edge, owner: L.actor });
  const baseNode = be === 1 ? e.n1 : e.n2;
  const farNode = be === 1 ? e.n2 : e.n1;

  const pts = ownRoadPath(L.actor, baseNode, { skipEdge: L.edge }).map(nodeXY);
  pts.push(nodeXY(farNode));
  const uniq = trimPts(pts);
  if (uniq.length < 2) return;

  const seg = [];
  for (let i = 1; i < uniq.length; i++) seg.push(Math.hypot(uniq[i][0] - uniq[i - 1][0], uniq[i][1] - uniq[i - 1][1]));
  const walkLen = seg.slice(0, -1).reduce((a, b) => a + b, 0);
  const walkMs = Math.min(1200, Math.round(walkLen / WALK_SPEED));
  const layMs = 640;

  // 道の側は、作業員が着いてから敷き始める
  const road = document.querySelector(`[data-edge="${L.edge}"] .roadgrow`);
  if (road) {
    road.style.animationDelay = `${walkMs}ms`;
    road.style.animationDuration = `${layMs}ms`;
  }

  // 作業員は最後の点に置いて、そこからの相対で動かす
  const last = uniq[uniq.length - 1];
  const g = fxAdd(`<g transform="translate(${last[0]},${last[1]})">${workerShape(PLAYER_COLORS[L.actor])}</g>`, "worker");

  const times = [0];
  for (let i = 0; i < seg.length; i++) {
    const isLay = i === seg.length - 1;
    times.push(times[i] + (isLay ? layMs : walkLen ? (seg[i] / walkLen) * walkMs : 0));
  }
  const total = times[times.length - 1];
  const frames = uniq.map((p, i) => ({
    transform: `translate(${(p[0] - last[0]).toFixed(1)}px, ${(p[1] - last[1]).toFixed(1)}px)`,
    offset: Math.min(1, times[i] / total),
  }));
  // 時刻が重なると WAAPI が受け付けない。わずかにずらす
  for (let i = 1; i < frames.length; i++)
    if (frames[i].offset <= frames[i - 1].offset) frames[i].offset = Math.min(1, frames[i - 1].offset + 0.001);
  g.animate(frames, { duration: total, easing: "linear", fill: "both" });

  // 着いたらハンマーを振り始める
  setTimeout(() => g.classList.add("laying"), walkMs);
  sfx.play("road", walkMs);
  setTimeout(() => {
    const [px, py] = last;
    const puff = fxAdd(`
      <g transform="translate(${px},${py + 6})">
        <circle cx="-9" cy="0" r="5" fill="#cfe6f2" opacity=".65"/>
        <circle cx="0" cy="-3" r="7" fill="#cfe6f2" opacity=".5"/>
        <circle cx="9" cy="0" r="5" fill="#cfe6f2" opacity=".65"/>
      </g>`, "puff");
    fxRemove(puff, 700);
    g.classList.remove("laying");
    g.style.transition = "opacity .34s";
    g.style.opacity = "0";
  }, total);
  fxRemove(g, total + 500);
}

/** 土埃。粒をばらけさせて外へ逃がす（同じ形が並ぶと「模様」に見えてしまう） */
function dustBurst(x, y, n = 7, spread = 20) {
  const g = fxAdd(`<g transform="translate(${x},${y})"></g>`);
  const at = g.firstElementChild; // 位置を持っているのは内側
  for (let i = 0; i < n; i++) {
    const a = (Math.PI * (i + 0.5)) / n; // 上向きの半円に散らす
    const r = spread * (0.55 + (i % 3) * 0.22);
    const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    c.setAttribute("r", (2.6 + (i % 3) * 1.4).toFixed(1));
    c.setAttribute("fill", "#d9cdb4");
    c.setAttribute("class", "dust");
    at.appendChild(c);
    c.animate(
      [
        { transform: "translate(0px,0px) scale(.5)", opacity: 0.75 },
        { transform: `translate(${(-Math.cos(a) * r).toFixed(1)}px, ${(-Math.sin(a) * r * 0.55 + 2).toFixed(1)}px) scale(1.3)`, opacity: 0 },
      ],
      { duration: 520 + i * 40, easing: "cubic-bezier(.2,.7,.3,1)", fill: "forwards" }
    );
  }
  fxRemove(g, 900);
  return g;
}

/**
 * 土煙。開拓地は**この煙の中から現れる**ので、粒ではなく塊で出す。
 * 地を這う大きな塊＋跳ね上がる細かい粒の 2 層。
 */
function dustCloud(x, y) {
  const g = fxAdd(`<g transform="translate(${x},${y})"></g>`);
  const at = g.firstElementChild;
  const blob = (dx, dy, r0, r1, ms, op) => {
    const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    c.setAttribute("r", r0.toFixed(1));
    c.setAttribute("fill", "#ded2b8");
    c.setAttribute("class", "dust");
    at.appendChild(c);
    c.animate(
      [
        { transform: "translate(0px,0px) scale(.45)", opacity: op },
        { transform: `translate(${dx.toFixed(1)}px, ${dy.toFixed(1)}px) scale(${(r1 / r0).toFixed(2)})`, opacity: 0 },
      ],
      { duration: ms, easing: "cubic-bezier(.15,.75,.3,1)", fill: "forwards" }
    );
  };
  // 地を這う塊。左右へ広く、少しだけ上へ
  for (let i = 0; i < 11; i++) {
    const dir = i % 2 ? 1 : -1;
    const t = (i + 1) / 11;
    blob(dir * (10 + t * 22), -2 - t * 9, 6 + (i % 4) * 2.4, 13 + (i % 3) * 4, 620 + i * 45, 0.85);
  }
  // 跳ね上がる細かい粒
  for (let i = 0; i < 7; i++) {
    const a = (Math.PI * (i + 0.5)) / 7;
    blob(-Math.cos(a) * 22, -Math.sin(a) * 19 - 3, 2.2, 4.5, 520 + i * 50, 0.7);
  }
  fxRemove(g, 1200);
  return g;
}

/** 都市の工事用の足場。丸太 2 本に足場板とはしご */
function scaffoldShape() {
  const post = (x) => `<rect x="${x - 1.5}" y="-22" width="3" height="30" rx="1.4"
      fill="#a5762f" stroke="#5a3d14" stroke-width="1"/>`;
  const plank = (y) => `<rect x="-14" y="${y}" width="28" height="2.6" rx="1.2"
      fill="#c99348" stroke="#5a3d14" stroke-width=".8"/>`;
  return `${post(-12)}${post(12)}${plank(-16)}${plank(-6)}${plank(4)}
    <path d="M-4,8 L-4,-14 M4,8 L4,-14" stroke="#8a5f22" stroke-width="1.6"/>
    <path d="M-4,3 L4,3 M-4,-3 L4,-3 M-4,-9 L4,-9" stroke="#8a5f22" stroke-width="1.4"/>`;
}

/**
 * 都市への建て替え。**足場を組んでから**大きくする。
 * 開拓地がそのまま膨らむだけだと「建て替えた」ことが伝わらない。
 */
function animCityUpgrade(node, delay = 0) {
  const [x, y] = nodeXY(node);
  const sc = fxAdd(`<g transform="translate(${x},${y})">${scaffoldShape()}</g>`, "scaffold");
  sc.style.animationDelay = `${delay}ms`;
  setTimeout(() => {
    sc.classList.add("off");
    dustBurst(x, y + 8, 8, 22);
  }, delay + 780);
  fxRemove(sc, delay + 1250);
}

function runAnimations() {
  const L = state.last;
  if (!L || !L.seq || L.seq === lastSeq) return;
  lastSeq = L.seq;
  switch (L.kind) {
    case "MOVE_ROBBER":
      // siege が生きている＝直前に騎士を出された。7 で動かされた時とは足取りが違う
      if (L.fromTile !== null) animRobberMove(L.fromTile, L.tile, siege !== null);
      if (L.victim === mySeat) sfx.play("robbed", 560);
      setTimeout(() => animSteal(L), 520);
      break;
    case "ROLL": animHarvest(); break;
    case "PLAY_KNIGHT": animKnight(L.actor); break;
    case "PLAY_MONOPOLY": animMonopoly(L.actor); break;
    case "MARITIME_TRADE":
    case "BUY_DEV":
    // 捨てた札も行き先は銀行。出す側しか無いので、同じ仕掛けで足りる
    case "DISCARD": animBankExchange(L); break;
    case "BUILD_ROAD":
    case "SETUP_ROAD": animRoadBuild(L); break;
    case "BUILD_SETTLEMENT":
    case "SETUP_SETTLEMENT": {
      const [x, y] = nodeXY(L.node);
      // 初期配置は「どこかから出向いて建てる」ものではない。
      // 2 つ目を置くと、遠く離れた 1 つ目から人が歩いて来ることになってしまう
      const setup = L.kind === "SETUP_SETTLEMENT";
      const walk = setup ? 0 : animBuildAt(L.actor, L.node);
      // 作業員が着いたところで土煙が上がり、その中からせり上がってくる
      setTimeout(() => dustCloud(x, y + 5), walk + (setup ? 60 : 120));
      setPieceDelay(L.node, walk + (setup ? 220 : 300));
      sfx.play("build", walk + (setup ? 160 : 240));
      break;
    }
    case "BUILD_CITY": {
      const walk = animBuildAt(L.actor, L.node, 700);
      animCityUpgrade(L.node, walk);
      setPieceDelay(L.node, walk + 380);
      sfx.play("buildCity", walk + 300);
      break;
    }
    default: break;
  }
}

// ---------------------------------------------------------------- 右のパネル

let robberTile = null;

function selectTile(tile) {
  // どのヘクスでも必ず確認を挟む。
  // 誰も面していないヘクスでも、押した瞬間に確定すると押し間違いが戻せない。
  robberTile = tile;
  render();
}

/**
 * その対局で出た目の分布。
 *
 * 「7 ばかり出る」と感じた時に、本当に偏っているのか、そう見えるだけなのかを
 * その場で見分けられるようにする。理論値の線を重ねて、比べる相手を画面に置く。
 */
function renderRollDist() {
  const box = document.getElementById("rolldist");
  const c = state.rollCounts || [];
  const total = c.reduce((a, b) => a + b, 0);
  if (!total) { box.innerHTML = `<span class="rdnone">出目 —</span>`; return; }

  // 2..12 の理論比（1,2,3,4,5,6,5,4,3,2,1）/36
  const theory = [1, 2, 3, 4, 5, 6, 5, 4, 3, 2, 1];
  const max = Math.max(...c, 1);
  // 棒の取り分。上に回数、下に目を置くので、100px の枠から 30px ほど残す
  const BAR = 70;
  const bars = c
    .map((n, i) => {
      const sum = i + 2;
      const exp = (theory[i] / 36) * total;
      const h = Math.round((n / max) * BAR);
      const eh = Math.round((exp / max) * BAR) + 14; // 目の字のぶんだけ持ち上げる（実測 14px）
      const cls = sum === 7 ? " seven" : sum === 6 || sum === 8 ? " hot" : "";
      return `<span class="rdbar${cls}"
                    title="${sum}: ${n} 回（理論 ${exp.toFixed(1)} 回）">
        <em>${n}</em>
        <i style="height:${h}px"></i>
        <u style="bottom:${eh}px"></u>
        <b>${sum}</b>
      </span>`;
    })
    .join("");
  box.innerHTML =
    `<div class="rdhead">出目 <b>${total}</b> 回<span class="rdkey"><s></s>理論値</span></div>` +
    `<div class="rdbars">${bars}</div>`;
}

/**
 * 決着したら結果を盤の上に出す。
 *
 * 「誰が勝ったか」だけでは、なぜ勝ったのかが分からない。
 * 勝利点の内訳（開拓地・都市・最長交易路・最大騎士力・伏せた勝利点）を
 * 全員ぶん、点数の高い順に並べる。
 */
function renderResult() {
  const box = document.getElementById("result");
  if (state.winner === null) { box.hidden = true; box.innerHTML = ""; return; }
  box.hidden = false;

  const rows = state.players
    .map((p) => ({ p, vp: p.vp !== null ? p.vp : p.publicVp, parts: p.vpParts || {} }))
    .sort((a, b) => b.vp - a.vp);

  const cell = (n) => `<td class="${n ? "" : "z"}">${n ? n : "—"}</td>`;
  const body = rows
    .map((r, i) => {
      const q = r.parts;
      return `<tr class="${r.p.id === state.winner ? "win" : ""}" style="--c:${PLAYER_COLORS[r.p.id]}">
        <td class="rank">${i + 1}</td>
        <td class="who"><span class="sw"></span>${escapeHtml(nameOf(r.p.id))}</td>
        ${cell(q.settlements || 0)}
        ${cell((q.cities || 0) * 2)}
        ${cell(q.longestRoad ? 2 : 0)}
        ${cell(q.largestArmy ? 2 : 0)}
        ${cell(q.devVp || 0)}
        <td class="total">${r.vp}</td>
      </tr>`;
    })
    .join("");

  box.innerHTML = `
    <div class="rcard">
      <div class="rhead" style="--c:${PLAYER_COLORS[state.winner]}">
        <span class="sw"></span>
        <b>${escapeHtml(nameOf(state.winner))} の勝ち</b>
        <span class="rturn">${state.turn} 手番</span>
      </div>
      <table class="rtable">
        <thead><tr>
          <th></th><th></th>
          <th title="開拓地 1 点ずつ">開拓地</th>
          <th title="都市 2 点ずつ">都市</th>
          <th title="最長交易路 2 点">最長路</th>
          <th title="最大騎士力 2 点">騎士力</th>
          <th title="伏せていた勝利点カード">発展</th>
          <th>合計</th>
        </tr></thead>
        <tbody>${body}</tbody>
      </table>
      <div class="rbtns">
        <button id="resultagain" class="ragain">もう一度遊ぶ</button>
        <button id="resulthome" class="rhome">ホームに戻る</button>
      </div>
      <div class="hnote" id="rnote"></div>
    </div>`;
  document.getElementById("resultagain").addEventListener("click", againVote);
  document.getElementById("resulthome").addEventListener("click", goHome);
  renderAgainNote();
}

/* --- 決着後の 2 つのボタン ---
 *
 * 「もう一度遊ぶ」は **全員が押したら**進む。
 * 「ホームに戻る」は **誰か 1 人が押したら**全員が戻る。
 * 片方は全会一致、もう片方は 1 人の意思で通る、という非対称は意図したもの
 * （設定を変えたい人が 1 人でもいるなら、待たせる意味が無い）。
 *
 * いまは対戦相手が CPU だけなので「全員」＝あなた 1 人。
 * オンライン対戦を足すときは、この 2 つの入口に他の人の票が入るだけで済む。
 */
let againVotes = new Set();

function humanSeats() {
  return state.players.filter((p) => p.human).map((p) => p.id);
}

function renderAgainNote() {
  const el = document.getElementById("rnote");
  if (!el) return;
  const need = humanSeats().length;
  el.textContent =
    need <= 1
      ? "「ホームに戻る」で人数や強さを変えられます"
      : `もう一度: ${againVotes.size} / ${need} 人`;
}

function againVote() {
  if (mySeat >= 0) againVotes.add(mySeat);
  const need = humanSeats();
  if (need.every((p) => againVotes.has(p)) || need.length === 0) {
    againVotes.clear();
    watching = false;
    newGame(true); // 設定はそのまま、盤と出目だけ引き直す
    return;
  }
  renderAgainNote();
}

function goHome() {
  againVotes.clear();
  showHome();
}

/**
 * 銀行の山。資源ごとに 1 枚の札を横に並べるだけ。
 *
 * 枚数は出さない。ここは「カードがどこから来てどこへ行くか」を見せるための
 * 置き場所で、数を読む欄ではない（後からアニメーションの起点／終点に使う）。
 * 各札は `data-res` を持っているので、外から狙って動かせる。
 */
/** 発展カードの山。資源とは別物なので、絵も色も資源札と分ける */
function devGlyph(size) {
  return `<svg width="${size}" height="${size}" viewBox="-12 -12 24 24">
    <rect x="-8" y="-10.5" width="16" height="21" rx="3" fill="#cdbcff" stroke="#3b2a63" stroke-width="1.2"/>
    <path d="M0,-6.4 L2,-1.7 L7,-1.3 L3.2,2 L4.4,6.9 L0,4.3 L-4.4,6.9 L-3.2,2 L-7,-1.3 L-2,-1.7 Z"
          fill="#3b2a63"/>
  </svg>`;
}

function renderBankPanel() {
  const box = document.getElementById("bankpanel");
  if (box.childElementCount === RES.length + 1) return; // 中身は変わらないので描き直さない
  box.innerHTML =
    RES.map((r) => `<div class="bpcard ${r}" data-res="${r}" title="${RES_JA[r]}">${glyphHtml(r, 36)}</div>`).join("") +
    `<div class="bpcard DEV" data-res="DEV" title="発展カードの山">${devGlyph(36)}</div>`;
}

/* ------------------------------------------------- 銀行と手札の間で札が飛ぶ */

/** 飛ばす端。DOM 要素でも、盤の座標から作った矩形でも受ける */
function fxRect(x) {
  if (!x) return null;
  return x.getBoundingClientRect ? x.getBoundingClientRect() : x;
}

/** 盤（SVG）の座標を画面の座標へ。飛ぶ札は画面座標で動くので変換が要る */
function svgRect(x, y, w = 34, h = 46) {
  const svg = document.getElementById("board");
  const m = svg.getScreenCTM();
  if (!m) return null;
  const p = new DOMPoint(x, y).matrixTransform(m);
  return { left: p.x - w / 2, top: p.y - h / 2, width: w, height: h };
}

/**
 * 銀行の欄と手札の間で札を 1 枚飛ばす。
 *
 * 盤の SVG ではなく **画面の座標** で動かす。両端（銀行の欄・手札の欄）は
 * 別々の要素で、共通の座標系を持たないため。
 */
function flyCard(from, to, inner, delay = 0) {
  const a = fxRect(from);
  const b = fxRect(to);
  if (!a || !b || !a.width || !b.width) return;
  const el = document.createElement("div");
  el.className = "flycard";
  el.style.cssText = `left:${a.left}px; top:${a.top}px; width:${a.width}px; height:${a.height}px`;
  el.innerHTML = inner;
  document.body.appendChild(el);

  const dx = b.left + b.width / 2 - (a.left + a.width / 2);
  const dy = b.top + b.height / 2 - (a.top + a.height / 2);
  const sc = b.width / a.width;
  const anim = el.animate(
    [
      { transform: "translate(0,0) scale(.7) rotate(0deg)", opacity: 0 },
      { transform: `translate(${(dx * 0.22).toFixed(1)}px, ${(dy * 0.22 - 28).toFixed(1)}px) scale(1) rotate(-7deg)`,
        opacity: 1, offset: 0.28 },
      { transform: `translate(${dx.toFixed(1)}px, ${dy.toFixed(1)}px) scale(${sc.toFixed(2)}) rotate(3deg)`,
        opacity: 1, offset: 0.86 },
      { transform: `translate(${dx.toFixed(1)}px, ${dy.toFixed(1)}px) scale(${(sc * 1.08).toFixed(2)}) rotate(0deg)`,
        opacity: 0 },
    ],
    { duration: 640, delay, easing: "cubic-bezier(.3,.75,.35,1)", fill: "both" }
  );
  const drop = () => el.remove();
  anim.finished.then(drop, drop);
}

/**
 * サイコロの収穫。**出目のマスから手札へ**札が飛んでくる。
 *
 * どのマスから来たかはログには入っていないので、盤から引き直す
 * （出目と同じ数字のマス／盗賊のいるマスは除く／自分の建物が面している頂点）。
 * ただし枚数は盤から数えた通りとは限らない（銀行の在庫切れ）ので、
 * **実際に増えた枚数まで切り詰める**。増えていない札は飛ばさない。
 */
function animHarvest() {
  if (mySeat < 0 || !state.dice) return;
  const me = state.players.find((p) => p.human);
  if (!me || !me.hand || !prevHandFx) return;
  const got = me.hand.map((n, i) => Math.max(0, n - prevHandFx[i]));
  if (!got.some((n) => n > 0)) return;

  const sum = state.dice[0] + state.dice[1];
  // 出目のマスのうち、自分の建物が面しているもの
  const src = [];
  for (const t of board.tiles) {
    if (t.number !== sum || t.id === state.robber || !t.resource) continue;
    for (const n of t.nodes) {
      const b = state.buildings.find((x) => x.node === n && x.owner === mySeat);
      if (!b) continue;
      src.push({ res: t.resource, x: t.x, y: t.y, n: b.kind === "CITY" ? 2 : 1 });
    }
  }
  if (!src.length) return;

  const hand = document.getElementById("handbar");
  let k = 0;
  for (let i = 0; i < RES.length; i++) {
    let left = got[i];
    if (!left) continue;
    for (const o of src) {
      if (o.res !== RES[i] || left <= 0) continue;
      for (let j = 0; j < Math.min(o.n, left); j++) {
        flyCard(
          svgRect(o.x, o.y),
          document.querySelector(`#handbar .hcard.${RES[i]}`) || hand,
          `<div class="fcard ${RES[i]}">${glyphHtml(RES[i], 26)}</div>`,
          700 + k++ * 95
        );
      }
      left -= Math.min(o.n, left);
    }
  }
}

/**
 * 銀行との交換・発展カードの購入。
 *
 * 出した札は手札 → 銀行、貰った札は銀行 → 手札へ飛ぶ。
 * 相手が誰であれ卓の上では同じことが起きているが、飛ばす先の「手札の欄」は
 * 自分のものしか無いので、自分の取引の時だけ動かす。
 */
function animBankExchange(L) {
  if (L.actor !== mySeat) return;
  const bp = (r) => document.querySelector(`#bankpanel .bpcard[data-res="${r}"]`);
  const hand = document.getElementById("handbar");
  const hc = (r) => document.querySelector(`#handbar .hcard.${r}`) || hand;
  const face = (r) => `<div class="fcard ${r}">${glyphHtml(r, 30)}</div>`;

  // 何枚動いたかは `last` には入っていない（座標の類しか持っていない）。
  // ログの先頭＝いま起きたことなので、そこから増減を取る
  const e = state.log && state.log[0];
  const cost = (e && e.actor === L.actor && e.cost) || [];
  const gain = (e && e.actor === L.actor && e.gain) || [];

  let k = 0;
  cost.forEach((n, i) => {
    for (let j = 0; j < n; j++) flyCard(hc(RES[i]), bp(RES[i]), face(RES[i]), k++ * 85);
  });
  const back = 220 + k * 85;
  let g = 0;
  gain.forEach((n, i) => {
    for (let j = 0; j < n; j++) flyCard(bp(RES[i]), hc(RES[i]), face(RES[i]), back + g++ * 85);
  });
  // 発展カードは手札の欄に居場所が無いので、欄そのものへ飛ばす
  if (L.kind === "BUY_DEV") flyCard(bp("DEV"), hand, `<div class="fcard DEV">${devGlyph(30)}</div>`, back);
}

/** 「できごと」の左に並べる。詳細ログはさらにその左へ */
function placeBankPanel() {
  const box = document.getElementById("sidestack");
  const r = document.getElementById("log").getBoundingClientRect();
  box.style.top = `${r.top}px`;
  box.style.left = `${r.left - box.offsetWidth - 12}px`;
}

function renderPlayers() {
  const box = document.getElementById("players-panel");
  box.innerHTML = "<h3>プレイヤー</h3>";
  for (const p of state.players) {
    const d = document.createElement("div");
    d.className = "pl" + (p.id === state.turnPlayer ? " active" : "");
    d.style.setProperty("--c", PLAYER_INK[p.id]);
    const tags = [];
    if (state.longestRoadOwner === p.id) tags.push(`<span class="tag">最長交易路</span>`);
    if (state.largestArmyOwner === p.id) tags.push(`<span class="tag">最大騎士力</span>`);
    const left = (n, label, icon) =>
      `<span title="${label}の残り">${iconHtml(icon, 16, "light")}<b class="${n === 0 ? "none" : ""}">${n}</b></span>`;
    // 点数はこの欄で一番先に読む数字。左端に置いて、その下に誰の点かを書く。
    // 伏せた勝利点は他人からは見えないので、見えている点だけを出す（「+」は付けない）
    const vpNum = p.vp !== null ? p.vp : p.publicVp;
    const vpTip = p.vp !== null ? "勝利点" : "見えている勝利点（伏せた発展カードは入っていない）";
    d.innerHTML = `
      <span class="swatch" style="background:${PLAYER_INK[p.id]}"></span>
      <div class="pscore">
        <div class="vp" title="${vpTip}">${vpNum}<i>pt</i></div>
        <div class="name">${escapeHtml(nameOf(p.id))}</div>
        <div class="me">${p.human ? "あなた" : "CPU"}</div>
      </div>
      <div class="pinfo">
        ${tags.length ? `<div class="ptags">${tags.join("")}</div>` : ""}
        <div class="stats">
          <span class="${p.handSize > 7 ? "over" : ""}"
                title="手札${p.handSize > 7 ? "（7 を超えているので 7 が出ると半分捨てる）" : ""}"
                >${iconHtml("hand", 21, "light")}<b class="${p.handSize > 7 ? "over" : ""}">${p.handSize}</b></span>
          <span title="発展カード">${iconHtml("dev", 21, "light")}<b>${p.devCount}</b></span>
          <span title="使った騎士">${iconHtml("knight", 21, "light")}<b>${p.playedKnights}</b></span>
          <span title="つながっている道">${iconHtml("road", 21, "light")}<b>${p.longestRoad}</b></span>
        </div>
        <div class="left">
          <span class="lb">残り</span>
          ${left(p.roadsLeft, "道", "road")}
          ${left(p.settlementsLeft, "開拓地", "settle")}
          ${left(p.citiesLeft, "都市", "city")}
        </div>
        ${p.human && devHeld() ? `<div class="mydev">${devHeld()}</div>` : ""}
      </div>`;
    box.appendChild(d);
  }
}

/**
 * 手札に **いま作りかけの交換を当てた結果**。
 *
 * 交易の画面で枚数をいじっている間、手札はその条件が通った後の枚数を出す。
 * 「これを出したら何が残るのか」を頭の中で引き算させない。
 */
function handDelta() {
  const d = [0, 0, 0, 0, 0];
  if (state.prompt === "DISCARD") {
    for (let i = 0; i < 5; i++) d[i] = -draft.give[i];
  } else if (openMenu === "OFFER_TRADE" || state.prompt === "DECIDE_TRADE") {
    for (let i = 0; i < 5; i++) d[i] = draft.want[i] - draft.give[i];
  } else if (openMenu === "MARITIME_TRADE") {
    for (let i = 0; i < 5; i++) d[i] = bank.take[i] - bank.give[i];
  } else if (openMenu === "PLAY_YEAR_OF_PLENTY") {
    for (let i = 0; i < 5; i++) d[i] = yop[i];
  }
  return d;
}

function renderHand() {
  const box = document.getElementById("handbar");
  const me = state.players.find((p) => p.human);
  if (!me || !me.hand) { box.innerHTML = ""; box.hidden = true; return; }
  box.hidden = false;
  const d = handDelta();
  const shown = me.hand.map((n, i) => Math.max(0, n + d[i]));
  const total = shown.reduce((a, b) => a + b, 0);
  let html = `<div class="hcards">`;
  RES.forEach((r, i) => {
    const cls = d[i] > 0 ? " up" : d[i] < 0 ? " down" : "";
    html += `<div class="hcard ${r}${shown[i] === 0 ? " zero" : ""}${cls}"
                  title="${RES_JA[r]} ${me.hand[i]} 枚${d[i] ? `　→ 成立すると ${shown[i]} 枚` : ""}">
      <span class="hglyph">${glyphHtml(r, 36)}</span>
      <span class="hn">${shown[i]}</span>
      <span class="hl">${RES_JA[r]}</span></div>`;
  });
  html += `</div>`;
  // 枚数はカードに書いてある。7 枚を超えた警告はプレイヤー欄の手札を赤くして示す
  // （自分だけでなく全員ぶん見える情報なので、そちらに置いた方が比べられる）
  box.innerHTML = html;
}

const SPATIAL = new Set(["SETUP_SETTLEMENT", "SETUP_ROAD", "BUILD_ROAD", "BUILD_SETTLEMENT", "BUILD_CITY"]);
/** 操作卓のタイルから選んでから置く手。選ぶ前は盤面を光らせない */
const MENU_SPATIAL = new Set(["BUILD_ROAD", "BUILD_SETTLEMENT", "BUILD_CITY"]);
const PLAY_DEV_KINDS = ["PLAY_KNIGHT", "PLAY_ROAD_BUILDING", "PLAY_YEAR_OF_PLENTY", "PLAY_MONOPOLY"];

/** 建てるのに要る資源。表に出して「何が足りないか」を見せるために持つ */
const COST = {
  BUILD_ROAD: [1, 1, 0, 0, 0],
  BUILD_SETTLEMENT: [1, 1, 1, 1, 0],
  BUILD_CITY: [0, 0, 0, 2, 3],
  BUY_DEV: [0, 0, 1, 1, 1],
};

/** 手番でできること。**できない物も必ず並べて**、なぜできないかを書く */
const MENU = [
  { kind: "BUILD_ROAD", name: "街道", icon: "road", left: "roadsLeft" },
  { kind: "BUILD_SETTLEMENT", name: "開拓地", icon: "settle", left: "settlementsLeft" },
  { kind: "BUILD_CITY", name: "都市", icon: "city", left: "citiesLeft" },
  // 買うと戻せないので、1 手しか無くても確認を挟む
  { kind: "BUY_DEV", name: "発展カード", icon: "dev", menu: true },
  { kind: "OFFER_TRADE", name: "交易を提案", icon: "hand", always: true },
  // menu: 手が 1 つしか無くても、必ず選ぶ画面を挟む。
  // 「気づいたら騎士を切っていた」は取り返しがつかない
  { kind: "MARITIME_TRADE", name: "銀行・港と交換", icon: "hand", menu: true },
  { kind: "PLAY_DEV", name: "発展を使う", icon: "knight", menu: true },
];

/** 場所を選んでいる最中の種別。選択中は盤面もその手だけを光らせる */
let pickKind = null;
/** 開いている小メニュー（銀行・提案・発展カード）*/
let openMenu = null;
/** 交易の下書き。give = 自分が出す枚数 / want = 欲しい枚数 */
let draft = { give: [0, 0, 0, 0, 0], want: [0, 0, 0, 0, 0] };
/** どの提案に対する下書きか。場面が変わったら作り直す目印 */
let draftKey = null;
/** 自分の提案が流れたら、提案の画面に戻す（手の一覧まで戻さない） */
let reopenOffer = false;
function clearDraft() {
  draft = { give: [0, 0, 0, 0, 0], want: [0, 0, 0, 0, 0] };
  draftKey = null;
  clearBank();
  yop = [0, 0, 0, 0, 0];
}

function actionsOf(kind) {
  if (kind === "PLAY_DEV") return state.actions.filter((a) => PLAY_DEV_KINDS.includes(a.kind));
  return state.actions.filter((a) => a.kind === kind);
}

function costChips(kind) {
  const c = COST[kind];
  if (!c) return "";
  const me = state.players.find((p) => p.human);
  return `<span class="cost">${c
    .map((n, i) =>
      n > 0
        ? `<span class="cc${me && me.hand && me.hand[i] < n ? " short" : ""}">${glyphHtml(RES[i], 14)}${
            n > 1 ? n : ""
          }</span>`
        : ""
    )
    .join("")}</span>`;
}

/**
 * 持っている発展カードの内訳（種類ごとの枚数）。
 * `playableOnly` にすると勝利点カードを外す ── 使う手ではないので、
 * 「発展を使う」の横に出しても押せる手が増えるわけではない。
 */
function devHeld(playableOnly) {
  const me = state.players.find((p) => p.human);
  if (!me || !me.dev) return "";
  const parts = me.dev
    .map((v, i) =>
      v > 0 && !(playableOnly && i === 4) ? `<span class="dv">${DEV_JA[i]}${v > 1 ? v : ""}</span>` : ""
    )
    .join("");
  return parts ? `<span class="devheld">${parts}</span>` : "";
}

/** できない理由。`short` は 1 行に収まる短い方 */
function whyNot(m, short) {
  const me = state.players.find((p) => p.human);
  const c = COST[m.kind];
  if (c && me && me.hand) {
    const lack = RES.filter((r, i) => me.hand[i] < c[i]);
    // 足りない資源は要りようのカードの絵が赤く落ちるので、字は短くてよい
    if (lack.length) return short ? "資源不足" : lack.map((r) => RES_JA[r]).join("・") + "が足りない";
  }
  if (m.left && me && me[m.left] === 0) return short ? "駒切れ" : "駒がもうない";
  if (m.kind === "PLAY_DEV") return short ? "カード無し" : "使えるカードが無い";
  if (m.kind === "MARITIME_TRADE") return short ? "枚数不足" : "交換できる枚数が無い";
  if (m.kind === "OFFER_TRADE") return short ? "手札が無い" : "出せる資源が無い";
  if (m.kind === "BUY_DEV") return short ? "山切れ" : "山が尽きた";
  return short ? "場所無し" : "置ける場所が無い";
}

function tile({ cls = "", icon, name, sub, disabled, title: tip, onClick }) {
  const b = document.createElement("button");
  b.className = "tile " + cls + (disabled ? " off" : "");
  b.disabled = !!disabled;
  if (tip) b.title = tip;
  b.innerHTML = `<span class="ticon">${icon ? iconHtml(icon, 20) : ""}</span>
    <span class="tname">${name}</span>
    <span class="tsub">${sub || ""}</span>`;
  if (onClick && !disabled) b.addEventListener("click", onClick);
  return b;
}

/** 資源の束を絵で。0 枚の物は出さない */
function bundleChips(b, cls) {
  if (!b) return "";
  const h = b
    .map((n, i) => (n > 0 ? `<span class="chip ${cls || ""}">${glyphHtml(RES[i], 15)}${n}</span>` : ""))
    .join("");
  return h || `<span class="chip none">なし</span>`;
}

/** 「A を渡して B を貰う」の 1 行 */
function dealHtml(give, want, giveLabel, wantLabel) {
  return `<div class="deal">
    <span class="dl">${giveLabel}</span>${bundleChips(give, "out")}
    <span class="darrow">→</span>
    <span class="dl">${wantLabel}</span>${bundleChips(want, "in")}
  </div>`;
}

/**
 * 交易の提案画面。文字の一覧ではなく、**手札と同じ絵で枚数を決める**。
 * 上段 = 欲しい物（銀行にある 5 種から）、下段 = 自分が出す物（手札の範囲で）。
 * 同じ資源を両側に置くことはできない（ただで貰う形になるため）。
 */
function renderTradeComposer(box, mode) {
  // answer = 提案された側（元の条件を入れた状態から始める）
  const answer = mode === "answer";
  const counter = answer || mode === "counter";
  const me = state.players.find((p) => p.human);
  const hand = (me && me.hand) || [0, 0, 0, 0, 0];
  const gTotal = draft.give.reduce((a, b) => a + b, 0);
  const wTotal = draft.want.reduce((a, b) => a + b, 0);

  const row = (which) =>
    RES.map((r, i) => {
      const n = draft[which][i];
      const locked = which === "want" ? draft.give[i] > 0 : draft.want[i] > 0;
      const capped = which === "give" ? n >= hand[i] : n >= 19;
      // 出す側は、持っていない資源をひと目で分かるように落とす
      const empty = which === "give" && hand[i] === 0;
      const why = locked
        ? "（反対側に置いているので使えません）"
        : empty
        ? "（手札にありません）"
        : "";
      return `<div class="tcard ${r}${locked ? " locked" : ""}${empty ? " empty" : ""}"
                   data-w="${which}" data-i="${i}" title="${RES_JA[r]}${why}">
        <button class="tminus" data-act="-" ${n === 0 ? "disabled" : ""}>−</button>
        <span class="tglyph">${glyphHtml(r, 22)}</span>
        <span class="tn">${n}</span>
        <button class="tplus" data-act="+" ${locked || capped ? "disabled" : ""}>+</button>
        ${which === "give" ? `<span class="thave">${hand[i]}</span>` : ""}
      </div>`;
    }).join("");

  const t = state.trade;
  // 元の条件のまま？ 変えていなければ「受ける」、変えていれば「対案」
  const asOffered =
    !!t &&
    t.give.every((n, i) => n === draft.want[i]) &&
    t.want.every((n, i) => n === draft.give[i]);

  const head = counter
    ? `<div class="offerbox" style="--c:${PLAYER_COLORS[t ? t.proposer : 0]}">
         <b>${t ? escapeHtml(nameOf(t.proposer)) : "?"} からの提案</b>
         ${t ? dealHtml(t.give, t.want, "渡す", "欲しい") : ""}
       </div>`
    : `<div class="banner"><b>交易を提案</b>　欲しい物と出す物を選んでください</div>`;

  box.innerHTML = `
    ${head}
    <div class="trade">
      <div class="trow">${row("want")}</div>
      <div class="tarrow"><span class="up">▲ もらう</span><span class="dn">▼ わたす</span></div>
      <div class="trow">${row("give")}</div>
    </div>`;

  const ok = gTotal > 0 && wTotal > 0;

  // 元の条件のままなら「受ける」を出す（承諾は払える時だけ合法）
  const acc = state.actions.find((a) => a.kind === "ACCEPT_TRADE");
  // 「このまま受ける」は条件を作る操作（積む・返す）の**後**に置く。
  // 上に置くと、条件をいじってから目を戻す先が毎回上へ飛ぶ
  const accept = answer
    ? tile({
        cls: "big roll mid",
        name: "このまま受ける",
        sub: acc
          ? asOffered
            ? ""
            : `<span class="no">枚数を戻すと受けられます</span>`
          : `<span class="no">求められた資源が足りない</span>`,
        disabled: !acc || !asOffered,
        onClick: () => play(acc.i),
      })
    : null;

  // 「木か土ならいいよ」＝ 成立してよい条件を並べて、提案者に選ばせる。
  // 対案の時だけ。提案側は相手が複数いるので、この形は使わない。
  // 積んだ候補は自分の返答欄にそのまま入っている（相手にはまだ見えない）
  const mySeat0 = state.players.find((p) => p.human)?.id;
  const alts = (t && t.answers && t.answers.find((a) => a.p === mySeat0)?.alts) || [];
  const stagedCount = alts.length;

  if (counter) {
    const staged = stagedCount;
    const allowed = wasm.can_counter() === 1;
    if (staged > 0) {
      const list = alts
        .map(
          (x, k) => `<div class="stitem">
            <span class="sn">${k + 1}</span>
            ${dealHtml(x.give, x.want, "出す", "貰う")}
            <button class="sdel" data-i="${x.alt}" title="この候補を取り消す">✕</button>
          </div>`
        )
        .join("");
      box.insertAdjacentHTML(
        "beforeend",
        `<div class="staged">
          <div class="sl">積んだ候補（${staged} 本）</div>
          ${list}
        </div>`
      );
      box.querySelectorAll(".sdel").forEach((b) => {
        b.addEventListener("click", (ev) => {
          if (wasm.counter_alt_remove(+ev.currentTarget.dataset.i) === 1) {
            refreshState();
            render();
          }
        });
      });
    }
    // 押せない条件は上と同じ。色は「押せる時だけ」返答色にする
    const canStage = ok && staged < 4 && allowed && !asOffered;
    box.appendChild(
      tile({
        cls: "mid" + (canStage ? " roll" : ""),
        name: "候補として積む",
        // 提案そのままの条件は積ませない。それは対案ではなく「受ける」だから。
        // 枚数を変えて初めて押せるようにすることで、2 つのボタンの役割が重ならない。
        sub: !allowed
          ? `<span class="no">交渉の時間切れ</span>`
          : staged >= 4
          ? `<span class="no">これ以上は無理</span>`
          : "",
        disabled: !ok || staged >= 4 || !allowed || asOffered,
        onClick: () => {
          if (wasm.counter_alt(...draft.give, ...draft.want) === 1) {
            sfx.play("offer");
            // 積んだ後は空にする。元の条件に戻すと、次の候補を作るのに
            // わざわざ「−」で減らすところから始めることになる。
            // draftKey は残すので、提案の条件が入り直すこともない。
            draft = { give: [0, 0, 0, 0, 0], want: [0, 0, 0, 0, 0] };
            refreshState();
            render();
          }
        },
      })
    );
  }

  // 対案は「積んでから返す」の 1 本道にする。
  // 条件を作る操作（下の欄）と、返答を確定する操作（このボタン）を混ぜると、
  // いま作りかけの条件が入るのか入らないのかが分からなくなる。
  const canSend = counter ? wasm.can_counter() === 1 && stagedCount > 0 : ok;
  const send = tile({
    cls: counter ? "mid" + (canSend ? " roll" : "") : "big roll mid",
    name: counter ? "この条件で対案を返す" : "この条件で提案する",
    // 対案側は注釈を出さない。「候補として積む」が隣にあるので、
    // 押せない理由（＝まだ積んでいない）は並びを見れば分かる
    sub: counter ? "" : ok ? "" : `<span class="no">両側に 1 枚以上</span>`,
    disabled: !canSend,
    onClick: () => {
      const okDone = counter ? wasm.counter_finish() === 1 : wasm.offer_custom(...draft.give, ...draft.want) === 1;
      if (okDone) {
        sfx.play("offer");
        // 断られて手番に戻ってきたら、条件を作り直せるようにここへ戻す
        reopenOffer = !counter;
        clearDraft();
        openMenu = null;
        refreshState();
        render();
        scheduleBot();
      }
    },
  });
  box.appendChild(send);
  if (accept) box.appendChild(accept);
  if (answer) {
    const rej = state.actions.find((a) => a.kind === "REJECT_TRADE");
    // 「受ける」と「断る」は同じ返答なので同じ見た目に。
    // 条件を作る側（積む・返す）とは色で分かれる
    if (rej) box.appendChild(tile({ cls: "wide mid roll", name: "断る", onClick: () => play(rej.i) }));
  } else {
    box.appendChild(tile({ cls: "wide mid", name: "やめる", onClick: () => { clearDraft(); openMenu = null; render(); } }));
  }

  box.querySelectorAll(".tcard button").forEach((b) => {
    b.addEventListener("click", (ev) => {
      const c = ev.currentTarget.closest(".tcard");
      const w = c.dataset.w;
      const i = +c.dataset.i;
      const d = ev.currentTarget.dataset.act === "+" ? 1 : -1;
      const next = draft[w][i] + d;
      const max = w === "give" ? hand[i] : 19;
      draft[w][i] = Math.max(0, Math.min(max, next));
      // 同種を両側に置けない
      if (draft[w][i] > 0) draft[w === "give" ? "want" : "give"][i] = 0;
      renderActions();
      renderHand();
    });
  });
}

/**
 * 盗賊を置いたヘクスに面している相手を、持ち物つきのカードで並べて選ばせる。
 *
 * 「P2 から盗む」という字だけでは、誰が太っているのか分からない。
 * 面している人が 1 人でも出す（誰から取るかを確認せずに取らされない）。
 */
function renderVictimPick(box) {
  const t = board.tiles[robberTile];
  const acts = state.actions.filter((a) => a.tile === robberTile);
  const me = state.players.find((p) => p.human);

  // そのヘクスに面している相手
  const near = new Set();
  for (const n of t.nodes || []) {
    const b = state.buildings.find((x) => x.node === n);
    if (b) near.add(b.owner);
  }

  // 相手は **全員** 並べる。奪えない人も理由つきで残す方が、
  // 「なぜこの人からは取れないのか」を盤と見比べずに済む
  const others = state.players.filter((p) => !me || p.id !== me.id);
  const canAny = others.some((p) => acts.some((x) => x.victim === p.id));

  box.innerHTML = `<div class="banner pick">${
    canAny
      ? "<b>誰から奪うか</b>　持ち物を見て選んでください"
      : "<b>ここに置きますか？</b>　この場所からは誰からも奪えません"
  }</div>`;

  const wrap = document.createElement("div");
  wrap.className = "victims";
  for (const p of others) {
    const a = acts.find((x) => x.victim === p.id);
    const why = a ? "" : !near.has(p.id) ? "この場所に面していない" : "手札が無い";
    const b = document.createElement("button");
    b.className = "vcard" + (a ? "" : " off");
    b.disabled = !a;
    b.style.setProperty("--c", PLAYER_COLORS[p.id]);
    b.innerHTML = `
      <div class="vhead"><span class="sw"></span>${escapeHtml(nameOf(p.id))}
        <span class="vvp">${p.vp !== null ? p.vp : p.publicVp}<i>点</i></span></div>
      <div class="vstats">
        <span title="手札">${iconHtml("hand", 15)}<b>${p.handSize}</b></span>
        <span title="発展カード">${iconHtml("dev", 15)}<b>${p.devCount}</b></span>
        <span title="使った騎士">${iconHtml("knight", 15)}<b>${p.playedKnights}</b></span>
        <span title="最長の道">${iconHtml("road", 15)}<b>${p.longestRoad}</b></span>
      </div>
      ${why ? `<div class="vno">${why}</div>` : ""}`;
    if (a) b.addEventListener("click", () => play(a.i));
    wrap.appendChild(b);
  }
  box.appendChild(wrap);

  // 誰からも奪えないヘクスは、置くだけ
  const none = acts.find((x) => x.victim === undefined);
  if (none) {
    box.appendChild(
      tile({ cls: "big roll mid", name: "ここに置く", sub: "", onClick: () => play(none.i) })
    );
  }
  // 「選び直す」ボタンは置かない。盤の光っている場所はそのまま押せるので、
  // 別のヘクスをクリックすればそのまま選び直せる
}

/** 銀行・港との交換の下書き（give はレートの倍数、take は枚数） */
let bank = { give: [0, 0, 0, 0, 0], take: [0, 0, 0, 0, 0] };
function clearBank() { bank = { give: [0, 0, 0, 0, 0], take: [0, 0, 0, 0, 0] }; }

/**
 * 銀行・港との交換。提案と同じく枚数を絵で決める。
 *
 * 1 回ずつしか交換できないと、8 枚出して 2 枚貰うのに同じ操作を 2 度させることになる。
 * ここでまとめて組ませて、中では普通の海上交易を続けて指す。
 * 出す側はレートの倍数でしか動かない（4:1 の資源なら 4 枚ずつ）。
 */
function renderBankTrade(box) {
  const me = state.players.find((p) => p.human);
  const hand = (me && me.hand) || [0, 0, 0, 0, 0];
  const rate = RES.map((_, i) => wasm.maritime_rate(i));

  // 出した枚数で何回交換できるか
  const trades = RES.reduce((n, _, i) => n + (rate[i] ? bank.give[i] / rate[i] : 0), 0);
  const taken = bank.take.reduce((a, b) => a + b, 0);

  const row = (which) =>
    RES.map((r, i) => {
      const n = bank[which][i];
      const locked = which === "take" ? bank.give[i] > 0 : bank.take[i] > 0;
      const step = which === "give" ? rate[i] : 1;
      const capped =
        which === "give" ? n + step > hand[i] : taken >= trades || n >= (state.bank ? state.bank[i] : 19);
      const empty = which === "give" && hand[i] < rate[i];
      return `<div class="tcard ${r}${locked ? " locked" : ""}${empty ? " empty" : ""}"
                   data-w="${which}" data-i="${i}"
                   title="${RES_JA[r]}${which === "give" ? `　${rate[i]} 枚で 1 枚` : ""}">
        <button class="tminus" data-act="-" ${n === 0 ? "disabled" : ""}>−</button>
        <span class="tglyph">${glyphHtml(r, 22)}</span>
        <span class="tn">${n}</span>
        <button class="tplus" data-act="+" ${locked || capped ? "disabled" : ""}>+</button>
        ${which === "give" ? `<span class="thave">${rate[i]}:1</span>` : ""}
      </div>`;
    }).join("");

  box.innerHTML = `
    <div class="trade">
      <div class="trow">${row("take")}</div>
      <div class="tarrow"><span class="up">▲ もらう</span><span class="dn">▼ わたす</span></div>
      <div class="trow">${row("give")}</div>
    </div>`;

  const ok = trades > 0 && taken === trades;
  box.appendChild(
    tile({
      cls: "big roll mid",
      name: "この条件で交換する",
      sub: "",
      disabled: !ok,
      onClick: () => {
        if (wasm.maritime_bulk(...bank.give, ...bank.take) === 1) {
          clearBank();
          openMenu = null;
          refreshState();
          render();
          scheduleBot();
        }
      },
    })
  );
  box.appendChild(
    tile({ cls: "wide mid", name: "やめる", onClick: () => { clearBank(); openMenu = null; render(); } })
  );

  box.querySelectorAll(".tcard button").forEach((b) => {
    b.addEventListener("click", (ev) => {
      const c = ev.currentTarget.closest(".tcard");
      const w = c.dataset.w;
      const i = +c.dataset.i;
      const step = (w === "give" ? rate[i] : 1) * (ev.currentTarget.dataset.act === "+" ? 1 : -1);
      const max = w === "give" ? hand[i] : 19;
      bank[w][i] = Math.max(0, Math.min(max, bank[w][i] + step));
      // 同じ資源を両側に置かない
      if (bank[w][i] > 0) bank[w === "give" ? "take" : "give"][i] = 0;
      // 出す枚数を減らしたら、貰う枚数も入る分まで詰める
      let t = RES.reduce((n, _, k) => n + (rate[k] ? bank.give[k] / rate[k] : 0), 0);
      for (let k = 4; k >= 0 && bank.take.reduce((a, b) => a + b, 0) > t; k--) {
        while (bank.take[k] > 0 && bank.take.reduce((a, b) => a + b, 0) > t) bank.take[k]--;
      }
      renderActions();
      renderHand();
    });
  });
}

/** 資源カード 1 枚ぶんの絵。枚数の増減つきにもできる */
function resCard(i, n, opts = {}) {
  const r = RES[i];
  const { max, have, big } = opts;
  const atMax = max !== undefined && n >= max;
  return `<div class="tcard ${r}${big ? " big" : ""}" data-i="${i}"
               title="${RES_JA[r]}${have !== undefined ? ` ${have} 枚` : ""}">
    ${opts.steppers !== false ? `<button class="tminus" data-act="-" ${n === 0 ? "disabled" : ""}>−</button>` : ""}
    <span class="tglyph">${glyphHtml(r, big ? 30 : 22)}</span>
    <span class="tn">${n}</span>
    ${opts.steppers !== false ? `<button class="tplus" data-act="+" ${atMax ? "disabled" : ""}>+</button>` : ""}
    ${have !== undefined ? `<span class="thave">${have}</span>` : ""}
  </div>`;
}

/**
 * 7 が出た時の廃棄。交易と同じ絵で、**出す側だけ**を選ばせる。
 * 組み合わせの一覧を文字で並べると、何を捨てるのか読み取れない。
 */
function renderDiscard(box) {
  const me = state.players.find((p) => p.human);
  const hand = (me && me.hand) || [0, 0, 0, 0, 0];
  const need = wasm.discard_needed();
  const picked = draft.give.reduce((a, b) => a + b, 0);

  box.innerHTML = `
    <div class="banner"><b>7 が出ました</b>　${need} 枚 捨ててください</div>
    <div class="trade">
      <div class="tlabel">捨てる<span class="${picked === need ? "" : "short"}">${picked} / ${need} 枚</span></div>
      <div class="trow">${RES.map((r, i) =>
        resCard(i, draft.give[i], { max: hand[i], have: hand[i] })
      ).join("")}</div>
    </div>`;

  box.appendChild(
    tile({
      cls: "big roll",
      name: "これを捨てる",
      sub: picked === need ? `${need} 枚` : `あと ${need - picked} 枚`,
      disabled: picked !== need,
      onClick: () => {
        if (wasm.discard_custom(...draft.give) === 1) {
          clearDraft();
          refreshState();
          render();
          scheduleBot();
        }
      },
    })
  );

  box.querySelectorAll(".tcard button").forEach((b) => {
    b.addEventListener("click", (ev) => {
      const i = +ev.currentTarget.closest(".tcard").dataset.i;
      const d = ev.currentTarget.dataset.act === "+" ? 1 : -1;
      draft.give[i] = Math.max(0, Math.min(hand[i], draft.give[i] + d));
      renderActions();
      renderHand();
    });
  });
}

/** 独占・収穫で資源を選ぶ。文字でなくカードを並べて選ばせる */
/** 収穫（銀行から 2 枚）で選んでいる枚数 */
let yop = [0, 0, 0, 0, 0];

/**
 * 独占はカードを 1 枚選ぶだけ。
 * 収穫は **2 枚** なので、提案と同じように枚数で決めさせる（同じ資源 2 枚も取れる）。
 */
function renderResourcePick(box, kind) {
  const list = actionsOf(kind);

  if (kind === "PLAY_MONOPOLY") {
    box.innerHTML = `<div class="banner"><b>独占</b>　全員から集める資源を選んでください</div>`;
    const row = document.createElement("div");
    row.className = "trow pickrow";
    for (const a of list) {
      const b = document.createElement("button");
      b.className = "rescard " + RES[a.res];
      b.innerHTML = `${glyphHtml(RES[a.res], 30)}<span class="rl">${RES_JA[RES[a.res]]}</span>`;
      b.addEventListener("click", () => { openMenu = null; play(a.i); });
      row.appendChild(b);
    }
    box.appendChild(row);
    box.appendChild(tile({ cls: "wide mid", name: "やめる", onClick: () => { openMenu = null; render(); } }));
    return;
  }

  // --- 収穫（2 枚）---
  const total = yop.reduce((a, b) => a + b, 0);
  // あと 1 枚その資源を足せるか。銀行の在庫は「その組み合わせの手が生成されているか」で分かる
  const canAdd = (i) => {
    if (total >= 2) return false;
    const other = yop.findIndex((n, k) => n > 0 && k !== i);
    if (yop[i] === 1) return list.some((a) => a.res === i && a.res2 === i);
    if (other >= 0) {
      return list.some((a) => (a.res === i && a.res2 === other) || (a.res === other && a.res2 === i));
    }
    return list.some((a) => a.res === i || a.res2 === i);
  };

  const chosen =
    total === 2
      ? list.find((a) => {
          const c = [0, 0, 0, 0, 0];
          c[a.res]++;
          c[a.res2]++;
          return c.every((n, k) => n === yop[k]);
        })
      : null;

  box.innerHTML = `
    <div class="banner"><b>収穫</b>　銀行から 2 枚（同じ資源 2 枚でも可）</div>
    <div class="trade">
      <div class="trow">${RES.map((r, i) => {
        const off = yop[i] === 0 && !canAdd(i);
        return `<div class="tcard ${r}${off ? " empty" : ""}" data-i="${i}" title="${RES_JA[r]}">
          <button class="tminus" data-act="-" ${yop[i] === 0 ? "disabled" : ""}>−</button>
          <span class="tglyph">${glyphHtml(r, 22)}</span>
          <span class="tn">${yop[i]}</span>
          <button class="tplus" data-act="+" ${canAdd(i) ? "" : "disabled"}>+</button>
        </div>`;
      }).join("")}</div>
    </div>`;

  box.appendChild(
    tile({
      cls: "big roll mid",
      name: "これをもらう",
      sub: chosen ? "" : `<span class="no">あと ${2 - total} 枚</span>`,
      disabled: !chosen,
      onClick: () => { yop = [0, 0, 0, 0, 0]; openMenu = null; play(chosen.i); },
    })
  );
  box.appendChild(
    tile({ cls: "wide mid", name: "やめる", onClick: () => { yop = [0, 0, 0, 0, 0]; openMenu = null; render(); } })
  );

  box.querySelectorAll(".tcard button").forEach((b) => {
    b.addEventListener("click", (ev) => {
      const i = +ev.currentTarget.closest(".tcard").dataset.i;
      yop[i] = Math.max(0, yop[i] + (ev.currentTarget.dataset.act === "+" ? 1 : -1));
      renderActions();
      renderHand();
    });
  });
}

/**
 * 提案されたとき。**いきなり対案の画面**にして、提案された条件を入れた状態から始める。
 *
 * 別画面で「受ける / 対案 / 断る」を選ばせると、対案が白紙から始まって
 * 元の条件との関係が切れる。ここで枚数をいじれば対案、いじらなければ承諾。
 */
function renderTradeAnswer(box) {
  const t = state.trade;
  const me = state.players.find((p) => p.human);
  const hand = (me && me.hand) || [0, 0, 0, 0, 0];

  // 提案の向きをこちら側に読み替えて入れておく（相手が欲しい物＝こちらが出す物）。
  // 払えない分は手札まで落とす（そのぶん自動的に「対案」になる）
  if (t) {
    const key = `${t.proposer}|${t.give}|${t.want}`;
    if (draftKey !== key) {
      draftKey = key;
      draft = {
        give: t.want.map((n, i) => Math.min(n, hand[i])),
        want: t.give.slice(),
      };
    }
  }
  renderTradeComposer(box, "answer");
}

/** 全員の返答が出そろったあと、提案者が相手を選ぶ */
function renderTradePick(box) {
  // 成立させる手。候補ごとに 1 つ並ぶので、中身を絵で添えて区別できるようにする。
  // 返答の一覧（誰が対案・誰が断った）は出さない。**この並びを見れば同じことが分かる**ので、
  // 上でもう一度説明されると、読む物が 2 つに増えるだけになる
  const picks = state.actions.filter((a) => a.kind === "CONFIRM_TRADE" || a.kind === "ACCEPT_COUNTER");

  box.innerHTML = picks.length
    ? `<div class="banner"><b>相手を選ぶ</b>　成立させられる条件です</div>`
    : `<div class="banner"><b>相手を選ぶ</b>　応じた相手はいませんでした</div>`;

  for (const a of picks) {
    const b = document.createElement("button");
    b.className = "tile pick";
    b.innerHTML = `<span class="pname" style="--c:${PLAYER_COLORS[a.who]}">${escapeHtml(nameOf(a.who))} と成立</span>
      ${dealHtml(a.give, a.want, "渡す", "貰う")}`;
    b.addEventListener("click", () => play(a.i));
    box.appendChild(b);
  }
  const cancel = state.actions.find((a) => a.kind === "CANCEL_TRADE");
  if (cancel) box.appendChild(tile({ cls: "wide", name: "取り下げる", onClick: () => play(cancel.i) }));
}

/**
 * 盤のクリック待ちなのに光る場所が 1 つも無い時の逃げ道。
 *
 * 🔴 実際に詰まったことがある（街道建設カードの無償の道が光らずゲームが進まなくなった）。
 * 盤の光だけに進行を預けると、絞り込みを 1 か所間違えただけで**詰む**。
 * 見落としがあっても必ず手が指せるよう、この保険は残しておく。
 */
function appendFallback(box) {
  if (document.querySelectorAll("#board .spot, #board .spot-edge, #board .spot-tile").length) return;
  const bar = document.createElement("div");
  bar.className = "banner";
  bar.innerHTML = "盤に候補が出ていません。ここから選んでください";
  box.appendChild(bar);
  const wrap = document.createElement("div");
  wrap.className = "sublist";
  for (const a of state.actions) {
    const b = document.createElement("button");
    b.className = "subitem";
    b.textContent = a.label + (a.node !== undefined ? ` #${a.node}` : a.edge !== undefined ? ` #${a.edge}` : "");
    b.addEventListener("click", () => play(a.i));
    wrap.appendChild(b);
  }
  box.appendChild(wrap);
}

function renderActions() {
  const box = document.getElementById("actions");
  box.innerHTML = "";

  if (state.winner !== null) {
    box.innerHTML = `<div class="banner win">${escapeHtml(nameOf(state.winner))} の勝ち</div>`;
    return;
  }
  if (!isHumanTurn()) {
    // 誰の番かはプレイヤー欄が光って示している。ここには何も出さない
    box.innerHTML = "";
    pickKind = null;
    openMenu = null;
    return;
  }

  // --- 盤面から場所を選んでいる最中 ---
  if (pickKind) {
    const n = actionsOf(pickKind).length;
    const bar = document.createElement("div");
    bar.className = "banner pick";
    bar.innerHTML = `<b>盤面の光っている場所</b>をクリック（${n} か所）`;
    box.appendChild(bar);
    box.appendChild(tile({ cls: "wide", name: "やめる", onClick: () => { pickKind = null; render(); } }));
    appendFallback(box);
    return;
  }

  // --- 交易の提案・対案は専用の画面（枚数を絵で決める）---
  if (openMenu === "OFFER_TRADE") {
    renderTradeComposer(box, "offer");
    return;
  }

  // --- 銀行・港との交換。提案と同じく、カードを選んで決める ---
  if (openMenu === "MARITIME_TRADE") {
    renderBankTrade(box);
    return;
  }

  // --- 発展カードを買う。押した瞬間に引かれないよう確認を挟む ---
  if (openMenu === "BUY_DEV") {
    const a = actionsOf("BUY_DEV")[0];
    box.innerHTML = `<div class="banner"><b>発展カードを買いますか？</b></div>`;
    box.appendChild(
      tile({
        cls: "big roll mid",
        name: "買う",
        sub: "",
        disabled: !a,
        onClick: () => play(a.i),
      })
    );
    box.appendChild(tile({ cls: "wide mid", name: "やめる", onClick: () => { openMenu = null; render(); } }));
    return;
  }

  // --- 独占・収穫の資源選び ---
  if (openMenu === "PLAY_MONOPOLY" || openMenu === "PLAY_YEAR_OF_PLENTY") {
    renderResourcePick(box, openMenu);
    return;
  }

  // --- 発展カードを使う。種類で選ばせてから中身を決める ---
  if (openMenu === "PLAY_DEV") {
    const kinds = [
      { k: "PLAY_KNIGHT", name: "騎士", sub: "盗賊を動かす" },
      { k: "PLAY_ROAD_BUILDING", name: "街道建設", sub: "道を 2 本ただで" },
      { k: "PLAY_YEAR_OF_PLENTY", name: "収穫", sub: "銀行から 2 枚" },
      { k: "PLAY_MONOPOLY", name: "独占", sub: "全員から集める" },
    ];
    box.innerHTML = "";
    for (const d of kinds) {
      const list = actionsOf(d.k);
      box.appendChild(
        tile({
          cls: "mid",
          icon: "dev",
          name: d.name,
          sub: list.length ? d.sub : `<span class="no">持っていない</span>`,
          disabled: !list.length,
          onClick: () => {
            if (d.k === "PLAY_MONOPOLY" || d.k === "PLAY_YEAR_OF_PLENTY") { openMenu = d.k; render(); }
            else play(list[0].i);
          },
        })
      );
    }
    box.appendChild(tile({ cls: "wide mid", name: "やめる", onClick: () => { openMenu = null; render(); } }));
    return;
  }

  // --- 小メニュー（発展カードを使う など）---
  if (openMenu) {
    const list = actionsOf(openMenu);
    const bar = document.createElement("div");
    bar.className = "banner";
    bar.innerHTML = `<b>${MENU.find((m) => m.kind === openMenu)?.name || "選ぶ"}</b>（${list.length}）`;
    box.appendChild(bar);
    const wrap = document.createElement("div");
    wrap.className = "sublist";
    for (const a of list) {
      const b = document.createElement("button");
      b.className = "subitem";
      b.textContent = a.label;
      b.addEventListener("click", () => { openMenu = null; play(a.i); });
      wrap.appendChild(b);
    }
    box.appendChild(wrap);
    box.appendChild(tile({ cls: "wide", name: "やめる", onClick: () => { openMenu = null; render(); } }));
    return;
  }

  // --- 初期配置・盗賊・無償の道: 盤面をクリックするだけ ---
  if (["SETUP_SETTLEMENT", "SETUP_ROAD", "FREE_ROAD", "MOVE_ROBBER"].includes(state.prompt)) {
    const bar = document.createElement("div");
    bar.className = "banner pick";
    if (state.prompt === "MOVE_ROBBER" && robberTile !== null) {
      renderVictimPick(box);
      return;
    }
    // 初期配置は盤が光っているのを見れば分かる。案内は出さない。
    // 手番の途中に割り込む場面（街道建設カードの無償の道・盗賊）だけ、
    // なぜ盤を押させられているのかを一言だけ出す
    if (state.prompt !== "SETUP_SETTLEMENT" && state.prompt !== "SETUP_ROAD") {
      bar.innerHTML =
        state.prompt === "FREE_ROAD" ? "<b>無償の道</b>を置く場所をクリック" : "<b>盗賊</b>を置く場所をクリック";
      box.appendChild(bar);
    }
    appendFallback(box);
    return;
  }

  // --- 交易を持ちかけられた側 ---
  if (state.prompt === "DECIDE_TRADE") {
    renderTradeAnswer(box);
    return;
  }
  // --- 提案者が相手を選ぶ（承諾と対案が並ぶ）---
  if (state.prompt === "DECIDE_ACCEPTEES") {
    renderTradePick(box);
    return;
  }
  // --- 7 の廃棄も絵で ---
  if (state.prompt === "DISCARD") {
    renderDiscard(box);
    return;
  }

  // --- 手番のはじめ: サイコロか、先に発展カードか ---
  const roll = state.actions.find((a) => a.kind === "ROLL");
  if (roll) {
    // 発展カードを先に置く。騎士で盗賊をどける判断は「振る前」にするものなので、
    // 決める順番どおりに並べる
    const dev = actionsOf("PLAY_DEV");
    box.appendChild(
      tile({
        cls: "big mid",
        icon: "knight",
        name: "発展カードを使う",
        sub: dev.length ? "" : `<span class="no">使えるカードが無い</span>`,
        disabled: !dev.length,
        onClick: () => { openMenu = "PLAY_DEV"; render(); },
      })
    );
    box.appendChild(
      tile({ cls: "big roll mid", icon: null, name: "サイコロを振る", sub: "", onClick: () => play(roll.i) })
    );
    return;
  }

  // --- 提案が流れた直後は、提案の画面へ戻す ---
  if (reopenOffer) {
    reopenOffer = false;
    clearDraft();
    openMenu = "OFFER_TRADE";
    renderTradeComposer(box, "offer");
    return;
  }

  // --- 本番: できること全部を並べる。できない物も理由つきで残す ---
  for (const m of MENU) {
    const list = actionsOf(m.kind);
    // 提案は自分で組むので、生成された一覧が空でも手札さえあれば開ける
    const ok = m.always
      ? (state.players.find((p) => p.human)?.hand || []).some((n) => n > 0)
      : list.length > 0;
    const me = state.players.find((p) => p.human);
    const leftTxt = m.left && me ? `のこり ${me[m.left]}` : "";
    box.appendChild(
      tile({
        icon: m.icon,
        name: m.name,
        sub:
          m.kind === "PLAY_DEV"
            ? devHeld(true) || (ok ? "" : `<span class="no">${whyNot(m, true)}</span>`)
            : costChips(m.kind) ||
              (ok ? leftTxt : "") + (ok ? "" : `<span class="no">${whyNot(m, true)}</span>`),
        disabled: !ok,
        title: ok ? "" : whyNot(m, false),
        onClick: () => {
          if (SPATIAL.has(m.kind)) { pickKind = m.kind; render(); }
          else if (!m.always && !m.menu && list.length === 1) play(list[0].i);
          else { openMenu = m.kind; render(); }
        },
      })
    );
  }
  const end = state.actions.find((a) => a.kind === "END_TURN");
  if (end) {
    box.appendChild(tile({ cls: "end mid", name: "手番を終える", sub: "", onClick: () => play(end.i) }));
  }
}

// ---------------------------------------------------------------- ログ

/** 資源の増減をカードの絵で。払った分は赤、得た分は緑 */
function flowHtml(e) {
  const chips = (b, cls, sign) =>
    !b
      ? ""
      : b
          .map((n, i) =>
            n > 0
              ? `<span class="chip ${cls}">${sign}${glyphHtml(RES[i], 13)}${n > 1 ? n : ""}</span>`
              : ""
          )
          .join("");
  const html = chips(e.cost, "out", "−") + chips(e.gain, "in", "+");
  return html ? `<span class="flow">${html}</span>` : "";
}

function logLine(e) {
  return `<div class="line">
    <span class="who" style="background:${PLAYER_INK[e.actor]}"></span>
    <span class="txt">${escapeHtml(nameOf(e.actor))} ${escapeHtml(e.text)}${flowHtml(e)}</span></div>`;
}

function renderLog() {
  const box = document.getElementById("loglines");
  // 直近 5 巡ぶんを残してスクロールで遡れるようにする。
  // 数行で切ると「さっき何が起きたか」が見えず、全部だと長い対局で重くなる。
  const n = state.players.length;
  const roundOf = (t) => (t ? Math.floor((t - 1) / n) + 1 : 0);
  const latest = roundOf(state.turn);
  const from = Math.max(0, latest - 4);
  box.innerHTML = state.log
    .filter((e) => e.major && roundOf(e.turn || 0) >= from)
    .map(logLine)
    .join("");
  if (!document.getElementById("drawer").hidden) {
    renderDrawer();
    placeDrawer();
  }
}

/** 詳細ログで見ている巡。null なら「最新」に追従する */
let drawerRound = null;

/** その局面に存在する巡の一覧（0 = 初期配置）。新しい順ではなく昇順 */
function logRounds() {
  const n = state.players.length;
  const set = new Set(
    state.log.filter((e) => e.major).map((e) => (e.turn ? Math.floor((e.turn - 1) / n) + 1 : 0))
  );
  return [...set].sort((a, b) => a - b);
}

/**
 * 詳細ログ。**1 巡ぶんだけ**を出して、矢印で前後の巡へ移る。
 *
 * 4 人を **縦に区切って横へ並べる**（列ごとに独立して縦スクロール）。
 * 縦に積むと「誰の行いか」を見るのにスクロールが要る。
 * 高さは「できごと」と同じ、幅はそれより広く取る。
 */
function renderDrawer() {
  const cols = document.getElementById("dcols");
  const label = document.getElementById("dround");
  const rounds = logRounds();
  if (!rounds.length) {
    cols.innerHTML = `<div class="dempty">まだ何も起きていません</div>`;
    label.textContent = "—";
    return;
  }

  const latest = rounds[rounds.length - 1];
  const cur = drawerRound === null ? latest : Math.max(rounds[0], Math.min(latest, drawerRound));
  const n = state.players.length;
  const roundOf = (t) => (t ? Math.floor((t - 1) / n) + 1 : 0);

  label.innerHTML =
    (cur === 0 ? "初期配置" : `${cur} 巡目`) + (cur === latest ? `<span class="now">最新</span>` : "");
  document.getElementById("dprev").disabled = cur <= rounds[0];
  document.getElementById("dnext").disabled = cur >= latest;

  // カードが動いた出来事だけを残す。
  // 「提案した」「断った」といった交渉の往復は過程であって、盤面には何も起きていない。
  const mine = state.log.filter((e) => e.major && roundOf(e.turn || 0) === cur);
  cols.style.gridTemplateColumns = `repeat(${n}, minmax(0, 1fr))`;
  cols.innerHTML = state.players
    .map((p) => {
      const lines = mine.filter((e) => e.actor === p.id);
      const body = lines.length
        ? lines
            .slice()
            .reverse()
            .map((e) => `<div class="dline">${escapeHtml(e.text)}${flowHtml(e)}</div>`)
            .join("")
        : `<div class="dempty">—</div>`;
      return `<div class="dcol" style="--c:${PLAYER_INK[p.id]}">
        <div class="dcolhead"><span class="sw"></span>${escapeHtml(nameOf(p.id))}
          <span class="dbn">${lines.length}</span></div>
        <div class="dcolbody">${body}</div>
      </div>`;
    })
    .join("");
}

function escapeHtml(s) {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
}

// ---------------------------------------------------------------- サイコロ

const DIE_CELLS = {
  1: [4], 2: [0, 8], 3: [0, 4, 8], 4: [0, 2, 6, 8], 5: [0, 2, 4, 6, 8], 6: [0, 2, 3, 5, 6, 8],
};

function pipCells(v) {
  const on = new Set(DIE_CELLS[v] || []);
  let cells = "";
  for (let i = 0; i < 9; i++) cells += on.has(i) ? "<i></i>" : "<span></span>";
  return cells;
}

function dieHtml(v, settled) {
  return `<span class="die${settled ? " settled" : ""}">${pipCells(v)}</span>`;
}

const rnd6 = () => 1 + Math.floor(Math.random() * 6);
let rollTimer = null;
let rollHide = null;

/** 盤の中央でカランカランと転がす */
function bigRoll(a, b) {
  const fx = document.getElementById("rollfx");
  const d0 = document.getElementById("bd0");
  const d1 = document.getElementById("bd1");
  const sum = document.getElementById("rollsum");
  clearInterval(rollTimer);
  clearTimeout(rollHide);

  fx.hidden = false;
  fx.classList.remove("out");
  sum.classList.remove("show");
  sum.innerHTML = "";
  for (const d of [d0, d1]) {
    d.classList.remove("rolling", "landed");
    void d.offsetWidth; // アニメーションを頭から掛け直す
    d.classList.add("rolling");
  }

  // 転がっている間は目がころころ変わる
  rollTimer = setInterval(() => {
    d0.innerHTML = pipCells(rnd6());
    d1.innerHTML = pipCells(rnd6());
  }, 48);

  setTimeout(() => {
    clearInterval(rollTimer);
    rollTimer = null;
    d0.innerHTML = pipCells(a);
    d1.innerHTML = pipCells(b);
    for (const d of [d0, d1]) { d.classList.remove("rolling"); d.classList.add("landed"); }
    sum.innerHTML = `合計 <b>${a + b}</b>`;
    sum.classList.add("show");
    rollHide = setTimeout(() => {
      fx.classList.add("out");
      rollHide = setTimeout(() => { fx.hidden = true; fx.classList.remove("out"); }, 300);
    }, 520);
  }, 650);
}

function renderDice() {
  const box = document.getElementById("dicebox");
  if (!state.dice) { box.innerHTML = ""; lastDiceKey = ""; return; }
  box.innerHTML = dieHtml(state.dice[0], true) + dieHtml(state.dice[1], true) +
    `<span class="sum">${state.dice[0] + state.dice[1]}</span>`;
  // 転がすのは「いま振った」時だけ。
  // 交渉中は返答のたびに to_act が変わるので、そこを鍵に入れると
  // 提案のたびにサイコロが降ってくる（実際そうなっていた）。
  const isRoll = state.last && state.last.kind === "ROLL";
  const key = isRoll ? `roll:${state.last.seq}` : "";
  if (!isRoll || key === lastDiceKey) return;
  lastDiceKey = key;
  sfx.play("dice");
  bigRoll(state.dice[0], state.dice[1]);
}

/**
 * 手番の欄の一番上。
 *
 * 「何手番目に誰が何をした」はログを見れば分かるので出さない。
 * ここに残すのは **今すぐ効く情報だけ** ── 交渉の残り時間。
 */
function renderPrompt() {
  // 上に出す文字は無い。持ち時間も設けていないので、この帯は常に畳んでおく
  document.getElementById("prompt").hidden = true;
  renderDice();
}

// ---------------------------------------------------------------- 進行

function isHumanTurn() { return wasm.is_human_turn() === 1; }
function refreshState() { state = readJson(wasm.state_json()); }

/**
 * 効果音の見張り。
 *
 * 出来事の側から鳴らせない音（手番が回ってきた・捨てる場面に入った・
 * 称号が動いた）は、**描画のたびに前回と見比べて**変わった時だけ鳴らす。
 * 最初の描画では鳴らさない ── 対局を開いた瞬間に全部鳴ってしまうので。
 */
let sfxPrev = null;

/** 1 つ前の描画時点での自分の手札。収穫で「何枚増えたか」を出すのに使う */
let prevHandFx = null;

function sfxWatch() {
  const now = {
    prompt: isHumanTurn() ? state.prompt : null,
    turnPlayer: state.turnPlayer,
    army: state.largestArmyOwner,
    road: state.longestRoadOwner,
    winner: state.winner,
  };
  const was = sfxPrev;
  sfxPrev = now;
  if (!was) return;

  // 決着。リザルトが出るのと同じ描画で鳴らす
  if (was.winner === null && now.winner !== null) sfx.play("win");
  if (now.winner !== null) return;

  // 自分の手番が回ってきた
  if (mySeat >= 0 && now.turnPlayer !== was.turnPlayer && now.turnPlayer === mySeat) sfx.play("turn");

  if (now.prompt !== was.prompt) {
    // 7 で捨てる場面に入った
    if (now.prompt === "DISCARD") sfx.play("discard");
    // 提案が届いた／返答が出そろった
    if (now.prompt === "DECIDE_TRADE" || now.prompt === "DECIDE_ACCEPTEES") sfx.play("offer");
  }

  // 称号の移動。取った側も取られた側も同じ音（盤の上で起きたことは同じ）
  if (now.army !== was.army || now.road !== was.road) sfx.play("award");
}

function render() {
  drawState();
  runAnimations();
  renderPlayers();
  renderBankPanel();
  renderResult();
  renderRollDist();
  renderHand();
  renderActions();
  renderLog();
  renderPrompt();
  // 出すものが何も無い時（観戦中など）は、空の箱を残さない
  const tb = document.getElementById("turnbox");
  const hb = document.getElementById("handbar");
  tb.hidden = !document.getElementById("actions").innerHTML && hb.hidden;
  placeBankPanel();
  placeDrawer();
  sfxWatch();
  const meFx = state.players.find((p) => p.human);
  prevHandFx = meFx && meFx.hand ? meFx.hand.slice() : null;
}

function play(i) {
  robberTile = null;
  pickKind = null;
  openMenu = null;
  // 自分で手を指したなら、提案の画面へ戻す必要はない
  // （成立・取り下げ・手番終了のどれであっても、その意思で先へ進んでいる）
  reopenOffer = false;
  clearDraft();
  wasm.apply_index(i);
  refreshState();
  render();
  scheduleBot();
}

function scheduleBot() {
  clearTimeout(botTimer);
  if (state.winner !== null) { watching = false; return; }
  if (isHumanTurn()) return;
  // サイコロの演出が終わるまでは次の手を待つ
  // サイコロが転がり終わるまでは次の手を止める（合計 650 + 見せる 520）
  const wait = state.last && state.last.kind === "ROLL" ? 1020 : (watching ? 110 : 430);
  botTimer = setTimeout(() => {
    if (wasm.bot_step() === 1) {
      refreshState();
      render();
      scheduleBot();
    }
  }, wait);
}

/* ---------------------------------------------------------------- ホーム（待機所） */

/**
 * 待機所の設定。
 *
 * **参加者の一覧**として持つ（席の一覧ではない）。席は対局を始める時に配る。
 * オンライン対戦を足すときは、ここに種類 "remote" の参加者が並ぶだけで済む。
 */
let lobby = {
  name: "あなた",
  members: [
    { kind: "human" },
    { kind: "cpu", level: 1 },
    { kind: "cpu", level: 1 },
    { kind: "cpu", level: 1 },
  ],
};

const LEVEL_NAMES = ["やさしい", "ふつう", "つよい", "さいきょう"];

/** 席 → 表示名。対局が始まると席の並びが決まるので、そこで作る */
let seatNames = [];

function nameOf(pid) {
  return seatNames[pid] || `P${pid}`;
}

function loadLobby() {
  try {
    const raw = localStorage.getItem("catan.lobby");
    if (raw) {
      const v = JSON.parse(raw);
      if (v && Array.isArray(v.members) && v.members.length >= 3) lobby = v;
    }
  } catch {
    // 読めなくても既定のままで遊べる
  }
}

function saveLobby() {
  try {
    localStorage.setItem("catan.lobby", JSON.stringify(lobby));
  } catch {
    // 覚えられなくても、この対局の間は効く
  }
}

function renderHome() {
  document.getElementById("myname").value = lobby.name;

  const seg = document.getElementById("hplayers");
  seg.innerHTML = "";
  for (const n of [3, 4]) {
    const b = document.createElement("button");
    b.textContent = `${n} 人`;
    b.className = lobby.members.length === n ? "on" : "";
    b.addEventListener("click", () => {
      while (lobby.members.length > n) lobby.members.pop();
      while (lobby.members.length < n) lobby.members.push({ kind: "cpu", level: 1 });
      saveLobby();
      renderHome();
    });
    seg.appendChild(b);
  }

  const box = document.getElementById("hseats");
  box.innerHTML = "";
  lobby.members.forEach((m, i) => {
    const d = document.createElement("div");
    d.className = "hseat";
    d.style.setProperty("--c", PLAYER_INK[i]);
    if (m.kind === "human") {
      d.innerHTML = `<span class="sw"></span>
        <span class="who">${escapeHtml(lobby.name || "あなた")}<span class="me">あなた</span></span>`;
    } else {
      const opts = LEVEL_NAMES.map(
        (nm, lv) => `<option value="${lv}"${lv === m.level ? " selected" : ""}>${nm}</option>`
      ).join("");
      d.innerHTML = `<span class="sw"></span>
        <span class="who">CPU ${i}</span>
        <select data-i="${i}">${opts}</select>`;
    }
    box.appendChild(d);
  });
  box.querySelectorAll("select").forEach((sel) => {
    sel.addEventListener("change", (ev) => {
      lobby.members[+ev.currentTarget.dataset.i].level = +ev.currentTarget.value;
      saveLobby();
    });
  });

  const lv = lobby.members.filter((m) => m.kind === "cpu").map((m) => LEVEL_NAMES[m.level]);
  document.getElementById("hnote").textContent =
    `${lobby.members.length} 人（あなた + CPU ${lv.length} 体: ${lv.join("・")}）`;
}

function showHome() {
  clearTimeout(botTimer);
  watching = false;
  document.getElementById("home").hidden = false;
  document.getElementById("app").hidden = true;
  renderHome();
}

function startFromHome() {
  lobby.name = document.getElementById("myname").value.trim() || "あなた";
  saveLobby();
  document.getElementById("home").hidden = true;
  document.getElementById("app").hidden = false;
  newGame(true);
}

function newGame(newSeed) {
  clearTimeout(botTimer);
  clearTimeout(rollHide);
  clearInterval(rollTimer);
  document.getElementById("rollfx").hidden = true;
  robberTile = null;
  lastPieces = new Set();
  lastDiceKey = "";
  lastSeq = 0;
  robberEverMoved = false;
  sfxPrev = null;
  prevHandFx = null;
  // 出来事ごとに別の種。盤だけ固定してサイコロを振り直す、といったことができる
  const ids = ["seed", "seed2", "seed3", "seed4"];
  if (newSeed) {
    for (const [k, id] of ids.entries()) document.getElementById(id).value = randomSeed(k);
  }
  const [boardSeed, diceSeed, devSeed, stealSeed] = ids.map((id) =>
    parseInt(document.getElementById(id).value || "1", 10)
  );
  const players = lobby.members.length;
  // 席もランダムに。毎回 1 番手だと初手の有利不利が固定されてしまう。
  // 種から決めるので、同じ種なら席も含めて同じ試合を再現できる。
  mySeat = watching ? -1 : diceSeed % players;

  // 参加者を席に配る。人間は上で決めた席、CPU は残りへ順に。
  // 名前と強さはここで席の並びに写す（以降の描画は席番号だけを見る）
  seatNames = new Array(players);
  let levels = 0;
  const cpus = lobby.members.filter((m) => m.kind === "cpu");
  let ci = 0;
  for (let seat = 0; seat < players; seat++) {
    if (seat === mySeat) {
      seatNames[seat] = lobby.name || "あなた";
      continue;
    }
    const m = cpus[ci++] || { level: 1 };
    seatNames[seat] = LEVEL_NAMES[m.level];
    levels |= (m.level & 0b11) << (seat * 2);
  }
  if (watching) {
    for (let seat = 0; seat < players; seat++) seatNames[seat] = `CPU ${seat}`;
  }

  clearDraft();
  wasm.game_new(
    boardSeed, diceSeed, devSeed, stealSeed, players,
    watching ? 0 : 1 << mySeat, levels
  );
  board = readJson(wasm.board_json());
  refreshState();
  drawBoard();
  render();
  scheduleBot();
}

// 実時間の注入。エンジンは自分で時計を読まないので、ここで流し込まないと
// 手番の持ち時間が永久に減らない（＝交渉の打ち切りが効かない）
let lastTick = performance.now();
setInterval(() => {
  if (!wasm) return;
  const now = performance.now();
  wasm.tick(Math.round(now - lastTick));
  lastTick = now;
}, 500);

document.getElementById("newgame").addEventListener("click", () => { watching = false; newGame(true); });
document.getElementById("watch").addEventListener("click", () => { watching = true; newGame(true); });
/**
 * 「できごと」の実寸に合わせて、そのすぐ左隣に置く。
 *
 * ただし手札と操作の欄（`#turnbox`）は絶対に隠さない。
 * 交易の画面が開くと操作欄が高くなるので、その時は下を詰めて避ける。
 */
function placeDrawer() {
  const d = document.getElementById("drawer");
  if (d.hidden) return;
  const r = document.getElementById("log").getBoundingClientRect();
  const gap = 12;
  // 高さは「できごと」と同じ。幅は 4 人を横に並べたいので広く取る。
  // 銀行の欄の上に**被せて**開く（避けて左へ回すと盤に食い込む）
  const anchor = r.left;
  const w = Math.max(280, Math.min(660, anchor - gap * 2));
  let left = anchor - w - gap;
  let h = r.height;

  // 手札と操作の欄は絶対に隠さない。
  // まず高さを詰めて避け、それでも狭くなりすぎるなら操作欄の左へ回す。
  const tb = document.getElementById("turnbox");
  if (tb && !tb.hidden) {
    const t = tb.getBoundingClientRect();
    if (left < t.right && left + w > t.left) {
      const fit = t.top - r.top - gap;
      if (fit >= 150) h = Math.min(h, fit);
      else left = Math.max(gap, t.left - w - gap);
    }
  }

  d.style.top = `${r.top}px`;
  d.style.height = `${h}px`;
  d.style.width = `${w}px`;
  d.style.left = `${left}px`;
}

function setDrawer(open) {
  document.getElementById("drawer").hidden = !open;
  if (open) {
    drawerRound = null; // 開いた時は最新の巡から
    renderDrawer();
    placeDrawer();
  }
}
addEventListener("resize", () => { placeBankPanel(); placeDrawer(); });
// 上の帯は普段は畳んでおく。盤を広く使いたいので、既定では出さない
document.getElementById("uitoggle").addEventListener("click", (ev) => {
  const h = document.querySelector("header");
  h.hidden = !h.hidden;
  ev.currentTarget.textContent = h.hidden ? "▾" : "▴";
  ev.currentTarget.classList.toggle("on", !h.hidden);
  // 帯が開くと「できごと」が下がる。銀行の欄もそれに付いて動かす
  placeBankPanel();
  placeDrawer();
});

// 出目の分布は普段は畳んでおく。銀行の山の下に開く
document.getElementById("disttoggle").addEventListener("click", (ev) => {
  const box = document.getElementById("rolldist");
  box.hidden = !box.hidden;
  ev.currentTarget.textContent = (box.hidden ? "▾" : "▴") + " 出目";
  ev.currentTarget.classList.toggle("on", !box.hidden);
  placeBankPanel();
});

// 効果音の入切。押した時が最初の操作になるので、ここで音が鳴らせるようになる
{
  const b = document.getElementById("sfxtoggle");
  const paint = () => {
    b.classList.toggle("off", sfx.isMuted());
    b.title = sfx.isMuted() ? "効果音を鳴らす" : "効果音を消す";
  };
  paint();
  b.addEventListener("click", () => {
    sfx.setMuted(!sfx.isMuted());
    paint();
    if (!sfx.isMuted()) sfx.play("offer");
  });
}

document.getElementById("logmore").addEventListener("click", () => setDrawer(true));
document.getElementById("drawerclose").addEventListener("click", () => setDrawer(false));

function stepRound(d) {
  const rounds = logRounds();
  if (!rounds.length) return;
  const latest = rounds[rounds.length - 1];
  const cur = drawerRound === null ? latest : drawerRound;
  const i = rounds.indexOf(Math.max(rounds[0], Math.min(latest, cur)));
  const next = rounds[Math.max(0, Math.min(rounds.length - 1, i + d))];
  // 最新まで戻したら「最新に追従」へ戻す（対局が進んでも勝手に付いていく）
  drawerRound = next === latest ? null : next;
  renderDrawer();
}
document.getElementById("dprev").addEventListener("click", () => stepRound(-1));
document.getElementById("dnext").addEventListener("click", () => stepRound(1));
document.getElementById("dlatest").addEventListener("click", () => { drawerRound = null; renderDrawer(); });

/**
 * 種を 1 つ引く。`Math.random` ではなく OS の乱数源（`crypto`）から取る。
 *
 * ⚠ ここで良い種を引いても、**出目の質そのものは変わらない**（PCG32 は元から一様）。
 * 効くのは「毎回ちがう試合になること」と「出来事どうしが絡まないこと」の 2 点。
 */
function randomSeed(k) {
  const buf = new Uint32Array(4);
  (self.crypto || self.msCrypto).getRandomValues(buf);
  return buf[k % 4] % 1_000_000_000;
}

// 種が固定だと毎回まったく同じ試合になる
for (const [k, id] of ["seed", "seed2", "seed3", "seed4"].entries()) {
  document.getElementById(id).value = randomSeed(k);
}
/**
 * 起動。
 *
 * 🔴 **何があっても真っ白にはしない**。
 * 以前ここは「盤を隠す → ホームを出す」の順だったので、
 * ブラウザが古い `index.html` を握っていて `#home` が無いと、
 * 盤を隠した直後に失敗して画面に何も残らなかった（実際に踏んだ）。
 * 配信物は別々に更新されるので、**HTML と JS の版がずれる前提**で書く。
 */
function showFatal(msg) {
  const box = document.createElement("div");
  box.style.cssText =
    "position:fixed;inset:0;z-index:999;display:flex;align-items:center;justify-content:center;" +
    "padding:24px;font:15px/1.8 system-ui,sans-serif;color:#16232e;text-align:center";
  box.innerHTML =
    `<div style="max-width:420px;background:rgba(255,255,255,.94);border-radius:16px;padding:24px">${msg}</div>`;
  document.body.appendChild(box);
}

function boot() {
  // 古い HTML が残っていたら、1 度だけ読み直して自分で直す
  if (!document.getElementById("home")) {
    let tried = "1";
    try {
      tried = sessionStorage.getItem("catan.reload") || "";
      sessionStorage.setItem("catan.reload", "1");
    } catch {
      // 読めない環境では読み直しに頼らず、下の案内を出す
    }
    if (!tried) {
      location.replace(location.pathname + "?v=" + Date.now());
      return;
    }
    showFatal("表示を更新できませんでした。<br>ページを再読み込みしてください（Ctrl + F5）。");
    return;
  }
  try {
    sessionStorage.removeItem("catan.reload");
  } catch {
    // 消せなくても実害は無い
  }

  loadLobby();
  document.getElementById("hstart").addEventListener("click", startFromHome);
  document.getElementById("myname").addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") startFromHome();
  });

  // 盤に入るのは「はじめる」を押してから。まず待機所を出す
  loadWasm()
    .then(() => {
      document.getElementById("app").hidden = true;
      showHome();
    })
    .catch((e) => {
      const note = document.getElementById("hnote");
      if (note) note.textContent = "読み込みに失敗しました: " + e;
      else showFatal("読み込みに失敗しました: " + e);
      console.error(e);
    });
}

boot();
