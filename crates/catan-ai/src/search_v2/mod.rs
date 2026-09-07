//! M4（研究段階）: 自分の手番の先、**相手の手番を自分の次の手番までロールアウト**する葉評価。
//!
//! 設計書 8.2 の 2 つの失敗を、作りで避ける:
//! - **strategy fusion**: 根の候補は推定サンプル全体で 1 つ（サンプルごとに別の手を選ばない）。
//!   ロールアウトは自分の次の手番が来た所で止めるので、「将来の自分」がサンプルの秘密を見て
//!   手を変える場面を作らない。
//! - **相手の全知化**: ロールアウト内の相手方策 [`OpponentPolicy`] は、**その相手自身の手札と
//!   公開情報だけ**を読む。根の手番の手札・他人の手札・山の内訳・発展カードの種類は読まない
//!   （テスト `small_games::本番のロールアウト相手方策は根の秘密を読まない`）。
//!
//! 近似（明記）: 相手方策は手続き的な簡易方策（建てられる物を建て、道を伸ばし、盗賊は
//! 相手の産出を止める）。相手どうしの交易・独占は行わない。出目は推論用 RNG で 1 本サンプルする
//! （期待値ではない）ので葉の値には分散があり、静的評価と混ぜて使う。
//!
//! これは **root sampling 型の近似**であり、情報集合を木の単位にした探索（ISMCTS）ではない。
//! 木の内側での再サンプルはしていない。

use catan_core::action::{Action, Bundle, Prompt};
use catan_core::board::{BuildingKind, PlayerId, NUM_RESOURCES};
use catan_core::game::{Game, COST_CITY, COST_SETTLEMENT, MAX_PLAYERS};
use catan_core::rng::Rng;
use catan_core::topology::{EdgeId, NodeId, Topology, NUM_NODES, NUM_TILES};

use crate::eval::{evaluate, EvalWeights};

/// ロールアウト内で相手を動かす簡易方策。**相手自身の情報集合しか読まない**。
///
/// 読むもの: 自分（= その相手）の資源・発展、盤面、公開の枚数、盗賊、点数。
/// 読まないもの: 他人の手札の中身、他人の発展の種類、山の内訳、根の手番の秘密。
pub struct OpponentPolicy {
    _seed: u64,
}

fn affords(hand: &Bundle, cost: &Bundle) -> bool {
    (0..NUM_RESOURCES).all(|i| hand[i] >= cost[i])
}

fn node_value(g: &Game, n: NodeId) -> u16 {
    g.board.node_pips(n).iter().map(|&v| v as u16).sum()
}

fn open_spot(g: &Game, n: usize) -> bool {
    let topo = Topology::get();
    g.board.building[n].is_none() && topo.node_neighbors[n].as_slice().iter().all(|&m| g.board.building[m as usize].is_none())
}

impl OpponentPolicy {
    pub fn new(seed: u64) -> Self {
        OpponentPolicy { _seed: seed }
    }

    /// 相手 `q` の判断。`legal` は q に提示された合法手。
    pub fn choose(&mut self, g: &Game, q: PlayerId, legal: &[Action], rng: &mut Rng) -> Action {
        let topo = Topology::get();
        let ps = &g.players[q as usize];
        let hand = ps.hand;
        let pick = |pred: &dyn Fn(&Action) -> bool| legal.iter().copied().find(|a| pred(a));
        match g.prompt {
            Prompt::PlayTurn => {
                if !g.rolled {
                    // 盗賊に塞がれていれば騎士を先に（自分の建物と盗賊の位置は公開）
                    let blocked = topo.tile_nodes[g.board.robber as usize]
                        .iter()
                        .any(|&n| matches!(g.board.building[n as usize], Some(b) if b.owner == q));
                    if blocked {
                        if let Some(a) = pick(&|a| matches!(a, Action::PlayKnight)) {
                            return a;
                        }
                    }
                    return Action::Roll;
                }
                // 都市 → 開拓地（産出の高い頂点）→ 発展 → 道（空き頂点へ向かう）
                if let Some(a) = legal.iter().copied().filter(|a| matches!(a, Action::BuildCity(_))).max_by_key(|a| match a {
                    Action::BuildCity(n) => node_value(g, *n),
                    _ => 0,
                }) {
                    return a;
                }
                if let Some(a) = legal.iter().copied().filter(|a| matches!(a, Action::BuildSettlement(_))).max_by_key(|a| match a {
                    Action::BuildSettlement(n) => node_value(g, *n),
                    _ => 0,
                }) {
                    return a;
                }
                // 収穫: 都市か開拓地があと 2 枚以内で建つなら使う
                if let Some(a) = legal.iter().copied().find(|a| match a {
                    Action::PlayYearOfPlenty(r1, r2) => {
                        let mut h = hand;
                        h[r1.idx()] += 1;
                        h[r2.idx()] += 1;
                        (affords(&h, &COST_CITY) && !affords(&hand, &COST_CITY)) || (affords(&h, &COST_SETTLEMENT) && !affords(&hand, &COST_SETTLEMENT))
                    }
                    _ => false,
                }) {
                    return a;
                }
                // 銀行交換: 都市・開拓地が建つようになる交換だけ
                if let Some(a) = legal.iter().copied().find(|a| match a {
                    Action::MaritimeTrade { give, count, take } => {
                        let mut h = hand;
                        h[give.idx()] -= count;
                        h[take.idx()] += 1;
                        (affords(&h, &COST_CITY) && !affords(&hand, &COST_CITY)) || (affords(&h, &COST_SETTLEMENT) && !affords(&hand, &COST_SETTLEMENT))
                    }
                    _ => false,
                }) {
                    return a;
                }
                if let Some(a) = pick(&|a| matches!(a, Action::BuyDevCard)) {
                    return a;
                }
                // 騎士で最大騎士力が取れるなら
                if let Some(a) = pick(&|a| matches!(a, Action::PlayKnight)) {
                    let mine = ps.played_knights() + 1;
                    let gain = mine >= 3
                        && match g.largest_army_owner {
                            None => true,
                            Some(o) => o != q && mine > g.players[o as usize].played_knights(),
                        };
                    if gain {
                        return a;
                    }
                }
                if let Some(a) = pick(&|a| matches!(a, Action::PlayRoadBuilding)) {
                    return a;
                }
                // 道: 空き頂点に隣接する辺へ。手札に木・土が余っている時だけ
                if hand[0] >= 1 && hand[1] >= 1 && (hand.iter().sum::<u8>() >= 4) {
                    if let Some(a) = self.best_road(g, q, legal) {
                        return a;
                    }
                }
                Action::EndTurn
            }
            Prompt::FreeRoad => self.best_road(g, q, legal).unwrap_or(legal[0]),
            Prompt::Discard => {
                // 都市・開拓地に要る分を残し、多い物から捨てる
                let mut best = legal[0];
                let mut best_score = f32::NEG_INFINITY;
                for a in legal {
                    if let Action::Discard(b) = a {
                        let mut h = hand;
                        for i in 0..NUM_RESOURCES {
                            h[i] -= b[i];
                        }
                        let mut s = 0.0f32;
                        if affords(&h, &COST_CITY) {
                            s += 3.0;
                        }
                        if affords(&h, &COST_SETTLEMENT) {
                            s += 2.0;
                        }
                        // 種類が散っている方が良い
                        s += (0..NUM_RESOURCES).filter(|&i| h[i] > 0).count() as f32 * 0.3;
                        if s > best_score {
                            best_score = s;
                            best = *a;
                        }
                    }
                }
                best
            }
            Prompt::MoveRobber => {
                // 相手（q から見た他人）の産出を最も止める場所へ。自分の建物のある所は避ける。
                // 奪う相手は手札の枚数が最も多い人（枚数は公開）
                let mut best = legal[0];
                let mut best_score = f32::NEG_INFINITY;
                for a in legal {
                    if let Action::MoveRobber { tile, victim } = a {
                        let t = *tile as usize;
                        if t >= NUM_TILES {
                            continue;
                        }
                        let pips = g.board.tile_pips(*tile) as f32;
                        let mut s = 0.0f32;
                        for &n in &topo.tile_nodes[t] {
                            if let Some(b) = g.board.building[n as usize] {
                                let mult = if b.kind == BuildingKind::City { 2.0 } else { 1.0 };
                                if b.owner == q {
                                    s -= 3.0 * pips * mult;
                                } else {
                                    s += pips * mult * (1.0 + 0.1 * g.public_vp(b.owner) as f32);
                                }
                            }
                        }
                        if let Some(v) = victim {
                            s += 0.5 * g.players[*v as usize].hand_size() as f32;
                        }
                        if s > best_score {
                            best_score = s;
                            best = *a;
                        }
                    }
                }
                best
            }
            // 相手どうしの交渉はロールアウトでは行わない（提案は出さないので、返答は断るだけ）
            Prompt::DecideTrade => Action::RejectTrade,
            Prompt::DecideAcceptees => Action::CancelTrade,
            _ => {
                let _ = rng;
                legal[0]
            }
        }
    }

    fn best_road(&self, g: &Game, q: PlayerId, legal: &[Action]) -> Option<Action> {
        let topo = Topology::get();
        let mut best: Option<(Action, u16)> = None;
        for a in legal {
            if let Action::BuildRoad(e) = a {
                let e = *e as EdgeId;
                let mut s = 0u16;
                for &n in &topo.edge_nodes[e as usize] {
                    if open_spot(g, n as usize) {
                        s += 4 + node_value(g, n);
                    } else if g.board.building[n as usize].is_none() {
                        // 空いてはいるが距離ルールで建てられない: 先へ伸ばす価値
                        s += 1;
                    }
                }
                if best.map_or(true, |(_, bs)| s > bs) {
                    best = Some((*a, s));
                }
            }
        }
        let _ = (q, NUM_NODES);
        best.map(|(a, _)| a)
    }
}

/// 自分の手番が終わった局面 `g` から、自分の次の手番が始まるまでロールアウトして評価する。
///
/// - 自分の手番の残り（廃棄・盗賊・無償の道など）は簡易方策で片付ける
/// - 相手の手番は [`OpponentPolicy`]（相手自身の情報集合だけ）
/// - 終局なら ±terminal、それ以外は自分の次の手番の開始時点の静的評価
pub fn rollout_value(start: &Game, me: PlayerId, w: &EvalWeights, rng: &mut Rng, max_turns: u32) -> f32 {
    let mut g = start.clone();
    // ロールアウトでは提案を生成しない（相手どうしの交易は扱わない）。合法手の生成も軽くなる
    g.cfg.max_generated_offers = 0;
    g.cfg.domestic_trade = false;
    g.rng_dice = Rng::new(rng.next_u32() as u64);
    g.rng_dev = Rng::new(rng.next_u32() as u64);
    g.rng_steal = Rng::new(rng.next_u32() as u64);
    let mut pol = OpponentPolicy::new(rng.next_u32() as u64);
    let start_turn = g.turn;
    let mut buf = Vec::with_capacity(64);
    let mut steps = 0u32;
    loop {
        if g.is_over() {
            break;
        }
        // 自分の次の手番が来たら止める（手番プレイヤーが自分で、手番の開始時）
        if g.turn_player == me && g.turn > start_turn && g.prompt == Prompt::PlayTurn && !g.rolled {
            break;
        }
        if g.turn > start_turn + max_turns as u32 * MAX_PLAYERS as u32 || steps > 400 {
            break;
        }
        g.legal_actions_into(&mut buf);
        if buf.is_empty() {
            break;
        }
        let actor = g.to_act;
        let a = if g.turn_player == me && actor == me {
            // 自分の手番の残り: 手番を終える（探索の葉なので、残りの手は読まない）
            if buf.contains(&Action::EndTurn) {
                Action::EndTurn
            } else {
                pol.choose(&g, me, &buf, rng)
            }
        } else if actor == me {
            // 他人の手番中の自分の判断（廃棄・自分への提案への返答）
            match g.prompt {
                Prompt::DecideTrade => Action::RejectTrade,
                _ => pol.choose(&g, me, &buf, rng),
            }
        } else {
            pol.choose(&g, actor, &buf, rng)
        };
        g.apply(a);
        steps += 1;
    }
    evaluate(&g, me, w)
}
