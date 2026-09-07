//! 隠れ状態の推定（M2）: 重み付き粒子フィルタ。
//!
//! 粒子 1 つが「あり得る世界」1 つ。**ルール上あり得る状態しか持たない**:
//! 資源は各 19 枚、発展は騎士14・街道2・収穫2・独占2・勝利点5 を全粒子が満たす。
//!
//! ## 資源と発展は別の粒子集合で持つ（Rao-Blackwell 化）
//!
//! 相手の**資源手札**と**発展カード**は、出来事の上でほぼ独立に動く
//! （産出・建設・交易・盗みは資源だけ、購入した札の種類・使用・不使用は発展だけを動かす。
//! 唯一の結合は「購入の代金」で、それは公開の枚数で決まる）。
//! 1 つの粒子に両方を同居させると、独占の枚数照合のような**資源側の硬い証拠**で粒子が数本まで
//! 絞られた瞬間に、発展側の多様性まで巻き添えで失われる（校正で実際に起きた: 全粒子が同じ札を決め打ち）。
//! そこで `res`（資源）と `dev`（発展・山・相手の傾向 θ）を別々に持ち、別々に再サンプルする。
//! 完全情報の世界を 1 つ作る時は、両方から独立に 1 本ずつ引く。
//!
//! ## 更新（設計書 5.2）
//! - ルールによる観測整合（産出・建設・公開交易・独占の枚数・購入当日の使用禁止）は**硬い制約**。
//!   満たさない粒子は重み 0。
//! - 相手の行動選択の尤度（使った／使わなかった）は**柔らかい重み**。傾向 θ に依存する。
//! - 自分が当事者の盗みは知らされた種類で条件付け。第三者の盗みは粒子内の手札比で移す。
//! - 手番プレイヤーが勝っていない ⇒ その人の伏せ勝利点は `10 − 公開点` 未満（正しいタイミングだけ）。
//!
//! 粒子が全滅したら**本番の手札では埋めない**。公開台帳（[`History`]）から制約を組み直す。

pub mod reach;

use catan_core::action::{Bundle, DevCard, DEV_CARDS, DEV_DECK_COMPOSITION, NUM_DEV_KINDS, EMPTY};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::{
    Game, BANK_PER_RESOURCE, COST_CITY, COST_DEV, COST_ROAD, COST_SETTLEMENT, MAX_PLAYERS, VP_TO_WIN,
};
use catan_core::observation::{EventAftermath, Observation, ObservationProfile, ObservedEvent, VisibleEvent, LEGACY_APP};
use catan_core::rng::Rng;

use crate::history::History;
use crate::opponent::{q_use, Opportunity, OpponentParams, NUM_STYLES, STYLE_BALANCED, STYLE_PRIOR};

/// 発展カードの購入世代。「いつ買った何枚か」を持ち、内部カード ID は持たない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cohort {
    pub bought_turn: u32,
    /// 買ってから本人が終えた手番の数
    pub own_turns: u32,
    pub cards: [u8; NUM_DEV_KINDS],
}

impl Cohort {
    #[inline]
    pub fn total(&self) -> u8 {
        self.cards.iter().sum()
    }
}

/// 資源の粒子: 全員の資源手札（自分の分は正確）
#[derive(Clone, Debug)]
pub struct ResParticle {
    pub w: f32,
    pub hand: [Bundle; MAX_PLAYERS],
}

/// 発展の粒子: 全員の発展カード（世代つき）・山の内訳・相手の傾向
#[derive(Clone, Debug)]
pub struct DevParticle {
    pub w: f32,
    pub cohorts: [Vec<Cohort>; MAX_PLAYERS],
    pub deck: [u8; NUM_DEV_KINDS],
    pub style: [u8; MAX_PLAYERS],
}

impl DevParticle {
    pub fn dev(&self, p: PlayerId) -> [u8; NUM_DEV_KINDS] {
        let mut out = [0u8; NUM_DEV_KINDS];
        for c in &self.cohorts[p as usize] {
            for i in 0..NUM_DEV_KINDS {
                out[i] += c.cards[i];
            }
        }
        out
    }
    pub fn dev_total(&self, p: PlayerId) -> u8 {
        self.cohorts[p as usize].iter().map(|c| c.total()).sum()
    }
    pub fn dev_bought_at(&self, p: PlayerId, turn: u32) -> [u8; NUM_DEV_KINDS] {
        let mut out = [0u8; NUM_DEV_KINDS];
        for c in &self.cohorts[p as usize] {
            if c.bought_turn == turn {
                for i in 0..NUM_DEV_KINDS {
                    out[i] += c.cards[i];
                }
            }
        }
        out
    }
    #[inline]
    pub fn hidden_vp(&self, p: PlayerId) -> u8 {
        self.cohorts[p as usize]
            .iter()
            .map(|c| c.cards[DevCard::VictoryPoint.idx()])
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeliefConfig {
    pub particles: usize,
    /// 手番終了時の「使わなかった」尤度を使う
    pub use_nonuse: bool,
    /// 有効粒子数がこの割合を下回ったら再サンプル
    pub resample_frac: f32,
    pub opp: OpponentParams,
}

impl Default for BeliefConfig {
    fn default() -> Self {
        BeliefConfig {
            particles: 256,
            use_nonuse: true,
            resample_frac: 0.5,
            opp: OpponentParams::default(),
        }
    }
}

impl BeliefConfig {
    /// アブレーション「保持履歴なし」= 超幾何分布（購入世代と山の追跡だけ）
    pub fn no_history() -> Self {
        BeliefConfig { use_nonuse: false, opp: OpponentParams::duration_only(), ..Self::default() }
    }
    /// 「保持期間だけ」= 機会の特徴量なし・傾向なし
    pub fn duration_only() -> Self {
        BeliefConfig { use_nonuse: true, opp: OpponentParams::duration_only(), ..Self::default() }
    }
    /// 「使用機会あり」= 特徴量あり・傾向なし
    pub fn features_only() -> Self {
        BeliefConfig { use_nonuse: true, opp: OpponentParams::features_only(), ..Self::default() }
    }
    /// 「相手傾向もあり」= 全部入り
    pub fn full() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub events: u64,
    pub soft_updates: u64,
    pub resamples_res: u64,
    pub resamples_dev: u64,
    /// 全粒子の重みが 0 になった回数（観測だけから組み直した）
    pub collapses_res: u64,
    pub collapses_dev: u64,
    /// 観測との照合で不整合を検出した粒子の数（= バグの指標。0 であるべき）
    pub inconsistent: u64,
    /// 観測スナップショットから作り直した回数（出来事が届かなかった時のフォールバック）
    pub resyncs: u64,
    pub min_ess_res: f32,
    pub min_ess_dev: f32,
}

impl Diagnostics {
    pub fn collapses(&self) -> u64 {
        self.collapses_res + self.collapses_dev
    }
}

pub struct Belief {
    pub viewer: PlayerId,
    pub n: usize,
    pub cfg: BeliefConfig,
    pub profile: ObservationProfile,
    pub res: Vec<ResParticle>,
    pub dev: Vec<DevParticle>,
    pub hist: History,
    pub diag: Diagnostics,
    rng: Rng,
    last_after: Option<EventAftermath>,
    /// 自分の手札の影（粒子が全滅しても止まらない正確な追跡）
    shadow_hand: Bundle,
    /// 自分の札の影（種類は私的観測で分かる）
    shadow_cohorts: Vec<Cohort>,
}

#[inline]
fn total(b: &Bundle) -> u8 {
    b.iter().sum()
}

#[inline]
fn contains(hand: &Bundle, need: &Bundle) -> bool {
    (0..NUM_RESOURCES).all(|i| hand[i] >= need[i])
}

#[inline]
fn add(hand: &mut Bundle, b: &Bundle) {
    for i in 0..NUM_RESOURCES {
        hand[i] += b[i];
    }
}

/// `hand` から `cost` を引く。足りなければ false（引けるだけ引く）
#[inline]
fn take(hand: &mut Bundle, cost: &Bundle) -> bool {
    let ok = contains(hand, cost);
    for i in 0..NUM_RESOURCES {
        hand[i] = hand[i].saturating_sub(cost[i]);
    }
    ok
}

/// 手札の枚数比で 1 枚選ぶ
fn sample_resource(hand: &Bundle, rng: &mut Rng) -> Option<usize> {
    let t = total(hand);
    if t == 0 {
        return None;
    }
    let mut k = rng.below(t as u32) as u8;
    for r in 0..NUM_RESOURCES {
        if k < hand[r] {
            return Some(r);
        }
        k -= hand[r];
    }
    None
}

fn sample_deck(deck: &[u8; NUM_DEV_KINDS], rng: &mut Rng) -> Option<DevCard> {
    let t: u8 = deck.iter().sum();
    if t == 0 {
        return None;
    }
    let mut k = rng.below(t as u32) as u8;
    for c in DEV_CARDS {
        if k < deck[c.idx()] {
            return Some(c);
        }
        k -= deck[c.idx()];
    }
    None
}

fn sample_style(rng: &mut Rng) -> u8 {
    let x = rng.next_u32() as f32 / u32::MAX as f32;
    let mut acc = 0.0;
    for (i, p) in STYLE_PRIOR.iter().enumerate() {
        acc += p;
        if x < acc {
            return i as u8;
        }
    }
    (NUM_STYLES - 1) as u8
}

fn full_deck() -> [u8; NUM_DEV_KINDS] {
    let mut deck = [0u8; NUM_DEV_KINDS];
    for (c, k) in DEV_DECK_COMPOSITION {
        deck[c.idx()] = k;
    }
    deck
}

/// 生きている粒子 `live`（重み `ws`）から、重みに比例して `k` 本を系統サンプルで選ぶ
fn systematic_pick(ws: &[f32], live: &[usize], k: usize, rng: &mut Rng) -> Vec<usize> {
    let sum: f64 = live.iter().map(|&i| ws[i] as f64).sum();
    let u0 = (rng.next_u32() as f64 / (u32::MAX as f64 + 1.0)) / k as f64;
    let mut out = Vec::with_capacity(k);
    let mut acc = 0.0f64;
    let mut j = 0usize;
    for t in 0..k {
        let u = (u0 + t as f64 / k as f64) * sum;
        while j + 1 < live.len() && acc + ws[live[j]] as f64 <= u {
            acc += ws[live[j]] as f64;
            j += 1;
        }
        out.push(live[j]);
    }
    out
}

/// 正規化して ESS を返す。全滅なら None
fn normalize(ws: &mut [f32]) -> Option<f32> {
    let sum: f64 = ws.iter().map(|&w| w as f64).sum();
    if sum <= 0.0 || !sum.is_finite() {
        return None;
    }
    let mut sq = 0.0f64;
    for w in ws.iter_mut() {
        *w = (*w as f64 / sum) as f32;
        sq += (*w as f64) * (*w as f64);
    }
    Some(if sq > 0.0 { (1.0 / sq) as f32 } else { 0.0 })
}

impl Belief {
    pub fn new(viewer: PlayerId, n: usize, cfg: BeliefConfig, seed: u64) -> Self {
        let rng = Rng::with_stream(seed, 0xBE11_EF00);
        let mut b = Belief {
            viewer,
            n,
            cfg,
            profile: LEGACY_APP,
            res: Vec::new(),
            dev: Vec::new(),
            hist: History::new(viewer, n),
            diag: Diagnostics { min_ess_res: f32::INFINITY, min_ess_dev: f32::INFINITY, ..Default::default() },
            rng,
            last_after: None,
            shadow_hand: EMPTY,
            shadow_cohorts: Vec::new(),
        };
        b.init_particles();
        b
    }

    /// 対局開始の状態（全員手札なし・山は 25 枚）
    fn init_particles(&mut self) {
        let np = self.cfg.particles.max(1);
        let mut rng = self.rng;
        self.res = (0..np).map(|_| ResParticle { w: 1.0 / np as f32, hand: [EMPTY; MAX_PLAYERS] }).collect();
        self.dev = (0..np)
            .map(|_| DevParticle {
                w: 1.0 / np as f32,
                cohorts: Default::default(),
                deck: full_deck(),
                style: std::array::from_fn(|_| if self.cfg.opp.use_styles { sample_style(&mut rng) } else { STYLE_BALANCED }),
            })
            .collect();
        self.rng = rng;
    }

    // ------------------------------------------------------------ 更新

    /// 出来事を 1 つ取り込む
    pub fn observe(&mut self, ev: &ObservedEvent) {
        if !self.hist.record(ev) {
            return; // 重複配信
        }
        self.diag.events += 1;
        self.last_after = Some(ev.after);
        let n = self.n;
        let viewer = self.viewer;
        let cfg = self.cfg;
        let want = self.hist.open_trade.map(|t| t.want);
        // EndTurn の使用機会は台帳が「手番開始 ∨ 終了」で合成したものを使う
        let merged_opp = self
            .hist
            .turns
            .last()
            .filter(|t| t.turn == ev.turn && t.player == ev.actor)
            .map(|t| t.opportunity);
        let mut rng = self.rng;

        // 影: 自分の手札と自分の札。相手側の仮説が合わなくても自分の分は必ず通す
        {
            let mut sh = ResParticle { w: 1.0, hand: [EMPTY; MAX_PLAYERS] };
            sh.hand[viewer as usize] = self.shadow_hand;
            let mut srng = Rng::new(rng.next_u32() as u64);
            apply_res(&mut sh, ev, viewer, n, want, &mut srng, false);
            self.shadow_hand = sh.hand[viewer as usize];
            let mut sd = DevParticle { w: 1.0, cohorts: Default::default(), deck: full_deck(), style: [STYLE_BALANCED; MAX_PLAYERS] };
            sd.cohorts[viewer as usize] = std::mem::take(&mut self.shadow_cohorts);
            apply_dev(&mut sd, ev, viewer, n, &cfg, merged_opp, &mut srng, false);
            self.shadow_cohorts = std::mem::take(&mut sd.cohorts[viewer as usize]);
        }

        // 資源
        let mut res_changed = false;
        for pt in self.res.iter_mut() {
            if pt.w <= 0.0 {
                continue;
            }
            let before = pt.w;
            apply_res(pt, ev, viewer, n, want, &mut rng, true);
            if pt.w > 0.0 && (0..n).any(|q| total(&pt.hand[q]) != ev.after.hand_size[q]) {
                debug_assert!(
                    false,
                    "資源の粒子が観測と矛盾: seq={} actor={} {:?}\n 粒子 {:?} 観測 {:?}",
                    ev.seq,
                    ev.actor,
                    ev.payload,
                    (0..n).map(|q| total(&pt.hand[q])).collect::<Vec<_>>(),
                    &ev.after.hand_size[..n]
                );
                pt.w = 0.0;
                self.diag.inconsistent += 1;
            }
            if pt.w != before {
                res_changed = true;
            }
        }
        // 発展
        let mut dev_changed = false;
        for pt in self.dev.iter_mut() {
            if pt.w <= 0.0 {
                continue;
            }
            let before = pt.w;
            apply_dev(pt, ev, viewer, n, &cfg, merged_opp, &mut rng, true);
            if pt.w > 0.0 && (0..n).any(|q| pt.dev_total(q as PlayerId) != ev.after.dev_count[q]) {
                debug_assert!(
                    false,
                    "発展の粒子が観測と矛盾: seq={} actor={} {:?}\n 粒子 {:?} 観測 {:?}",
                    ev.seq,
                    ev.actor,
                    ev.payload,
                    (0..n).map(|q| pt.dev_total(q as PlayerId)).collect::<Vec<_>>(),
                    &ev.after.dev_count[..n]
                );
                pt.w = 0.0;
                self.diag.inconsistent += 1;
            }
            if pt.w != before {
                dev_changed = true;
            }
        }
        self.rng = rng;
        if res_changed || dev_changed {
            self.diag.soft_updates += 1;
        }
        if res_changed {
            self.normalize_res();
        }
        if dev_changed {
            self.normalize_dev();
        }
    }

    fn normalize_res(&mut self) {
        let mut ws: Vec<f32> = self.res.iter().map(|p| p.w).collect();
        match normalize(&mut ws) {
            None => {
                self.diag.collapses_res += 1;
                self.rebuild_res();
            }
            Some(ess) => {
                for (p, w) in self.res.iter_mut().zip(ws.iter()) {
                    p.w = *w;
                }
                if ess < self.diag.min_ess_res {
                    self.diag.min_ess_res = ess;
                }
                if ess < self.cfg.resample_frac * self.res.len() as f32 {
                    let live: Vec<usize> = (0..self.res.len()).filter(|&i| self.res[i].w > 0.0).collect();
                    let n = self.res.len();
                    let mut rng = self.rng;
                    let picks = systematic_pick(&ws, &live, n, &mut rng);
                    self.rng = rng;
                    self.res = picks.iter().map(|&i| ResParticle { w: 1.0 / n as f32, hand: self.res[i].hand }).collect();
                    self.diag.resamples_res += 1;
                }
            }
        }
    }

    fn normalize_dev(&mut self) {
        let mut ws: Vec<f32> = self.dev.iter().map(|p| p.w).collect();
        match normalize(&mut ws) {
            None => {
                self.diag.collapses_dev += 1;
                self.rebuild_dev();
            }
            Some(ess) => {
                for (p, w) in self.dev.iter_mut().zip(ws.iter()) {
                    p.w = *w;
                }
                if ess < self.diag.min_ess_dev {
                    self.diag.min_ess_dev = ess;
                }
                if ess < self.cfg.resample_frac * self.dev.len() as f32 {
                    let live: Vec<usize> = (0..self.dev.len()).filter(|&i| self.dev[i].w > 0.0).collect();
                    let n = self.dev.len();
                    let mut rng = self.rng;
                    let picks = systematic_pick(&ws, &live, n, &mut rng);
                    self.rng = rng;
                    let mut out = Vec::with_capacity(n);
                    for &i in &picks {
                        let mut p = self.dev[i].clone();
                        p.w = 1.0 / n as f32;
                        out.push(p);
                    }
                    self.dev = out;
                    self.diag.resamples_dev += 1;
                }
            }
        }
    }

    // ------------------------------------------------------------ 組み直し

    /// 公開台帳と直近の公開の要点から粒子を作り直す（全滅時・観測欠落時）
    pub fn rejuvenate(&mut self) {
        self.rebuild_res();
        self.rebuild_dev();
    }

    /// 観測スナップショットから作り直す。出来事が届いていない（順序番号が飛んだ）時のフォールバック。
    /// 履歴は失われるが、秘密は使わない。
    pub fn resync(&mut self, obs: &Observation) {
        let n = obs.num_players();
        let g = &obs.public.game;
        let mut after = EventAftermath {
            dev_deck_left: obs.public.dev_deck_left,
            bank: obs.public.bank,
            longest_road_owner: g.longest_road_owner,
            largest_army_owner: g.largest_army_owner,
            turn_player: g.turn_player,
            to_act: g.to_act,
            prompt: g.prompt,
            winner: g.winner,
            ..EventAftermath::default()
        };
        for q in 0..n {
            after.hand_size[q] = obs.public.players[q].hand_size;
            after.dev_count[q] = obs.public.players[q].dev_count;
            after.dev_bought_this_turn[q] = obs.public.players[q].dev_bought_this_turn;
            after.public_vp[q] = obs.public.players[q].public_vp;
        }
        for q in 0..n {
            self.hist.ledgers[q].played_dev = obs.public.players[q].played_dev;
        }
        self.hist.turn = g.turn;
        self.hist.turn_player = g.turn_player;
        self.last_after = Some(after);
        // 自分の札は私的観測から
        let mut own = Vec::new();
        let mut rest = obs.own.dev;
        let now = obs.own.dev_bought_this_turn;
        for i in 0..NUM_DEV_KINDS {
            rest[i] -= now[i];
        }
        if now.iter().any(|&v| v > 0) {
            own.push(Cohort { bought_turn: g.turn, own_turns: 0, cards: now });
        }
        if rest.iter().any(|&v| v > 0) {
            own.push(Cohort { bought_turn: 0, own_turns: 1, cards: rest });
        }
        self.diag.resyncs += 1;
        self.shadow_hand = obs.own.hand;
        self.shadow_cohorts = own;
        self.rejuvenate();
    }

    /// 資源の粒子を、公開の枚数・銀行・自分の手札・開いている交易の条件から作り直す
    fn rebuild_res(&mut self) {
        let Some(after) = self.last_after else {
            self.res = (0..self.cfg.particles.max(1)).map(|_| ResParticle { w: 1.0 / self.cfg.particles.max(1) as f32, hand: [EMPTY; MAX_PLAYERS] }).collect();
            return;
        };
        let n = self.n;
        let me = self.viewer as usize;
        let own_hand = self.shadow_hand;
        let bank = after.bank.unwrap_or(EMPTY);
        let mut fixed = [EMPTY; MAX_PLAYERS];
        if let Some(t) = self.hist.open_trade {
            fixed[t.proposer as usize] = t.give;
            for q in 0..n {
                if t.accepted[q] {
                    for r in 0..NUM_RESOURCES {
                        fixed[q][r] = fixed[q][r].max(t.want[r]);
                    }
                }
                for r in 0..NUM_RESOURCES {
                    fixed[q][r] = fixed[q][r].max(t.counter_give_max[q][r]);
                }
            }
        }
        fixed[me] = EMPTY;
        for q in 0..n {
            if total(&fixed[q]) > after.hand_size[q] {
                fixed[q] = EMPTY;
                self.diag.inconsistent += 1;
            }
        }
        let mut pool = [0u8; NUM_RESOURCES];
        for r in 0..NUM_RESOURCES {
            let mut v = BANK_PER_RESOURCE.saturating_sub(bank[r]).saturating_sub(own_hand[r]);
            for q in 0..n {
                v = v.saturating_sub(fixed[q][r]);
            }
            pool[r] = v;
        }
        let np = self.cfg.particles.max(1);
        let mut rng = self.rng;
        let mut out = Vec::with_capacity(np);
        for _ in 0..np {
            let mut bag: Vec<u8> = Vec::new();
            for r in 0..NUM_RESOURCES {
                for _ in 0..pool[r] {
                    bag.push(r as u8);
                }
            }
            rng.shuffle(&mut bag);
            let mut hand = [EMPTY; MAX_PLAYERS];
            hand[me] = own_hand;
            let mut k = 0usize;
            for q in 0..n {
                if q == me {
                    continue;
                }
                let mut h = fixed[q];
                let need = after.hand_size[q].saturating_sub(total(&h)) as usize;
                for _ in 0..need {
                    if k < bag.len() {
                        h[bag[k] as usize] += 1;
                        k += 1;
                    }
                }
                hand[q] = h;
            }
            out.push(ResParticle { w: 1.0 / np as f32, hand });
        }
        self.rng = rng;
        self.res = out;
    }

    /// 発展の粒子を、公開台帳（購入世代・使用）・自分の札・公開の枚数から作り直す
    fn rebuild_dev(&mut self) {
        let Some(after) = self.last_after else {
            let np = self.cfg.particles.max(1);
            let mut rng = self.rng;
            self.dev = (0..np)
                .map(|_| DevParticle {
                    w: 1.0 / np as f32,
                    cohorts: Default::default(),
                    deck: full_deck(),
                    style: std::array::from_fn(|_| if self.cfg.opp.use_styles { sample_style(&mut rng) } else { STYLE_BALANCED }),
                })
                .collect();
            self.rng = rng;
            return;
        };
        let n = self.n;
        let me = self.viewer as usize;
        let own_cohorts = self.shadow_cohorts.clone();
        let mut dpool = full_deck();
        for q in 0..n {
            for i in 0..NUM_DEV_KINDS {
                dpool[i] = dpool[i].saturating_sub(self.hist.ledgers[q].played_dev[i]);
            }
        }
        for c in &own_cohorts {
            for i in 0..NUM_DEV_KINDS {
                dpool[i] = dpool[i].saturating_sub(c.cards[i]);
            }
        }
        let np = self.cfg.particles.max(1);
        let mut rng = self.rng;
        let tp = after.turn_player as usize;
        let cap = if after.winner.is_none() { (VP_TO_WIN - 1).saturating_sub(after.public_vp[tp]) } else { 5 };
        let mut out = Vec::with_capacity(np);
        for _ in 0..np {
            let mut best: Option<DevParticle> = None;
            for _try in 0..20 {
                let pt = self.draw_dev_particle(&after, &own_cohorts, &dpool, &mut rng, np);
                let ok = after.winner.is_some() || tp == me || pt.hidden_vp(tp as PlayerId) <= cap;
                if ok {
                    best = Some(pt);
                    break;
                }
                if best.is_none() {
                    best = Some(pt);
                }
            }
            out.push(best.unwrap());
        }
        self.rng = rng;
        self.dev = out;
        self.reweight_dev_from_history();
    }

    fn draw_dev_particle(&self, after: &EventAftermath, own_cohorts: &[Cohort], dpool: &[u8; NUM_DEV_KINDS], rng: &mut Rng, np: usize) -> DevParticle {
        let n = self.n;
        let me = self.viewer as usize;
        let mut dbag: Vec<u8> = Vec::new();
        for i in 0..NUM_DEV_KINDS {
            for _ in 0..dpool[i] {
                dbag.push(i as u8);
            }
        }
        rng.shuffle(&mut dbag);
        let mut cohorts: [Vec<Cohort>; MAX_PLAYERS] = Default::default();
        cohorts[me] = own_cohorts.to_vec();
        let mut di = 0usize;
        for q in 0..n {
            if q == me {
                continue;
            }
            let l = &self.hist.ledgers[q];
            let mut slots: Vec<(u32, u32, u8)> = l
                .purchases
                .iter()
                .map(|pu| (pu.turn, l.turns_ended.saturating_sub(pu.turns_ended_before), pu.count))
                .collect();
            for &(pt, _kind) in &l.plays {
                let elig: Vec<usize> = slots.iter().enumerate().filter(|(_, s)| s.0 != pt && s.2 > 0).map(|(i, _)| i).collect();
                let pick = if elig.is_empty() {
                    slots.iter().position(|s| s.2 > 0)
                } else {
                    Some(elig[rng.below(elig.len() as u32) as usize])
                };
                if let Some(i) = pick {
                    slots[i].2 -= 1;
                }
            }
            let have: u8 = slots.iter().map(|s| s.2).sum();
            let want = after.dev_count[q];
            if have != want {
                if slots.is_empty() {
                    slots.push((self.hist.turn, 0, 0));
                }
                let last = slots.len() - 1;
                slots[last].2 = (slots[last].2 as i16 + (want as i16 - have as i16)).max(0) as u8;
            }
            let mut cs = Vec::new();
            for (t, own_turns, cnt) in slots {
                if cnt == 0 {
                    continue;
                }
                let mut cards = [0u8; NUM_DEV_KINDS];
                for _ in 0..cnt {
                    if di < dbag.len() {
                        cards[dbag[di] as usize] += 1;
                        di += 1;
                    }
                }
                cs.push(Cohort { bought_turn: t, own_turns, cards });
            }
            cohorts[q] = cs;
        }
        let mut deck = [0u8; NUM_DEV_KINDS];
        for &c in &dbag[di..] {
            deck[c as usize] += 1;
        }
        DevParticle {
            w: 1.0 / np as f32,
            cohorts,
            deck,
            style: std::array::from_fn(|_| if self.cfg.opp.use_styles { sample_style(rng) } else { STYLE_BALANCED }),
        }
    }

    /// 組み直した発展の粒子に、台帳の「使わなかった」尤度を掛け直す。
    ///
    /// 今持っている札は、買ってから今までずっと持っていたので、過去の各機会でも手札にあった。
    /// その後に使われた札（種類は公開）も、使う前の機会には手札にあった。
    /// この 2 つから各機会の「持っていた種類」を復元して尤度を掛ける（近似・明記）。
    fn reweight_dev_from_history(&mut self) {
        if !self.cfg.use_nonuse {
            return;
        }
        let viewer = self.viewer;
        let cfg = self.cfg;
        let n = self.n;
        let turns: Vec<crate::history::TurnRecord> = self.hist.turns.iter().copied().filter(|t| t.player != viewer).collect();
        let plays: Vec<Vec<(u32, DevCard)>> = (0..MAX_PLAYERS).map(|q| self.hist.ledgers[q].plays.clone()).collect();
        for pt in self.dev.iter_mut() {
            let mut logw = 0.0f32;
            for t in &turns {
                let a = t.player as usize;
                if t.dev_count <= t.bought_this_turn || t.played.is_some() {
                    continue;
                }
                let mut qmax = 0.0f32;
                for c in &pt.cohorts[a] {
                    if c.bought_turn >= t.turn {
                        continue;
                    }
                    // その機会の時点で保持していた自分の手番数（全体の手番番号の差を人数で割る）
                    let held = ((t.turn - c.bought_turn) / n as u32).max(1);
                    for kind in DEV_CARDS {
                        if c.cards[kind.idx()] > 0 && kind != DevCard::VictoryPoint {
                            qmax = qmax.max(q_use(kind, &t.opportunity, pt.style[a], held, &cfg.opp));
                        }
                    }
                }
                for &(pt_turn, kind) in &plays[a] {
                    if pt_turn > t.turn {
                        // 使われた札の購入時期は世代の対応が不明なので、期間 1 の率で近似
                        qmax = qmax.max(q_use(kind, &t.opportunity, pt.style[a], 1, &cfg.opp));
                    }
                }
                logw += (1.0 - qmax).max(1e-6).ln();
            }
            pt.w *= logw.exp();
        }
        let mut ws: Vec<f32> = self.dev.iter().map(|p| p.w).collect();
        if normalize(&mut ws).is_some() {
            for (p, w) in self.dev.iter_mut().zip(ws.iter()) {
                p.w = *w;
            }
        } else {
            let np = self.dev.len().max(1) as f32;
            for p in self.dev.iter_mut() {
                p.w = 1.0 / np;
            }
        }
    }

    // ------------------------------------------------------------ 出力

    /// 観測と粒子が食い違っていないか（出来事の欠落の検出）。食い違えば作り直す。
    pub fn resync_if_needed(&mut self, obs: &Observation) {
        let n = obs.num_players();
        let me = self.viewer as usize;
        let res_ok = self.res.iter().any(|pt| pt.w > 0.0 && (0..n).all(|q| total(&pt.hand[q]) == obs.public.players[q].hand_size) && pt.hand[me] == obs.own.hand);
        let dev_ok = self
            .dev
            .iter()
            .any(|pt| pt.w > 0.0 && (0..n).all(|q| pt.dev_total(q as PlayerId) == obs.public.players[q].dev_count) && pt.dev(me as PlayerId) == obs.own.dev);
        if !res_ok || !dev_ok {
            self.resync(obs);
        }
    }

    /// `P(隠れ勝利点 = k)`（k = 0..=5）
    pub fn vp_dist(&self, p: PlayerId) -> [f32; 6] {
        let mut out = [0f32; 6];
        for pt in &self.dev {
            out[(pt.hidden_vp(p) as usize).min(5)] += pt.w;
        }
        out
    }
    pub fn expected_hidden_vp(&self, p: PlayerId) -> f32 {
        self.vp_dist(p).iter().enumerate().map(|(k, w)| k as f32 * w).sum()
    }
    /// `P(実際の勝利点 ≥ k)`（公開点は観測から渡す）
    pub fn prob_actual_vp_at_least(&self, p: PlayerId, public_vp: u8, k: u8) -> f32 {
        let need = k.saturating_sub(public_vp) as usize;
        self.vp_dist(p).iter().enumerate().filter(|(h, _)| *h >= need).map(|(_, w)| *w).sum()
    }
    pub fn style_dist(&self, p: PlayerId) -> [f32; NUM_STYLES] {
        let mut out = [0f32; NUM_STYLES];
        for pt in &self.dev {
            out[pt.style[p as usize] as usize % NUM_STYLES] += pt.w;
        }
        out
    }
    /// 相手 `p` の資源の期待枚数
    pub fn expected_hand(&self, p: PlayerId) -> [f32; NUM_RESOURCES] {
        let mut out = [0f32; NUM_RESOURCES];
        for pt in &self.res {
            for r in 0..NUM_RESOURCES {
                out[r] += pt.w * pt.hand[p as usize][r] as f32;
            }
        }
        out
    }
    /// 相手 `p` の手札が `hand` である確率
    pub fn prob_hand(&self, p: PlayerId, hand: &Bundle) -> f32 {
        self.res.iter().filter(|pt| pt.hand[p as usize] == *hand).map(|pt| pt.w).sum()
    }
    pub fn ess_res(&self) -> f32 {
        let sq: f64 = self.res.iter().map(|p| (p.w as f64) * (p.w as f64)).sum();
        if sq > 0.0 { (1.0 / sq) as f32 } else { 0.0 }
    }
    pub fn ess_dev(&self) -> f32 {
        let sq: f64 = self.dev.iter().map(|p| (p.w as f64) * (p.w as f64)).sum();
        if sq > 0.0 { (1.0 / sq) as f32 } else { 0.0 }
    }
    /// 有効粒子数（資源・発展の小さい方）
    pub fn ess(&self) -> f32 {
        self.ess_res().min(self.ess_dev())
    }

    /// 重みに比例して (資源, 発展) の粒子の対を `k` 組選ぶ。同じ粒子が複数回選ばれることもある
    pub fn sample_indices(&mut self, k: usize, rng: &mut Rng) -> Vec<(usize, usize)> {
        let k = k.max(1);
        let wr: Vec<f32> = self.res.iter().map(|p| p.w).collect();
        let wd: Vec<f32> = self.dev.iter().map(|p| p.w).collect();
        let lr: Vec<usize> = (0..wr.len()).filter(|&i| wr[i] > 0.0).collect();
        let ld: Vec<usize> = (0..wd.len()).filter(|&i| wd[i] > 0.0).collect();
        let ri = if lr.is_empty() { vec![0; k] } else { systematic_pick(&wr, &lr, k, rng) };
        let mut di = if ld.is_empty() { vec![0; k] } else { systematic_pick(&wd, &ld, k, rng) };
        // 対の作り方に相関を持ち込まないよう、発展側を混ぜる
        rng.shuffle(&mut di);
        ri.into_iter().zip(di).collect()
    }

    /// 粒子の対 `(ri, di)` を観測に重ねて、探索できる完全情報の局面を作る。
    /// 推論用 RNG から本番とは無関係な乱数の流れを与える。
    pub fn materialize(&self, obs: &Observation, idx: (usize, usize), rng: &mut Rng) -> Game {
        let pr = &self.res[idx.0];
        let pd = &self.dev[idx.1];
        let n = obs.num_players();
        let me = self.viewer as usize;
        let mut g = obs.public.game.clone();
        let turn = g.turn;
        for q in 0..n {
            g.players[q].hand = pr.hand[q];
            g.players[q].dev = pd.dev(q as PlayerId);
            g.players[q].dev_bought_this_turn = pd.dev_bought_at(q as PlayerId, turn);
        }
        g.players[me].hand = obs.own.hand;
        g.players[me].dev = obs.own.dev;
        g.players[me].dev_bought_this_turn = obs.own.dev_bought_this_turn;
        g.dev_deck = pd.deck;
        g.bank = match obs.public.bank {
            Some(b) => b,
            None => {
                let mut b = [BANK_PER_RESOURCE; NUM_RESOURCES];
                for q in 0..n {
                    for r in 0..NUM_RESOURCES {
                        b[r] = b[r].saturating_sub(g.players[q].hand[r]);
                    }
                }
                b
            }
        };
        g.rng_dice = Rng::new(rng.next_u32() as u64);
        g.rng_dev = Rng::new(rng.next_u32() as u64);
        g.rng_steal = Rng::new(rng.next_u32() as u64);
        debug_assert!((0..n).all(|q| g.players[q].hand_size() == obs.public.players[q].hand_size), "粒子と観測の枚数が合わない");
        g
    }

    /// 相手 `p` の隠れ状態（資源, 発展, 山, 他人の資源合計, 重み）を、同じ状態ごとにまとめて返す。
    ///
    /// 資源側と発展側の直積を重みの大きい順に最大 `cap` 件まで取り、重みを正規化し直す（近似・明記）。
    pub fn distinct_states_capped(&self, p: PlayerId, cap: usize) -> Vec<(Bundle, [u8; NUM_DEV_KINDS], [u8; NUM_DEV_KINDS], Bundle, f32)> {
        // 資源: (hand[p], others) ごと
        let mut rs: Vec<(Bundle, Bundle, f32)> = Vec::new();
        for pt in &self.res {
            if pt.w <= 0.0 {
                continue;
            }
            let h = pt.hand[p as usize];
            let mut others = EMPTY;
            for q in 0..self.n {
                if q != p as usize {
                    add(&mut others, &pt.hand[q]);
                }
            }
            if let Some(e) = rs.iter_mut().find(|e| e.0 == h && e.1 == others) {
                e.2 += pt.w;
            } else {
                rs.push((h, others, pt.w));
            }
        }
        // 発展: (dev[p], deck) ごと
        let mut ds: Vec<([u8; NUM_DEV_KINDS], [u8; NUM_DEV_KINDS], f32)> = Vec::new();
        for pt in &self.dev {
            if pt.w <= 0.0 {
                continue;
            }
            let d = pt.dev(p);
            if let Some(e) = ds.iter_mut().find(|e| e.0 == d && e.1 == pt.deck) {
                e.2 += pt.w;
            } else {
                ds.push((d, pt.deck, pt.w));
            }
        }
        let mut out: Vec<(Bundle, [u8; NUM_DEV_KINDS], [u8; NUM_DEV_KINDS], Bundle, f32)> = Vec::with_capacity(rs.len() * ds.len());
        for (h, others, wr) in &rs {
            for (d, deck, wd) in &ds {
                out.push((*h, *d, *deck, *others, wr * wd));
            }
        }
        out.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(cap.max(1));
        let sum: f32 = out.iter().map(|e| e.4).sum();
        if sum > 0.0 {
            for e in out.iter_mut() {
                e.4 /= sum;
            }
        }
        out
    }

    pub fn distinct_states(&self, p: PlayerId) -> Vec<(Bundle, [u8; NUM_DEV_KINDS], [u8; NUM_DEV_KINDS], Bundle, f32)> {
        self.distinct_states_capped(p, 48)
    }
}

/// 資源の粒子に出来事を適用する。
///
/// `strict` が真なら、ルール上あり得ない粒子（払えない・持っていない）は重み 0 にして打ち切る。
/// 偽（影 = 自分の手札の追跡用）なら、相手側の仮説が合わなくても自分の分の更新は必ず通す。
fn apply_res(pt: &mut ResParticle, ev: &ObservedEvent, _viewer: PlayerId, n: usize, open_want: Option<Bundle>, rng: &mut Rng, strict: bool) {
    use VisibleEvent::*;
    let a = ev.actor as usize;
    macro_rules! fail {
        () => {{
            if strict {
                pt.w = 0.0;
                return;
            }
        }};
    }
    match ev.payload {
        SetupSettlement { gained, .. } => add(&mut pt.hand[a], &gained),
        Roll { produced, .. } => {
            for q in 0..n {
                add(&mut pt.hand[q], &produced[q]);
            }
        }
        Discard { count, bundle } => match bundle {
            Some(b) => {
                if !take(&mut pt.hand[a], &b) {
                    fail!();
                }
            }
            None => {
                // 内訳が非公開: 相手の廃棄方策は不明なので枚数比で抜く（近似・明記）
                for _ in 0..count {
                    match sample_resource(&pt.hand[a], rng) {
                        Some(r) => pt.hand[a][r] -= 1,
                        None => fail!(),
                    }
                }
            }
        },
        RobberMoved { victim: Some(v), stole_any: true, stolen, .. } => {
            let vi = v as usize;
            match stolen {
                Some(r) => {
                    if pt.hand[vi][r.idx()] == 0 {
                        fail!();
                    }
                    pt.hand[vi][r.idx()] = pt.hand[vi][r.idx()].saturating_sub(1);
                    pt.hand[a][r.idx()] += 1;
                }
                None => match sample_resource(&pt.hand[vi], rng) {
                    Some(r) => {
                        pt.hand[vi][r] -= 1;
                        pt.hand[a][r] += 1;
                    }
                    None => fail!(),
                },
            }
        }
        BuildRoad { free, .. } => {
            if !free && !take(&mut pt.hand[a], &COST_ROAD) {
                fail!();
            }
        }
        BuildSettlement { .. } => {
            if !take(&mut pt.hand[a], &COST_SETTLEMENT) {
                fail!();
            }
        }
        BuildCity { .. } => {
            if !take(&mut pt.hand[a], &COST_CITY) {
                fail!();
            }
        }
        BuyDev { .. } => {
            if !take(&mut pt.hand[a], &COST_DEV) {
                fail!();
            }
        }
        PlayYearOfPlenty { a: r1, b: r2 } => {
            pt.hand[a][r1.idx()] += 1;
            pt.hand[a][r2.idx()] += 1;
        }
        PlayMonopoly { res, taken } => {
            let r = res.idx();
            let mut sum = 0u8;
            for q in 0..n {
                if q == a {
                    continue;
                }
                if pt.hand[q][r] != taken[q] {
                    fail!();
                }
                sum += taken[q];
                pt.hand[q][r] = 0;
            }
            pt.hand[a][r] += sum;
        }
        Maritime { give, count, take: tk } => {
            if pt.hand[a][give.idx()] < count {
                fail!();
            }
            pt.hand[a][give.idx()] = pt.hand[a][give.idx()].saturating_sub(count);
            pt.hand[a][tk.idx()] += 1;
        }
        Offer { give, .. } => {
            if !contains(&pt.hand[a], &give) {
                fail!();
            }
        }
        Accept => {
            if let Some(want) = open_want {
                if !contains(&pt.hand[a], &want) {
                    fail!();
                }
            }
        }
        Counter { give, .. } | CounterAlt { give, .. } => {
            if !contains(&pt.hand[a], &give) {
                fail!();
            }
        }
        Confirm { partner, give, want } => {
            let pi = partner as usize;
            if !(contains(&pt.hand[a], &give) && contains(&pt.hand[pi], &want)) {
                fail!();
            }
            for r in 0..NUM_RESOURCES {
                pt.hand[a][r] = pt.hand[a][r].saturating_sub(give[r]) + want[r];
                pt.hand[pi][r] = pt.hand[pi][r].saturating_sub(want[r]) + give[r];
            }
        }
        AcceptCounter { partner, partner_gives, proposer_gives, .. } => {
            let pi = partner as usize;
            if !(contains(&pt.hand[pi], &partner_gives) && contains(&pt.hand[a], &proposer_gives)) {
                fail!();
            }
            for r in 0..NUM_RESOURCES {
                pt.hand[pi][r] = pt.hand[pi][r].saturating_sub(partner_gives[r]) + proposer_gives[r];
                pt.hand[a][r] = pt.hand[a][r].saturating_sub(proposer_gives[r]) + partner_gives[r];
            }
        }
        SetupRoad { .. } | RobberMoved { .. } | PlayKnight | PlayRoadBuilding | Cancel | Reject | CounterAltRemove { .. } | Hidden | EndTurn => {}
    }
}

/// 発展の粒子に出来事を適用する（資源は触らない）
#[allow(clippy::too_many_arguments)]
fn apply_dev(pt: &mut DevParticle, ev: &ObservedEvent, viewer: PlayerId, n: usize, cfg: &BeliefConfig, merged_opp: Option<Opportunity>, rng: &mut Rng, strict: bool) {
    use VisibleEvent::*;
    let a = ev.actor as usize;
    match ev.payload {
        BuyDev { card } => {
            let c = match card {
                Some(c) => c,
                None => match sample_deck(&pt.deck, rng) {
                    Some(c) => c,
                    None => {
                        if strict {
                            pt.w = 0.0;
                            return;
                        }
                        DevCard::Knight
                    }
                },
            };
            if pt.deck[c.idx()] == 0 && strict {
                pt.w = 0.0;
                return;
            }
            pt.deck[c.idx()] = pt.deck[c.idx()].saturating_sub(1);
            match pt.cohorts[a].iter_mut().find(|co| co.bought_turn == ev.turn) {
                Some(co) => co.cards[c.idx()] += 1,
                None => {
                    let mut cards = [0u8; NUM_DEV_KINDS];
                    cards[c.idx()] = 1;
                    pt.cohorts[a].push(Cohort { bought_turn: ev.turn, own_turns: 0, cards });
                }
            }
        }
        PlayKnight => play(pt, ev, viewer, n, cfg, DevCard::Knight, rng, strict),
        PlayRoadBuilding => play(pt, ev, viewer, n, cfg, DevCard::RoadBuilding, rng, strict),
        PlayYearOfPlenty { .. } => play(pt, ev, viewer, n, cfg, DevCard::YearOfPlenty, rng, strict),
        PlayMonopoly { .. } => play(pt, ev, viewer, n, cfg, DevCard::Monopoly, rng, strict),
        EndTurn => {
            nonuse(pt, ev, viewer, n, cfg, merged_opp);
            for co in pt.cohorts[a].iter_mut() {
                co.own_turns += 1;
            }
        }
        _ => {}
    }
    if !strict || pt.w <= 0.0 {
        return;
    }
    // 手番プレイヤーが勝っていない ⇒ その人の伏せ勝利点はこれ以下（正しいタイミングだけ）。
    // 初期配置中は発展カードが無いので自明に満たす
    if ev.after.winner.is_none() {
        let tp = ev.after.turn_player;
        let cap = (VP_TO_WIN - 1).saturating_sub(ev.after.public_vp[tp as usize]);
        if pt.hidden_vp(tp) > cap {
            pt.w = 0.0;
        }
    }
}

/// 発展カードの使用。どの世代から使われたかは仮説として選ぶ（決め打ちしない）
#[allow(clippy::too_many_arguments)]
fn play(pt: &mut DevParticle, ev: &ObservedEvent, viewer: PlayerId, n: usize, cfg: &BeliefConfig, kind: DevCard, rng: &mut Rng, strict: bool) {
    let a = ev.actor as usize;
    let k = kind.idx();
    let elig: u32 = pt.cohorts[a].iter().filter(|c| c.bought_turn != ev.turn).map(|c| c.cards[k] as u32).sum();
    if elig == 0 {
        if strict {
            pt.w = 0.0;
        }
        return;
    }
    let mut pick = rng.below(elig);
    let mut done = false;
    let mut held = 1u32;
    for c in pt.cohorts[a].iter_mut() {
        if c.bought_turn == ev.turn || c.cards[k] == 0 {
            continue;
        }
        if pick < c.cards[k] as u32 {
            c.cards[k] -= 1;
            held = c.own_turns.max(1);
            done = true;
            break;
        }
        pick -= c.cards[k] as u32;
    }
    debug_assert!(done);
    pt.cohorts[a].retain(|c| c.total() > 0);
    if ev.actor != viewer {
        let x = Opportunity::from_context(&ev.before, ev.actor, n);
        let q = q_use(kind, &x, pt.style[a], held, &cfg.opp);
        pt.w *= q.max(cfg.opp.floor);
    }
}

/// 「使用可能な自分の手番を終えたのに、何も使わなかった」尤度（手札単位・1 手番 1 枚）
fn nonuse(pt: &mut DevParticle, ev: &ObservedEvent, viewer: PlayerId, n: usize, cfg: &BeliefConfig, merged: Option<Opportunity>) {
    if ev.actor == viewer || !cfg.use_nonuse || ev.before.dev_played_this_turn {
        return;
    }
    let a = ev.actor as usize;
    let x = merged.unwrap_or_else(|| Opportunity::from_context(&ev.before, ev.actor, n));
    let mut qmax = 0.0f32;
    for c in &pt.cohorts[a] {
        if c.bought_turn == ev.turn {
            continue; // 買った手番は使えない
        }
        // own_turns はこの EndTurn の後で +1 されるので、買った次の手番の機会では 1
        let held = c.own_turns.max(1);
        for kind in DEV_CARDS {
            if c.cards[kind.idx()] > 0 && kind != DevCard::VictoryPoint {
                qmax = qmax.max(q_use(kind, &x, pt.style[a], held, &cfg.opp));
            }
        }
    }
    pt.w *= 1.0 - qmax;
}
