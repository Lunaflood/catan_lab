//! 交易の評価。
//!
//! ここが既存実装の空白地帯。catanatron は `OFFER_TRADE` を行動空間に生成すらしない
//! （= 交渉なしカタンを遊んでいる）。
//!
//! 1 手読みのままでは交易を評価できない、という構造的な問題がある:
//! `OfferTrade` を指しても資源は動かない（相手の返答を待つだけ）ので、
//! 評価値が 1 ミリも変わらず、ボットは絶対に交易しない。
//! そこで「成立したらどうなるか」を先に作ってから評価する。
//!
//! 「7 点以上のリーダーとは交易しない」という競技の規範は**書き込んでいない**。
//! 評価関数に「相手の手札の充実度 × 脅威度（勝利点で割り増し）」が入っているので、
//! リーダーを利する交換は自然に低く出る。規範は結果であって前提ではない。

use crate::eval::{hand_terms, EvalWeights};
use catan_core::action::{Action, Bundle, Prompt};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::{Game, MAX_PLAYERS};

/// `from` が `give` を出し、`to` が `want` を出す交換を、盤面に直接適用する。
/// 交渉のプロンプトを一巡させる代わりの近道（資源の総数は保存される）。
fn settle(g: &mut Game, from: PlayerId, to: PlayerId, give: &Bundle, want: &Bundle) {
    for i in 0..NUM_RESOURCES {
        debug_assert!(g.players[from as usize].hand[i] >= give[i]);
        debug_assert!(g.players[to as usize].hand[i] >= want[i]);
        g.players[from as usize].hand[i] -= give[i];
        g.players[to as usize].hand[i] += give[i];
        g.players[to as usize].hand[i] -= want[i];
        g.players[from as usize].hand[i] += want[i];
    }
}

/// 交換で 2 人の手札がどう変わるかを、`viewer` の視点で評価した差分。
///
/// `giver` が `give` を出し、`taker` が `want` を出す。
/// `viewer` が自分の得だけでなく「相手をどれだけ利するか」も込みで見るのが要点。
fn swap_delta(
    g: &Game,
    viewer: PlayerId,
    giver: PlayerId,
    taker: PlayerId,
    give: &Bundle,
    want: &Bundle,
    w: &EvalWeights,
    th: &[f32; MAX_PLAYERS],
) -> f32 {
    let mut delta = 0.0;
    for p in [giver, taker] {
        let old = g.players[p as usize].hand;
        let mut new = old;
        for i in 0..NUM_RESOURCES {
            if p == giver {
                new[i] = new[i] - give[i] + want[i];
            } else {
                new[i] = new[i] + give[i] - want[i];
            }
        }
        let threat = th[p as usize];
        delta += hand_terms(&new, p == viewer, threat, w) - hand_terms(&old, p == viewer, threat, w);
    }
    // 交易そのものの代価。わずかな得で無限に交換し続けないための床
    delta - w.trade_cost
}

/// その提案を**実際に受けてくれる**相手のうち、最も脅威が低い者。
///
/// ここが交易 AI の要。「払えるか」だけで相手を選ぶと、
/// 相手が同じくらい賢い場合はことごとく断られ、手番を提案で溶かす。
/// （実測: 相手モデル無しだと 4 体同型で 18% の対局が終わらなくなった）
/// 相手の立場で同じ評価関数を回し、得にならない提案は最初から出さない。
fn best_acceptor(
    g: &Game,
    proposer: PlayerId,
    give: &Bundle,
    want: &Bundle,
    w: &EvalWeights,
    th: &[f32; MAX_PLAYERS],
) -> Option<PlayerId> {
    let mut best: Option<(PlayerId, f32)> = None;
    for q in 0..g.n() {
        let p = q as PlayerId;
        if p == proposer {
            continue;
        }
        let hand = g.players[q].hand;
        if (0..NUM_RESOURCES).any(|i| hand[i] < want[i]) {
            continue; // 払えない
        }
        // 相手の視点で得になるか（相手も同じ物差しで考えると仮定する）
        if swap_delta(g, p, proposer, p, give, want, w, th) <= 0.0 {
            continue;
        }
        // 脅威度は同じ表から読む（勝利点は交易で動かない）
        let threat = th[q];
        if best.map_or(true, |(_, t)| threat < t) {
            best = Some((p, threat));
        }
    }
    best.map(|(p, _)| p)
}

/// 交易系の行動を「誰が誰に何を渡すか」に開く。
///
/// `None` なら成立しない（払える相手が居ない / 提案者が対案を払えない）。
fn resolve(
    g: &Game,
    me: PlayerId,
    a: Action,
    w: &EvalWeights,
    th: &[f32; MAX_PLAYERS],
) -> Option<(PlayerId, PlayerId, Bundle, Bundle)> {
    match a {
        Action::OfferTrade { give, want } => {
            let partner = best_acceptor(g, me, &give, &want, w, th)?;
            Some((me, partner, give, want))
        }
        Action::AcceptTrade => {
            let t = g.trade?;
            // 提案者が give を出し、自分が want を出す
            Some((t.proposer, me, t.give, t.want))
        }
        Action::CounterOffer { give, want } => {
            let t = g.trade?;
            let hand = g.players[t.proposer as usize].hand;
            if (0..NUM_RESOURCES).any(|i| hand[i] < want[i]) {
                return None; // 提案者が払えない対案は通らない
            }
            // 提案者が受けてくれる対案でなければ意味がない
            if swap_delta(g, t.proposer, me, t.proposer, &give, &want, w, th) <= 0.0 {
                return None;
            }
            Some((me, t.proposer, give, want))
        }
        _ => None,
    }
}

/// 交易が成立した局面（テスト用。実戦の評価は [`eval_delta`] を使う）
pub fn preview(g: &Game, me: PlayerId, a: Action, w: &EvalWeights) -> Option<Game> {
    let th = crate::eval::threats(g, w);
    let (from, to, give, want) = resolve(g, me, a, w, &th)?;
    let mut sim = g.clone();
    settle(&mut sim, from, to, &give, &want);
    Some(sim)
}

/// 交易による評価値の変化。
///
/// 交易で動くのは**手札だけ**なので、産出・拡張・勝利点は一切変わらない。
/// 局面を複製して評価し直す必要がなく、関係する 2 人の手札の項だけ差し引きすればよい。
/// 提案は 1 手番に何十通りも並ぶので、ここが速いかどうかが全体の速度を決める。
pub fn eval_delta(
    g: &Game,
    me: PlayerId,
    a: Action,
    w: &EvalWeights,
    th: &[f32; MAX_PLAYERS],
) -> Option<f32> {
    let (from, to, give, want) = resolve(g, me, a, w, th)?;
    Some(swap_delta(g, me, from, to, &give, &want, w, th))
}

/// 交易系の行動か（先読みの前に分岐するため）
#[inline]
pub fn is_negotiation(a: &Action) -> bool {
    matches!(
        a,
        Action::OfferTrade { .. } | Action::AcceptTrade | Action::CounterOffer { .. }
    )
}

/// 提案が流れた時の損。手番の持ち時間と、相手に手札を晒す分。
pub const FAILED_OFFER_PENALTY: f32 = 5.0;

/// いま交渉の返答を求められているか
#[inline]
pub fn is_responding(g: &Game) -> bool {
    g.prompt == Prompt::DecideTrade
}
