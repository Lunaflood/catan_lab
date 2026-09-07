//! 全局面の評価関数。
//!
//! JSettlers 系の「ビルドプランに固定の優先順位を持たせる」設計をやめ、
//! すべての手を同じ評価関数で並べる。Guhe & Lascarides 2014 は
//! 固定順（都市 ≻ 開拓地 ≻ 最大騎士力 ≻ 最長路 ≻ 発展カード）を
//! 状況依存のランキングに置き換えるだけで勝率 23.4% → 29.9% を得ている。
//!
//! catanatron のリーダーボードでも「深さ2の alpha-beta + 手作り評価関数」が
//! MCTS より強い。カタンでは**良い評価関数 × 浅い探索**が費用対効果で勝つ。

use catan_core::action::{Action, Bundle, DevCard, NUM_DEV_KINDS};
use catan_core::board::{Board, BuildingKind, PlayerId, Resource, NUM_RESOURCES};
use catan_core::game::{Forced, Game, MAX_PLAYERS, VP_TO_WIN};
use catan_core::topology::{EdgeId, NodeId, Topology, NUM_EDGES, NUM_NODES};

/// 1 pip = 2.778% = 1/36。産出を「1 ターンあたりの期待枚数」に直す係数。
const PIP: f32 = 1.0 / 36.0;

#[derive(Clone, Copy, Debug)]
pub struct EvalWeights {
    /// 勝利点。勝ちは他の何より優先されるので桁を分ける
    pub vp: f32,
    /// 決着（勝ち = +、負け = −）
    pub terminal: f32,
    /// 自分の期待産出（盗賊を考慮）
    pub production: f32,
    /// 資源 1 種を新たに持つことの価値（産出何 pip 相当か）
    pub variety: f32,
    /// 相手の期待産出（負の重み）。妨害はここに入る
    pub enemy_production: f32,
    /// 相手の手札の充実度（負）。「リーダーが今にも建てられる」を捉える。
    /// 資源の手札は見える方針なので、これは正当な情報
    pub enemy_hand: f32,
    /// 相手の脅威度を勝利点で割り増しする度合い。
    /// 「リーダーの一番良いヘクスを止める」を評価関数の言葉にしたもの
    pub leader_bias: f32,
    /// いま建てられる場所の産出
    pub expansion_now: f32,
    /// 道 1 本で届く場所の産出
    pub expansion_next: f32,
    /// 建設可能な頂点の数（囲まれていないこと）
    pub buildable_nodes: f32,
    /// 手札が次の建設にどれだけ噛み合っているか（連続量）
    pub hand_synergy: f32,
    /// **いま実際に払えるか**（離散）。
    /// 連続量だけで交易を評価すると、微差を埋める 1:1 交換が延々と正に見えて
    /// 1 試合で 1,000 回以上交易する（実測）。実際の交易は「あと 1 枚で建つ」を
    /// 埋めるために行われるので、そこに段差を作る。
    pub afford_city: f32,
    pub afford_settlement: f32,
    pub afford_dev: f32,
    /// 道が払えること。**実測では入れると弱い**（0.6 で 20.8%、0 で 27.3%）。
    /// 道はそれ自体が目的ではないので、手札の「力」に数えると判断が濁る。
    pub afford_road: f32,
    /// 1 回の交易そのものの代価（手番の持ち時間・手札の情報を晒す分）。
    /// 離散の段差（`afford_*`）を入れる前は、これが無いと 1 試合で 1,000 回以上
    /// 交換して手番が終わらなかった。段差を入れた後は 0〜6 のどこでも差が出ない
    /// （実測 2,400 戦）ので、交渉が死なない 0 側に置いてある。
    pub trade_cost: f32,
    pub hand_size: f32,
    /// 8 枚以上持っている時の罰
    pub discard_risk: f32,
    pub dev_in_hand: f32,
    pub army: f32,
    /// 最長路の長さ。拡張余地がある間は小さく、詰まったら大きく効かせる
    pub longest_road: f32,
    pub longest_road_when_boxed: f32,
    /// 接している異なるヘクスの数
    pub num_tiles: f32,
}

impl EvalWeights {
    /// 最初に手で決めた重み。ここから ablation と掃引で v2 に至った経緯の記録。
    ///
    /// この版は **4 体で回すと 37% の対局が膠着した**。全員が初期 2 軒を都市に変え、
    /// 拡張用の道を建てず（道は即時 VP を生まない）、発展カードの山も尽きて誰も 10 点に届かない。
    /// VpGreedy が道を 1 本も建てなかったのと同じ病が、桁を変えて再発していた。
    pub fn v1() -> Self {
        EvalWeights {
            expansion_now: 40.0,
            buildable_nodes: 3.0,
            ..EvalWeights::default()
        }
    }
}

impl EvalWeights {
    /// 自動調整で出した重み（2026-09-07）。
    ///
    /// 手で決めた既定値に対し、**据え置きの相手 3 体に当てた勝率が 24.5% → 55.7%**。
    /// 掃引に使っていない 4 つの seed で 1,500 戦ずつ検証しても
    /// 49.0〜51.1%（既定は 22.8〜23.6%）で、+25.5〜+28.0pt の再現があった。
    ///
    /// 傾向は一貫していて、**妨害を軽く・自分の手札を重く**。
    /// - `enemy_hand` −10 → −0.735、`leader_bias` 0.15 → 0.0525
    ///   ＝「相手の手札の充実」「リーダー狙い」を見て動くのは、ほぼ損だった
    /// - `hand_synergy` 20 → 89.6、`hand_size` 1 → 7.17
    ///   ＝手札そのものの厚みと噛み合いが、盤面の潜在力より効く
    /// - `expansion_now` 600 → 210 ＝拡張余地は「膠着しない程度」に効かせれば足りる
    /// - `afford_settlement` 1 → 0.206 / `afford_city` 1.2 → 3.29
    ///   ＝払えることに段差を付けるなら **都市に付けろ**（開拓地は建て急がない方が良い）
    pub fn tuned() -> Self {
        EvalWeights {
            enemy_hand: -0.735,
            hand_synergy: 89.6,
            leader_bias: 0.0525,
            expansion_now: 210.0,
            hand_size: 7.168,
            variety: 1.4,
            discard_risk: -2.8,
            afford_settlement: 0.2058,
            afford_city: 3.2928,
            ..EvalWeights::default()
        }
    }
}

impl Default for EvalWeights {
    /// 実測で詰めた重み（各条件 1,200 戦・相手は同じボット）。
    ///
    /// 決め手は 2 つとも「拡張余地」だった:
    /// - `buildable_nodes` 3 → 200 で **膠着 35% → 0%・勝率 23.8% → 85.7%**
    ///   （640 まで上げると今度は開拓地を建てなくなって 61.8% に崩れる）
    /// - `expansion_now` 40 → 600 で膠着 35% → 1%
    ///
    /// `vp` は 200〜2500 のどこでも結果が変わらなかった。
    /// 支配的でありさえすれば大きさに意味は無い。
    fn default() -> Self {
        EvalWeights {
            vp: 1000.0,
            terminal: 100_000.0,
            production: 300.0,
            variety: 4.0,
            enemy_production: -120.0,
            enemy_hand: -10.0,
            leader_bias: 0.15,
            expansion_now: 600.0,
            expansion_next: 15.0,
            buildable_nodes: 200.0,
            hand_synergy: 20.0,
            afford_city: 1.2,
            afford_settlement: 1.0,
            afford_dev: 0.5,
            afford_road: 0.0,
            trade_cost: 0.0,
            hand_size: 1.0,
            discard_risk: -8.0,
            dev_in_hand: 6.0,
            army: 8.0,
            longest_road: 1.0,
            longest_road_when_boxed: 15.0,
            num_tiles: 1.5,
        }
    }
}

impl EvalWeights {
    pub const ALL_TERMS: [&'static str; 22] = [
        "vp",
        "terminal",
        "production",
        "variety",
        "enemy_production",
        "enemy_hand",
        "leader_bias",
        "expansion_now",
        "expansion_next",
        "buildable_nodes",
        "hand_synergy",
        "afford_city",
        "afford_settlement",
        "afford_dev",
        "afford_road",
        "trade_cost",
        "hand_size",
        "discard_risk",
        "dev_in_hand",
        "army",
        "longest_road",
        "num_tiles",
    ];

    /// 名前で 1 項目を読む（自動調整で「今の値の何倍か」を作るのに要る）
    pub fn get(&self, name: &str) -> f32 {
        match name {
            "vp" => self.vp,
            "terminal" => self.terminal,
            "production" => self.production,
            "variety" => self.variety,
            "enemy_production" => self.enemy_production,
            "enemy_hand" => self.enemy_hand,
            "leader_bias" => self.leader_bias,
            "expansion_now" => self.expansion_now,
            "expansion_next" => self.expansion_next,
            "buildable_nodes" => self.buildable_nodes,
            "hand_synergy" => self.hand_synergy,
            "afford_city" => self.afford_city,
            "afford_settlement" => self.afford_settlement,
            "afford_dev" => self.afford_dev,
            "afford_road" => self.afford_road,
            "trade_cost" => self.trade_cost,
            "hand_size" => self.hand_size,
            "discard_risk" => self.discard_risk,
            "dev_in_hand" => self.dev_in_hand,
            "army" => self.army,
            "longest_road" => self.longest_road,
            "longest_road_when_boxed" => self.longest_road_when_boxed,
            "num_tiles" => self.num_tiles,
            other => panic!("知らない項目: {other}"),
        }
    }

    pub fn set(&mut self, name: &str, v: f32) {
        match name {
            "vp" => self.vp = v,
            "terminal" => self.terminal = v,
            "production" => self.production = v,
            "variety" => self.variety = v,
            "enemy_production" => self.enemy_production = v,
            "enemy_hand" => self.enemy_hand = v,
            "leader_bias" => self.leader_bias = v,
            "expansion_now" => self.expansion_now = v,
            "expansion_next" => self.expansion_next = v,
            "buildable_nodes" => self.buildable_nodes = v,
            "hand_synergy" => self.hand_synergy = v,
            "afford_city" => self.afford_city = v,
            "afford_settlement" => self.afford_settlement = v,
            "afford_dev" => self.afford_dev = v,
            "afford_road" => self.afford_road = v,
            "trade_cost" => self.trade_cost = v,
            "hand_size" => self.hand_size = v,
            "discard_risk" => self.discard_risk = v,
            "dev_in_hand" => self.dev_in_hand = v,
            "army" => self.army = v,
            "longest_road" => {
                self.longest_road = v;
                self.longest_road_when_boxed = v * 15.0;
            }
            "num_tiles" => self.num_tiles = v,
            other => panic!("知らない項目: {other}"),
        }
    }

    pub fn without(mut self, name: &str) -> Self {
        self.set(name, 0.0);
        self
    }
}

/// 盗賊を考慮した、資源別の期待産出（pip 単位・都市は 2 倍）
pub fn production_pips(board: &Board, p: PlayerId) -> [u16; NUM_RESOURCES] {
    let topo = Topology::get();
    let mut out = [0u16; NUM_RESOURCES];
    for n in 0..NUM_NODES {
        let Some(b) = board.building[n] else { continue };
        if b.owner != p {
            continue;
        }
        let mult: u16 = match b.kind {
            BuildingKind::Settlement => 1,
            BuildingKind::City => 2,
        };
        for t in &topo.node_tiles[n] {
            if t == board.robber {
                continue; // 盗賊が止めている
            }
            if let Some(r) = board.tile_resource[t as usize] {
                out[r.idx()] += mult * board.tile_pips(t) as u16;
            }
        }
    }
    out
}

/// 産出の総合値。総量に加えて「種類が揃っていること」を評価する。
fn production_value(pips: &[u16; NUM_RESOURCES], variety_w: f32) -> f32 {
    let total: u16 = pips.iter().sum();
    let kinds = pips.iter().filter(|&&v| v > 0).count() as f32;
    (total as f32 + kinds * variety_w) * PIP
}

/// 距離ルールを満たし空いている頂点か
#[inline]
fn is_open_spot(board: &Board, n: NodeId) -> bool {
    let topo = Topology::get();
    board.building[n as usize].is_none()
        && topo.node_neighbors[n as usize]
            .into_iter()
            .all(|m| board.building[m as usize].is_none())
}

/// 自分の道網から届く拡張先。
/// 戻り値は (いま建てられる場所の pip 合計, 道 1 本先の pip 合計, 建設可能頂点数)
fn expansion(board: &Board, p: PlayerId) -> (f32, f32, f32) {
    let topo = Topology::get();

    // 自分の道・建物が触れている頂点
    let mut frontier = [false; NUM_NODES];
    for e in 0..NUM_EDGES {
        if board.road[e] == Some(p) {
            for n in topo.edge_nodes[e] {
                frontier[n as usize] = true;
            }
        }
    }
    for n in 0..NUM_NODES {
        if let Some(b) = board.building[n] {
            if b.owner == p {
                frontier[n] = true;
            }
        }
    }

    let mut now = 0f32;
    let mut count = 0f32;
    for n in 0..NUM_NODES {
        if frontier[n] && is_open_spot(board, n as NodeId) {
            let pips: u16 = board.node_pips(n as NodeId).iter().map(|&v| v as u16).sum();
            now += pips as f32;
            count += 1.0;
        }
    }

    // 道 1 本先（相手の建物では止まる）
    let mut next = 0f32;
    for n in 0..NUM_NODES {
        if !frontier[n] {
            continue;
        }
        // 相手の建物がある頂点からは先へ伸ばせない
        if matches!(board.building[n], Some(b) if b.owner != p) {
            continue;
        }
        for m in &topo.node_neighbors[n] {
            if frontier[m as usize] || !is_open_spot(board, m) {
                continue;
            }
            let pips: u16 = board.node_pips(m).iter().map(|&v| v as u16).sum();
            next += pips as f32;
        }
    }

    (now * PIP, next * PIP, count)
}

/// 手札が「次の一手」にどれだけ近いか。0（遠い）〜1（すぐ建つ）。
fn hand_synergy(hand: &Bundle) -> f32 {
    let need = |have: u8, want: u8| (want.saturating_sub(have)) as f32;
    // 都市: 麦2 鉄3
    let to_city = (need(hand[Resource::Wheat.idx()], 2) + need(hand[Resource::Ore.idx()], 3)) / 5.0;
    // 開拓地: 木1 土1 羊1 麦1
    let to_settlement = (need(hand[Resource::Wood.idx()], 1)
        + need(hand[Resource::Brick.idx()], 1)
        + need(hand[Resource::Sheep.idx()], 1)
        + need(hand[Resource::Wheat.idx()], 1))
        / 4.0;
    (2.0 - to_city - to_settlement) / 2.0
}

/// 全員の脅威度。勝利点が高いほど、その相手を利する手の値段が上がる。
///
/// `public_vp` は 54 頂点を走査するので、1 手ごとに呼ぶと効かなくなる
/// （提案は 1 手番に数十通り並び、その各々で候補相手ぶん呼ばれる）。
/// 交易は勝利点を動かさないので、1 手番に 1 度計算して使い回せる。
pub fn threats(g: &Game, w: &EvalWeights) -> [f32; MAX_PLAYERS] {
    let mut out = [1.0; MAX_PLAYERS];
    for q in 0..g.n() {
        out[q] = 1.0 + w.leader_bias * g.public_vp(q as PlayerId) as f32;
    }
    out
}

#[inline]
fn affords(hand: &Bundle, cost: &Bundle) -> bool {
    (0..NUM_RESOURCES).all(|i| hand[i] >= cost[i])
}

/// 手札の「力」。連続量（あと何枚か）と離散量（いま払えるか）の両方を見る。
///
/// 離散の段差が要点。連続量だけだと、微差を埋めるだけの 1:1 交換が
/// いつでもわずかに正に見えてしまう。
pub fn hand_potential(hand: &Bundle, w: &EvalWeights) -> f32 {
    let mut v = hand_synergy(hand);
    if affords(hand, &catan_core::game::COST_CITY) {
        v += w.afford_city;
    }
    if affords(hand, &catan_core::game::COST_SETTLEMENT) {
        v += w.afford_settlement;
    }
    if affords(hand, &catan_core::game::COST_DEV) {
        v += w.afford_dev;
    }
    if affords(hand, &catan_core::game::COST_ROAD) {
        v += w.afford_road;
    }
    v
}

/// 手札に由来する項だけ。交易の差分評価で使い回す。
///
/// 自分の手札は「何が建てられる状態か・持ちすぎていないか」、
/// 相手の手札は「今にも建てられる状態か」を脅威度で重み付けして見る。
#[inline]
pub fn hand_terms(hand: &Bundle, is_me: bool, threat: f32, w: &EvalWeights) -> f32 {
    let pot = hand_potential(hand, w);
    if is_me {
        let n: u8 = hand.iter().sum();
        let mut v = w.hand_synergy * pot + w.hand_size * n as f32;
        if n > catan_core::game::HAND_LIMIT {
            v += w.discard_risk * (n - catan_core::game::HAND_LIMIT) as f32;
        }
        v
    } else {
        w.enemy_hand * pot * threat
    }
}

/// 局面を `me` の視点で評価する。
pub fn evaluate(g: &Game, me: PlayerId, w: &EvalWeights) -> f32 {
    let topo = Topology::get();
    let board = &g.board;

    // --- 決着 ---
    if let Some(win) = g.winner {
        return if win == me { w.terminal } else { -w.terminal };
    }

    let mut score = 0.0f32;

    // --- 勝利点 ---
    score += w.vp * g.actual_vp(me) as f32;

    // --- 産出 ---
    let mine = production_pips(board, me);
    score += w.production * production_value(&mine, w.variety);

    // --- 相手の産出（脅威度で重み付け）---
    let th = threats(g, w);
    let mut enemy = 0.0f32;
    let mut enemy_hands = 0.0f32;
    for q in 0..g.n() {
        let e = q as PlayerId;
        if e == me {
            continue;
        }
        let p = production_pips(board, e);
        // 勝利点が高い相手ほど、その産出を潰す価値が高い
        let threat = th[q];
        enemy += production_value(&p, w.variety) * threat;
        enemy_hands += hand_terms(&g.players[q].hand, false, threat, w);
    }
    score += w.enemy_production * enemy;
    score += enemy_hands;

    // --- 拡張 ---
    let (now, next, count) = expansion(board, me);
    score += w.expansion_now * now;
    score += w.expansion_next * next;
    score += w.buildable_nodes * count;

    // --- 手札 ---
    let ps = &g.players[me as usize];
    score += hand_terms(&ps.hand, true, 0.0, w);
    score += w.dev_in_hand * (ps.dev_count() - ps.dev[DevCard::VictoryPoint.idx()]) as f32;
    score += w.army * ps.played_knights() as f32;

    // --- 最長路: 拡張余地が尽きている時だけ本気で狙う ---
    let lr_w = if count == 0.0 {
        w.longest_road_when_boxed
    } else {
        w.longest_road
    };
    score += lr_w * ps.longest_road as f32;

    // --- 接しているヘクスの数（盗賊 1 個で全部止まらないこと）---
    let mut tiles = [false; catan_core::topology::NUM_TILES];
    for n in 0..NUM_NODES {
        if matches!(board.building[n], Some(b) if b.owner == me) {
            for t in &topo.node_tiles[n] {
                tiles[t as usize] = true;
            }
        }
    }
    score += w.num_tiles * tiles.iter().filter(|&&x| x).count() as f32;

    score
}

/// 行動の偶然の結果と、その確率。決定的な行動は 1 通りだけ返す。
///
/// これは 1 手読みの期待値計算にも、深さ 2 の expectimax にもそのまま使える。
pub fn chance_outcomes(g: &Game, a: Action) -> Vec<(Forced, f32)> {
    match a {
        Action::Roll => (2u8..=12)
            .map(|sum| {
                let d1 = sum / 2;
                let d2 = sum - d1;
                let ways = catan_core::board::pips(sum).max(if sum == 7 { 6 } else { 0 });
                (Forced::Dice(d1, d2), ways as f32 / 36.0)
            })
            .collect(),

        Action::BuyDevCard => {
            let total: u8 = g.dev_deck.iter().sum();
            if total == 0 {
                return vec![(Forced::No, 1.0)];
            }
            let mut out = Vec::new();
            for c in catan_core::action::DEV_CARDS {
                let n = g.dev_deck[c.idx()];
                if n > 0 {
                    out.push((Forced::Draw(c), n as f32 / total as f32));
                }
            }
            out
        }

        Action::MoveRobber {
            victim: Some(v), ..
        } => {
            let hand = g.players[v as usize].hand;
            let total: u8 = hand.iter().sum();
            if total == 0 {
                return vec![(Forced::Steal(None), 1.0)];
            }
            let mut out = Vec::new();
            for r in catan_core::board::RESOURCES {
                let n = hand[r.idx()];
                if n > 0 {
                    out.push((Forced::Steal(Some(r)), n as f32 / total as f32));
                }
            }
            out
        }

        _ => vec![(Forced::No, 1.0)],
    }
}

/// その手を指した後の評価値の**期待値**。
///
/// サイコロや引いたカードを 1 回だけサンプルすると、
/// 「たまたま勝利点カードを引いた」だけで購入が過大評価される。
///
/// 交易は特別扱いする。`OfferTrade` を指しても資源は動かないので、
/// 素直に 1 手読むと評価値が変わらず、ボットは永久に交易しない。
/// 「成立したらどうなるか」を先に作って比べる（[`crate::trade::preview`]）。
pub fn eval_after(g: &Game, a: Action, me: PlayerId, w: &EvalWeights) -> f32 {
    if crate::trade::is_negotiation(&a) {
        let th = threats(g, w);
        return eval_after_with_base(g, a, me, w, evaluate(g, me, w), &th);
    }
    let outcomes = chance_outcomes(g, a);
    if outcomes.len() == 1 {
        let mut sim = g.clone();
        sim.apply_forced(a, outcomes[0].0);
        return evaluate(&sim, me, w);
    }
    let mut acc = 0.0;
    for (f, p) in outcomes {
        let mut sim = g.clone();
        sim.apply_forced(a, f);
        acc += p * evaluate(&sim, me, w);
    }
    acc
}

/// `base = evaluate(g, me, w)` を外で 1 度だけ計算しておいて渡す版。
///
/// 交易は手札しか動かさないので、盤面由来の項を計算し直す必要がない。
/// 1 手番に数十通り並ぶ提案を捌くための入口。
pub fn eval_after_with_base(
    g: &Game,
    a: Action,
    me: PlayerId,
    w: &EvalWeights,
    base: f32,
    th: &[f32; MAX_PLAYERS],
) -> f32 {
    if crate::trade::is_negotiation(&a) {
        return match crate::trade::eval_delta(g, me, a, w, th) {
            Some(d) => base + d,
            // 成立しない提案は、手番の持ち時間を捨てるだけ
            None => base - crate::trade::FAILED_OFFER_PENALTY,
        };
    }
    eval_after(g, a, me, w)
}

/// 参考: 盤上で自分が持つ最長路の長さ（デバッグ表示用）
pub fn my_roads(board: &Board, p: PlayerId) -> Vec<EdgeId> {
    (0..NUM_EDGES)
        .filter(|&e| board.road[e] == Some(p))
        .map(|e| e as EdgeId)
        .collect()
}

/// 参考: 勝利までの残り点数
pub fn vp_to_go(g: &Game, p: PlayerId) -> u8 {
    VP_TO_WIN.saturating_sub(g.actual_vp(p))
}

const _: () = assert!(MAX_PLAYERS == 4);
const _: () = assert!(NUM_DEV_KINDS == 5);
