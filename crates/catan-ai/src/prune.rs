//! 候補手の枝刈り。
//!
//! 1 手読みでも 1 局面あたり 100 手前後が並ぶ。全部を完全に評価すると、
//! 4 体とも思考ボットにした時点で計算が破綻する（実測で 3 ゲーム毎秒まで落ちた）。
//! 深さ 2 に進むには、ここを削っておく必要がある。
//!
//! 削り方は「安い物差しで並べて上位だけ残す」。catanatron も同じことをしていて、
//! 盗賊に至っては最有力の 1 手しか残していない。こちらは複数残して探索に判断させる。

use crate::eval::{hand_potential, EvalWeights};
use catan_core::action::{Action, Bundle};
use catan_core::board::{BuildingKind, PlayerId};
use catan_core::game::{Game, MAX_PLAYERS};
use catan_core::topology::{TileId, Topology};

/// 盗賊の置き先として残す数
pub const KEEP_ROBBER: usize = 3;
/// 海上交易として残す数
pub const KEEP_MARITIME: usize = 4;
/// 交易の提案・対案として残す数
pub const KEEP_OFFERS: usize = 6;

/// 残す数の倍率。**測るためのつまみ**。
///
/// 枝刈りは「安い物差し」で並べて上位だけ残す。物差しが最終判定と違う以上、
/// ここで落とした手の中に良い手が混じっている可能性がある。
/// 倍率を上げて強さが変わるなら、それは削りすぎていた証拠になる。
/// 既定は 1（従来どおり）。
use std::cell::Cell;
thread_local! {
    static KEEP_MUL: Cell<f32> = const { Cell::new(1.0) };
}
pub fn set_keep_mul(m: f32) {
    KEEP_MUL.with(|c| c.set(m));
}
fn keep(n: usize) -> usize {
    let m = KEEP_MUL.with(|c| c.get());
    ((n as f32 * m).round() as usize).max(1)
}

/// そのヘクスを止めた時に減る産出（pip）。都市は 2 倍。
fn tile_yield(g: &Game, t: TileId, p: PlayerId) -> f32 {
    let topo = Topology::get();
    if g.board.tile_resource[t as usize].is_none() {
        return 0.0;
    }
    let pips = g.board.tile_pips(t) as f32;
    let mut n = 0.0;
    for node in topo.tile_nodes[t as usize] {
        if let Some(b) = g.board.building[node as usize] {
            if b.owner == p {
                n += match b.kind {
                    BuildingKind::Settlement => 1.0,
                    BuildingKind::City => 2.0,
                };
            }
        }
    }
    pips * n
}

/// 盗賊を `tile` に置いて `victim` から盗む手の「安い」点数。
///
/// 「相手の産出をどれだけ削るか − 自分の産出をどれだけ削るか」。
/// 相手側は勝利点で重み付けするので、リーダーのヘクスが自然に上に来る。
/// 競技の 4 原則（リーダーを止める・手札の多い相手から盗む・自分のヘクスに置かない）は
/// この式から出てくるので、規則としては書かない。
fn robber_score(
    g: &Game,
    me: PlayerId,
    tile: TileId,
    victim: Option<PlayerId>,
    th: &[f32; MAX_PLAYERS],
) -> f32 {
    let mut s = 0.0;
    for q in 0..g.n() {
        let p = q as PlayerId;
        let y = tile_yield(g, tile, p);
        if p == me {
            s -= y * 2.0; // 自分のヘクスを止めるのは二重に損
        } else {
            s += y * th[q];
        }
    }
    if let Some(v) = victim {
        // 手札が多い相手から盗む方が期待値が高い（枚数は公開情報）
        s += (g.players[v as usize].hand_size() as f32).min(8.0) * 0.5 * th[v as usize];
    }
    s
}

fn take_top<T, F: Fn(&T) -> f32>(mut xs: Vec<T>, n: usize, key: F) -> Vec<T> {
    if xs.len() <= n {
        return xs;
    }
    xs.sort_by(|a, b| key(b).partial_cmp(&key(a)).unwrap_or(std::cmp::Ordering::Equal));
    xs.truncate(n);
    xs
}

/// 4:1 / 3:1 / 2:1 の交換で、何かが**払えるようになる**か。
fn maritime_gain(g: &Game, me: PlayerId, a: &Action, w: &EvalWeights) -> f32 {
    let Action::MaritimeTrade { give, count, take } = a else {
        return 0.0;
    };
    let mut hand: Bundle = g.players[me as usize].hand;
    hand[give.idx()] -= count;
    hand[take.idx()] += 1;
    hand_potential(&hand, w) - hand_potential(&g.players[me as usize].hand, w)
}

/// 候補手を、安い物差しで並べて削る。
///
/// 完全に評価する手の数を 1/4 前後まで落とす。削るのは
/// 「同種で大量に並び、かつ安く順位を付けられる」ものだけ。
/// 建設（道・開拓地・都市）は削らない ── そこが本番なので。
pub fn prune(
    g: &Game,
    me: PlayerId,
    actions: &[Action],
    w: &EvalWeights,
    th: &[f32; MAX_PLAYERS],
) -> Vec<Action> {
    let mut robber = Vec::new();
    let mut maritime = Vec::new();
    let mut offers = Vec::new();
    let mut rest = Vec::new();

    for a in actions {
        match a {
            Action::MoveRobber { .. } => robber.push(*a),
            Action::MaritimeTrade { .. } => maritime.push(*a),
            Action::OfferTrade { .. } | Action::CounterOffer { .. } => offers.push(*a),
            _ => rest.push(*a),
        }
    }

    let robber = take_top(robber, keep(KEEP_ROBBER), |a| {
        let Action::MoveRobber { tile, victim } = a else { return 0.0 };
        robber_score(g, me, *tile, *victim, th)
    });

    let maritime = take_top(maritime, keep(KEEP_MARITIME), |a| maritime_gain(g, me, a, w));

    // 提案は差分評価がもともと安いので、それをそのまま順位付けに使う
    let offers = take_top(offers, keep(KEEP_OFFERS), |a| {
        crate::trade::eval_delta(g, me, *a, w, th).unwrap_or(f32::NEG_INFINITY)
    });

    rest.extend(robber);
    rest.extend(maritime);
    rest.extend(offers);
    rest
}

#[cfg(test)]
mod tests {
    use super::*;
    use catan_core::game::GameConfig;
    use catan_core::rng::Rng;

    #[test]
    fn 枝刈りしても手は必ず残る() {
        let mut g = Game::with_config(4, 7, GameConfig::default());
        let mut rng = Rng::with_stream(7, 3);
        let w = EvalWeights::default();
        let mut buf = Vec::new();
        for _ in 0..3000 {
            if g.is_over() {
                break;
            }
            g.legal_actions_into(&mut buf);
            let th = crate::eval::threats(&g, &w);
            let kept = prune(&g, g.to_act, &buf, &w, &th);
            assert!(!kept.is_empty(), "枝刈りで手が全部消えた {:?}", g.prompt);
            for a in &kept {
                assert!(buf.contains(a), "枝刈りが合法でない手を作った: {a:?}");
            }
            g.apply(buf[rng.below(buf.len() as u32) as usize]);
        }
    }

    #[test]
    fn 自分のヘクスには盗賊を置かない() {
        // 自分だけが接しているヘクスと、相手だけが接しているヘクスを比べる
        let mut g = Game::with_config(4, 11, GameConfig::default());
        let mut rng = Rng::with_stream(11, 5);
        let w = EvalWeights::default();
        let mut buf = Vec::new();
        while g.is_setup() {
            g.legal_actions_into(&mut buf);
            g.apply(buf[rng.below(buf.len() as u32) as usize]);
        }
        let th = crate::eval::threats(&g, &w);
        let topo = Topology::get();
        for t in 0..catan_core::topology::NUM_TILES {
            let t = t as TileId;
            let mine = tile_yield(&g, t, 0);
            let others: f32 = (1..4).map(|q| tile_yield(&g, t, q)).sum();
            if mine > 0.0 && others == 0.0 {
                let s = robber_score(&g, 0, t, None, &th);
                assert!(s < 0.0, "自分だけのヘクス {t} の点数が正 ({s})");
            }
        }
        let _ = topo;
    }
}
