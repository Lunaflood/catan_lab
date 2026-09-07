//! 手番内の先読み（expectimax）。
//!
//! カタンの 1 手番は「交易して → 建てて → もう 1 つ建てる」のような**手順**になる。
//! 1 手読みは各手を単独で比べるので、この手順が見えない
//! （例: いま損な海上交易をしてから都市を建てる、という 2 手の組み合わせ）。
//!
//! 読むのは**自分の手番の中だけ**。相手の手番へ踏み込まないのは意図的で:
//! - 4 人ゲームの多人数探索（max^n / paranoid / BRS）はカタンでの比較データが無い
//! - 相手の手を読むより、自分の手順を正しく組む方が取り分が大きい
//!   （catanatron の SameTurnAlphaBeta と同じ判断）
//!
//! 偶然（ダイス・引くカード・盗み）は期待値で畳む。1 回サンプルすると
//! 「たまたま勝利点カードを引いた」だけで購入が過大評価される。

use crate::eval::{chance_outcomes, evaluate, threats, EvalWeights};
use crate::prune::prune;
use catan_core::action::Action;
use catan_core::board::PlayerId;
use catan_core::game::Game;

pub struct SearchLimits {
    /// 自分の手を何手先まで並べるか
    pub depth: u32,
    /// 1 回の決定で展開してよい局面の数。超えたら葉として評価する
    pub max_nodes: u32,
}

impl Default for SearchLimits {
    fn default() -> Self {
        SearchLimits {
            depth: 2,
            max_nodes: 4000,
        }
    }
}

struct Ctx<'a> {
    me: PlayerId,
    w: &'a EvalWeights,
    nodes: u32,
    budget: u32,
}

impl Ctx<'_> {
    /// `g` を `me` の視点で評価する。`depth` は「あと何手 自分が指せるか」。
    fn value(&mut self, g: &Game, depth: u32) -> f32 {
        // 自分の番でなくなったら、そこが地平線
        if depth == 0 || g.is_over() || g.to_act != self.me {
            return evaluate(g, self.me, self.w);
        }
        if self.nodes >= self.budget {
            return evaluate(g, self.me, self.w);
        }

        let actions = g.legal_actions();
        let th = threats(g, self.w);
        let cand = prune(g, self.me, &actions, self.w, &th);

        let mut best = f32::NEG_INFINITY;
        for a in cand {
            let v = self.after(g, a, depth);
            if v > best {
                best = v;
            }
        }
        if best.is_finite() {
            best
        } else {
            evaluate(g, self.me, self.w)
        }
    }

    /// 手 `a` を指した後の値（偶然は期待値で畳む）
    fn after(&mut self, g: &Game, a: Action, depth: u32) -> f32 {
        // 交易は成立後の局面が手札しか動かさないので、差分で足りる。
        // ここを展開すると提案の数だけ分岐が増えて探索が持たない。
        if crate::trade::is_negotiation(&a) {
            let th = threats(g, self.w);
            let base = evaluate(g, self.me, self.w);
            return crate::eval::eval_after_with_base(g, a, self.me, self.w, base, &th);
        }

        let outcomes = chance_outcomes(g, a);
        let mut acc = 0.0;
        for (f, p) in outcomes {
            self.nodes += 1;
            let mut sim = g.clone();
            sim.apply_forced(a, f);
            acc += p * self.value(&sim, depth - 1);
        }
        acc
    }
}

/// 相手の伏せた発展カードの引き直しを **複数回** 行い、値を平均して選ぶ。
///
/// `determinize` は「あり得る世界」を 1 つ引くだけなので、引いた世界がたまたま
/// 極端だとその手番の判断がまるごと引きずられる。世界を何本か引いて平均すれば、
/// 見えていない部分に対する当てずっぽうの分散が減る。
/// 枝刈りは最初の世界で 1 回だけ行い、**同じ候補集合**を全部の世界で比べる
/// （世界ごとに候補が違うと平均が別物どうしの平均になる）。
pub fn best_action_worlds(
    v: &catan_core::view::View,
    me: PlayerId,
    actions: &[Action],
    w: &EvalWeights,
    lim: &SearchLimits,
    rng: &mut catan_core::rng::Rng,
    worlds: u32,
) -> Action {
    let g0 = v.determinize(rng);
    let th = threats(&g0, w);
    let cand = prune(&g0, me, actions, w, &th);
    if cand.len() == 1 {
        return cand[0];
    }
    let mut acc = vec![0.0f32; cand.len()];
    for k in 0..worlds.max(1) {
        let g = if k == 0 { g0.clone() } else { v.determinize(rng) };
        let mut ctx = Ctx { me, w, nodes: 0, budget: lim.max_nodes };
        for (i, a) in cand.iter().enumerate() {
            acc[i] += ctx.after(&g, *a, lim.depth);
        }
    }
    let mut best = cand[0];
    let mut best_v = f32::NEG_INFINITY;
    for (i, a) in cand.iter().enumerate() {
        if acc[i] > best_v {
            best_v = acc[i];
            best = *a;
        }
    }
    best
}

/// `me` の手番で、候補手のうち最善のものを選ぶ。
pub fn best_action(g: &Game, me: PlayerId, actions: &[Action], w: &EvalWeights, lim: &SearchLimits) -> Action {
    let th = threats(g, w);
    let cand = prune(g, me, actions, w, &th);
    if cand.len() == 1 {
        return cand[0];
    }
    let mut ctx = Ctx {
        me,
        w,
        nodes: 0,
        budget: lim.max_nodes,
    };
    let mut best = cand[0];
    let mut best_v = f32::NEG_INFINITY;
    for a in cand {
        let v = ctx.after(g, a, lim.depth);
        if v > best_v {
            best_v = v;
            best = a;
        }
    }
    best
}
