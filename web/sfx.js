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

const SOUNDS = {
  /** ピコーン。提案・対案が届いた／出した合図。低→高の 2 音で「!」を作る */
  offer(t) {
    tone({ t, freq: 988, dur: 0.08, gain: 0.16 });
    tone({ t: t + 0.07, freq: 1319, dur: 0.36, gain: 0.19 });
    tone({ t: t + 0.07, freq: 2637, dur: 0.22, gain: 0.05 });
  },

  /** カラカラ。粒がぶつかる音を **不規則に** 並べる。等間隔だと機械音になる */
  dice(t) {
    for (let i = 0; i < 11; i++) {
      const at = t + i * 0.045 + Math.random() * 0.03;
      noise({ t: at, dur: 0.035, gain: 0.1 + Math.random() * 0.06, freq: 1700 + Math.random() * 1600, q: 3 });
      tone({ t: at, freq: 420 + Math.random() * 260, dur: 0.04, gain: 0.05, type: "triangle" });
    }
    // 転がり終わって止まる音
    noise({ t: t + 0.56, dur: 0.09, gain: 0.13, freq: 900, q: 2 });
  },

  /** ガラガラ。低い唸りを敷いて、その上に砂利を撒く */
  road(t) {
    noise({ t, dur: 0.5, gain: 0.16, type: "lowpass", freq: 1400, to: 260, q: 0.7 });
    for (let i = 0; i < 7; i++) {
      const at = t + 0.02 + i * 0.06 + Math.random() * 0.03;
      noise({ t: at, dur: 0.04, gain: 0.07, freq: 1100 + Math.random() * 900, q: 2 });
    }
    tone({ t, freq: 120, to: 70, dur: 0.42, gain: 0.1, type: "triangle" });
  },

  /** カンカンカン。金物を打つ音。倍音を 2 本重ねて硬さを出す */
  build(t, hits = 3) {
    for (let i = 0; i < hits; i++) {
      const at = t + i * 0.17;
      noise({ t: at, dur: 0.03, gain: 0.14, type: "highpass", freq: 3200 });
      tone({ t: at, freq: 1850, dur: 0.2, gain: 0.13, type: "triangle" });
      tone({ t: at, freq: 2770, dur: 0.14, gain: 0.06, type: "sine" });
      tone({ t: at, freq: 620, dur: 0.09, gain: 0.07, type: "sine" });
    }
  },

  /** 都市は打つ回数を増やして、一段大きい工事にする */
  buildCity(t) {
    SOUNDS.build(t, 4);
  },

  /**
   * 自分の手番。
   * 「知らせ」であって「事件」ではないので、角の無い上がり 3 音にする。
   */
  turn(t) {
    const notes = [523.25, 659.25, 783.99];
    notes.forEach((f, i) => tone({ t: t + i * 0.085, freq: f, dur: 0.5, gain: 0.15, attack: 0.012 }));
    tone({ t, freq: 261.63, dur: 0.6, gain: 0.06, attack: 0.03 });
  },

  /**
   * 7 が出て手札を捨てる。
   * 札が手から落ちていく感じ。擦れる音を 3 回、下げながら重ねて最後に落とす。
   */
  discard(t) {
    for (let i = 0; i < 3; i++) {
      noise({ t: t + i * 0.09, dur: 0.13, gain: 0.11, freq: 2600 - i * 700, to: 900 - i * 220, q: 1.2 });
    }
    tone({ t: t + 0.3, freq: 170, to: 88, dur: 0.28, gain: 0.13, type: "triangle" });
  },

  /**
   * 盗賊に取られた。
   * 掠め取られる「しゅっ」と、落ちる音を重ねる。損をしたことが分かればよいので短く。
   */
  robbed(t) {
    noise({ t, dur: 0.22, gain: 0.14, freq: 3200, to: 420, q: 1.4 });
    tone({ t: t + 0.04, freq: 440, to: 155, dur: 0.4, gain: 0.14, type: "sawtooth" });
    tone({ t: t + 0.04, freq: 466, to: 164, dur: 0.4, gain: 0.05, type: "sine" });
  },

  /**
   * 最大騎士力・最長交易路の移動。
   * 取った時も取られた時も同じ音にする（盤の上で起きたことは同じ「札の移動」なので）。
   */
  award(t) {
    noise({ t, dur: 0.18, gain: 0.07, type: "highpass", freq: 5200 });
    [659.25, 987.77].forEach((f, i) => {
      const at = t + i * 0.12;
      tone({ t: at, freq: f, dur: 0.7, gain: 0.16, attack: 0.008 });
      tone({ t: at, freq: f * 1.5, dur: 0.5, gain: 0.06 });
      tone({ t: at, freq: f * 2, dur: 0.35, gain: 0.03 });
    });
  },

  /**
   * 決着。パン・パカ・パーン。
   *
   * 他の音と違ってここだけは鳴り切ってよい（対局が終わっていて、
   * 次の音と重なる心配が無い）ので、和音を重ねて長く伸ばす。
   * 金管らしさは鋸波の倍音で出す。
   */
  win(t) {
    const brass = (at, f, dur, gain) => {
      tone({ t: at, freq: f, dur, gain, type: "sawtooth", attack: 0.014 });
      tone({ t: at, freq: f * 1.5, dur, gain: gain * 0.42, type: "triangle" });
      tone({ t: at, freq: f * 2, dur, gain: gain * 0.22, type: "triangle" });
    };
    brass(t, 523.25, 0.17, 0.13);        // パン
    brass(t + 0.17, 523.25, 0.14, 0.12); // パ
    brass(t + 0.31, 659.25, 0.16, 0.13); // カ
    brass(t + 0.47, 783.99, 1.2, 0.15);  // パーン
    // 下から支える根音と、締めのシンバル
    tone({ t: t + 0.47, freq: 130.81, dur: 1.3, gain: 0.09, type: "triangle" });
    noise({ t: t + 0.47, dur: 0.95, gain: 0.075, type: "highpass", freq: 4200 });
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
