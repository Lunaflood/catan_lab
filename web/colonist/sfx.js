/**
 * 効果音。
 *
 * 音声ファイルは持たず、その場で **Web Audio で合成する**。
 * このプロジェクトは外部の依存を入れない方針で、
 * 音のためだけに読み込み待ちとファイル置き場を増やしたくないため。
 *
 * ブラウザは操作より前に音を鳴らさせない（自動再生の制限）。
 * `AudioContext` は最初に鳴らす時まで作らず、止まっていれば起こす。
 * まだ一度も触っていない画面では単に鳴らない ── それが正しい振る舞い。
 */

let ctx = null;
let master = null;
let noiseBuf = null;
let muted = false;

try {
  muted = localStorage.getItem("catan.mute") === "1";
} catch {
  // プライベートウィンドウなどで読めないことがある。既定（鳴らす）のままでよい
}

function ac() {
  if (muted) return null;
  if (!ctx) {
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    ctx = new AC();
    master = ctx.createGain();
    master.gain.value = 0.5;
    master.connect(ctx.destination);
  }
  if (ctx.state === "suspended") ctx.resume();
  return ctx;
}

/** 白色雑音。毎回作ると重いので 1 本作って読む位置をずらして使い回す */
function noiseBuffer() {
  if (!noiseBuf) {
    const n = ctx.sampleRate;
    noiseBuf = ctx.createBuffer(1, n, n);
    const d = noiseBuf.getChannelData(0);
    for (let i = 0; i < n; i++) d[i] = Math.random() * 2 - 1;
  }
  return noiseBuf;
}

/**
 * 単音。`to` を渡すと周波数がそこまで滑る。
 * 減衰は指数で落とす（線形だと切れ際が機械的に聞こえる）
 */
function tone({ t, freq, to, dur = 0.25, gain = 0.2, type = "sine", attack = 0.004 }) {
  const o = ctx.createOscillator();
  const g = ctx.createGain();
  o.type = type;
  o.frequency.setValueAtTime(freq, t);
  if (to && to !== freq) o.frequency.exponentialRampToValueAtTime(to, t + dur);
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(gain, t + attack);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  o.connect(g).connect(master);
  o.start(t);
  o.stop(t + dur + 0.02);
}

/** 雑音のひと吹き。帯域を絞ると「木」「砂利」「布」の質感になる */
function noise({ t, dur = 0.12, gain = 0.2, type = "bandpass", freq = 1200, to, q = 1 }) {
  const src = ctx.createBufferSource();
  src.buffer = noiseBuffer();
  src.loop = true;
  const f = ctx.createBiquadFilter();
  f.type = type;
  f.Q.value = q;
  f.frequency.setValueAtTime(freq, t);
  if (to && to !== freq) f.frequency.exponentialRampToValueAtTime(to, t + dur);
  const g = ctx.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(gain, t + 0.006);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(f).connect(g).connect(master);
  src.start(t, Math.random() * 0.5);
  src.stop(t + dur + 0.02);
}

/*
 * ここから下は **colonist の効果音を実測して作り直した** もの。
 * 音声ファイルは持たず合成する方針は変えていない。合わせたのは数値。
 *
 *   ・UI の音は 731ms 前後で統一（道・建設・提案の可否）
 *   ・クリックだけ 78ms
 *   ・通知と称号と勝利は 2〜3 秒。ここだけ立ち上がりを 300〜600ms 遅らせて「溜め」を作る
 *   ・立ち上がりの既定は 35〜50ms
 *   ・サイコロは **4 種類を用意して毎回選ぶ**（同じ音の連発を避ける）
 *
 * 各関数の頭に、元にした実測値（長さ / 立ち上がり / 強い帯域）を書いてある。
 */
const SOUNDS = {
  /** click 78ms / atk37 / 4kHz。UI を押した合図。極短 */
  click(t) {
    noise({ t, dur: 0.03, gain: 0.09, type: "highpass", freq: 3600 });
    tone({ t, freq: 2400, dur: 0.05, gain: 0.05, type: "triangle" });
  },

  /** offer_acceptable 731ms / atk42 / 1kHz 中心。ピコーン */
  offer(t) {
    tone({ t, freq: 988, dur: 0.09, gain: 0.15, attack: 0.042 });
    tone({ t: t + 0.07, freq: 1319, dur: 0.62, gain: 0.18 });
    tone({ t: t + 0.07, freq: 2637, dur: 0.3, gain: 0.045 });
  },

  /** offer_not_acceptable 731ms / atk40 / 250Hz 寄り。低く短い否定 */
  offerNo(t) {
    tone({ t, freq: 262, dur: 0.66, gain: 0.15, type: "triangle", attack: 0.04 });
    tone({ t: t + 0.06, freq: 196, dur: 0.5, gain: 0.09, type: "triangle" });
  },

  /** offer_accepted 758ms / env 22210000 / 500Hz〜1k。成立 */
  offerYes(t) {
    [523.25, 783.99].forEach((f, i) =>
      tone({ t: t + i * 0.1, freq: f, dur: 0.5, gain: 0.15, attack: 0.038 }));
    tone({ t, freq: 1046, dur: 0.3, gain: 0.05 });
  },

  /** offer_rejected 758ms / 250〜500Hz。下がって終わる */
  offerNope(t) {
    tone({ t, freq: 392, to: 262, dur: 0.7, gain: 0.14, type: "triangle", attack: 0.04 });
  },

  /**
   * dice_roll 1008ms / atk35 / 2kHz が最大 / env 11100000
   *   ＝ 前半 380ms だけ鳴って**余韻を残さない**硬い連打。
   * ★ colonist は 4 種類を持っている。ここでも 4 通りの撒き方から選ぶ。
   */
  dice(t) {
    const v = Math.floor(Math.random() * 4);
    const n = [10, 12, 11, 13][v];
    const step = [0.030, 0.026, 0.034, 0.028][v];
    for (let i = 0; i < n; i++) {
      const at = t + i * step + Math.random() * 0.022;
      // 帯域の山を 2kHz に置く。4kHz も混ぜて硬さを出す
      noise({ t: at, dur: 0.03, gain: 0.09 + Math.random() * 0.05, freq: 1800 + Math.random() * 1400, q: 3.2 });
      tone({ t: at, freq: 430 + Math.random() * 240, dur: 0.035, gain: 0.045, type: "triangle" });
    }
    // 止まる音。ここで 380ms 付近に収める
    noise({ t: t + 0.36, dur: 0.07, gain: 0.12, freq: 950, q: 2 });
  },

  /** road_place 731ms / atk36 / 250Hz と 1kHz / env 10000000 ＝ 頭だけ */
  road(t) {
    noise({ t, dur: 0.3, gain: 0.16, type: "lowpass", freq: 1400, to: 260, q: 0.7 });
    for (let i = 0; i < 6; i++) {
      const at = t + 0.015 + i * 0.04 + Math.random() * 0.02;
      noise({ t: at, dur: 0.035, gain: 0.07, freq: 1000 + Math.random() * 700, q: 2 });
    }
    tone({ t, freq: 250, to: 90, dur: 0.28, gain: 0.11, type: "triangle" });
  },

  /** settlement_place 731ms / atk38 / **500Hz 主体** / env 10000000 ＝ ドスッ */
  build(t, hits = 3) {
    for (let i = 0; i < hits; i++) {
      const at = t + i * 0.09;
      noise({ t: at, dur: 0.03, gain: 0.12, type: "highpass", freq: 2600 });
      tone({ t: at, freq: 520, dur: 0.22, gain: 0.15, type: "triangle" });
      tone({ t: at, freq: 1040, dur: 0.13, gain: 0.06, type: "sine" });
      tone({ t: at, freq: 175, dur: 0.12, gain: 0.08, type: "sine" });
    }
  },

  /** city_place 731ms / atk43 / env 21000000 ＝ settlement より低域が厚く、少し長い */
  buildCity(t) {
    SOUNDS.build(t, 4);
    tone({ t, freq: 130, to: 98, dur: 0.5, gain: 0.11, type: "triangle", attack: 0.043 });
  },

  /** robber_place 967ms / atk48 / **125Hz が最大** ＝ 重い足音 */
  robber(t) {
    for (let i = 0; i < 3; i++) {
      const at = t + i * 0.19;
      tone({ t: at, freq: 125, to: 88, dur: 0.26, gain: 0.17, type: "triangle", attack: 0.048 });
      noise({ t: at, dur: 0.09, gain: 0.06, type: "lowpass", freq: 500, q: 0.8 });
    }
  },

  /** your_turn 1944ms / atk39 / **4〜8kHz が立つ**明るいチャイム */
  turn(t) {
    const notes = [523.25, 659.25, 783.99];
    notes.forEach((f, i) =>
      tone({ t: t + i * 0.085, freq: f, dur: 1.2, gain: 0.14, attack: 0.039 }));
    // 高域の輝き。実測で 4k/8k が強いのはこの成分
    notes.forEach((f, i) => tone({ t: t + i * 0.085, freq: f * 4, dur: 0.7, gain: 0.035 }));
    tone({ t, freq: 261.63, dur: 1.6, gain: 0.05, attack: 0.05 });
  },

  /** discard_notification 1881ms / atk50 / 低域主体で長い ＝ 注意喚起 */
  discard(t) {
    for (let i = 0; i < 3; i++) {
      noise({ t: t + i * 0.09, dur: 0.13, gain: 0.1, freq: 2400 - i * 650, to: 800 - i * 200, q: 1.2 });
    }
    tone({ t: t + 0.26, freq: 165, to: 82, dur: 1.4, gain: 0.14, type: "triangle", attack: 0.05 });
    tone({ t: t + 0.26, freq: 110, dur: 1.2, gain: 0.07, type: "sine" });
  },

  /** 盗られた側。短く掠め取られる */
  robbed(t) {
    noise({ t, dur: 0.22, gain: 0.14, freq: 3200, to: 420, q: 1.4 });
    tone({ t: t + 0.04, freq: 440, to: 155, dur: 0.4, gain: 0.14, type: "sawtooth" });
    tone({ t: t + 0.04, freq: 466, to: 164, dur: 0.4, gain: 0.05, type: "sine" });
  },

  /** devcard_bought 1332ms / atk59 / env 22110000 */
  devBuy(t) {
    noise({ t, dur: 0.16, gain: 0.07, freq: 2600, to: 900, q: 1.2 });   // 引き抜く紙の音
    [440, 587.33].forEach((f, i) =>
      tone({ t: t + i * 0.1, freq: f, dur: 0.9, gain: 0.13, attack: 0.059 }));
  },

  /** devcard_used 1881ms / atk237 ＝ 少し溜めてから開く */
  devUse(t) {
    tone({ t, freq: 330, dur: 1.5, gain: 0.12, type: "triangle", attack: 0.237 });
    tone({ t: t + 0.24, freq: 494, dur: 1.1, gain: 0.1, attack: 0.05 });
    tone({ t: t + 0.24, freq: 988, dur: 0.6, gain: 0.035 });
  },

  /**
   * devcard_monopoly 2351ms / **atk634** / env 01321100
   *   ＝ 溜めが長く、中盤で山が来る。独占だけ専用音を持つのが colonist の判断。
   */
  monopoly(t) {
    tone({ t, freq: 98, to: 196, dur: 1.9, gain: 0.16, type: "sawtooth", attack: 0.634 });
    tone({ t: t + 0.6, freq: 196, to: 392, dur: 1.3, gain: 0.1, type: "triangle", attack: 0.2 });
    noise({ t: t + 1.0, dur: 0.7, gain: 0.06, type: "highpass", freq: 3600 });
  },

  /**
   * achievement_longest_road / _largest_army 2351ms / **atk344** / 低域のみ。
   * 取った側も取られた側も同じ音（盤の上で起きたことは同じ）。
   */
  award(t) {
    [329.63, 493.88].forEach((f, i) => {
      const at = t + i * 0.16;
      tone({ t: at, freq: f, dur: 1.7, gain: 0.16, attack: 0.344 });
      tone({ t: at, freq: f * 1.5, dur: 1.2, gain: 0.055, attack: 0.2 });
    });
    tone({ t, freq: 110, dur: 2.0, gain: 0.08, type: "triangle", attack: 0.3 });
  },

  /**
   * victory 2966ms / **atk425** / 低域主体 / env 12232110。
   * ゆっくり立ち上がって、山を 2 つ作ってから伸ばす。
   */
  win(t) {
    const brass = (at, f, dur, gain, atk) => {
      tone({ t: at, freq: f, dur, gain, type: "sawtooth", attack: atk });
      tone({ t: at, freq: f * 1.5, dur, gain: gain * 0.42, type: "triangle" });
      tone({ t: at, freq: f * 2, dur, gain: gain * 0.2, type: "triangle" });
    };
    brass(t, 523.25, 0.34, 0.13, 0.425);        // 溜めながらパン
    brass(t + 0.36, 523.25, 0.2, 0.12, 0.05);   // パ
    brass(t + 0.58, 659.25, 0.24, 0.14, 0.05);  // カ
    brass(t + 0.86, 783.99, 2.0, 0.15, 0.06);   // パーン
    tone({ t: t + 0.86, freq: 130.81, dur: 2.1, gain: 0.1, type: "triangle" });
    noise({ t: t + 0.86, dur: 1.5, gain: 0.07, type: "highpass", freq: 4200 });
  },
};

/** 名前で鳴らす。`delay` はミリ秒（演出の途中に合わせたい時に使う） */
export function play(name, delay = 0) {
  const c = ac();
  if (!c) return;
  const fn = SOUNDS[name];
  if (!fn) return;
  try {
    fn(c.currentTime + delay / 1000);
  } catch {
    // 音が出せなくても対局は続ける。ここで投げると描画ごと止まる
  }
}

export function isMuted() {
  return muted;
}

export function setMuted(v) {
  muted = !!v;
  try {
    localStorage.setItem("catan.mute", muted ? "1" : "0");
  } catch {
    // 覚えられなくても、この対局の間は効く
  }
  if (muted && ctx) master.gain.value = 0;
  else if (master) master.gain.value = 0.5;
}
