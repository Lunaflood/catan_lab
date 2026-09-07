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

use crate::eval::{chance_outcomes, evaluate, threats, threats_bonus, EvalWeights};
use crate::prune::prune;
use catan_core::action::Action;
use catan_core::board::PlayerId;
use catan_core::game::{Game, MAX_PLAYERS};

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
    /// 脅威度への上乗せ（v2 の勝利ハザード）。旧探索では 0
    bonus: [f32; MAX_PLAYERS],
    /// 自分の手番が終わった葉の評価（v2 のロールアウト）。無ければ静的評価
    leaf: Option<&'a dyn Fn(&Game) -> f32>,
}

impl Ctx<'_> {
    #[inline]
    fn th(&self, g: &Game) -> [f32; MAX_PLAYERS] {
        if self.bonus == [0.0; MAX_PLAYERS] {
            threats(g, self.w)
        } else {
            threats_bonus(g, self.w, &self.bonus)
        }
    }

    /// `g` を `me` の視点で評価する。`depth` は「あと何手 自分が指せるか」。
    fn value(&mut self, g: &Game, depth: u32) -> f32 {
        // 自分の番でなくなったら、そこが地平線
        if g.is_over() {
            return evaluate(g, self.me, self.w);
        }
        if g.to_act != self.me {
            return match self.leaf {
                Some(f) => f(g),
                None => evaluate(g, self.me, self.w),
            };
        }
        if depth == 0 {
            return evaluate(g, self.me, self.w);
        }
        if self.nodes >= self.budget {
            return evaluate(g, self.me, self.w);
        }

        let actions = g.legal_actions();
        let th = self.th(g);
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
            let th = self.th(g);
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
/// 各推定で残った候補の和集合を、すべての推定で比較する。
/// 根の候補ごとに同じ探索予算を割り当て、列挙順による打ち切りの偏りを防ぐ。
pub fn best_action_worlds(
    v: &catan_core::view::View,
    me: PlayerId,
    actions: &[Action],
    w: &EvalWeights,
    lim: &SearchLimits,
    rng: &mut catan_core::rng::Rng,
    worlds: u32,
) -> Action {
    let samples: Vec<Game> = (0..worlds.max(1)).map(|_| v.determinize(rng)).collect();
    // 最初の推定だけで良い手を捨てない。各推定の候補の和集合を比較する。
    let mut cand = Vec::new();
    for g in &samples {
        let th = threats(g, w);
        for a in prune(g, me, actions, w, &th) {
            if !cand.contains(&a) { cand.push(a); }
        }
    }
    if cand.len() == 1 { return cand[0]; }
    let mut acc = vec![0.0f32; cand.len()];
    // 候補の順序によって後半だけ読みが浅くならないよう、予算を分ける。
    let budget = (lim.max_nodes / cand.len().max(1) as u32).max(1);
    for g in &samples {
        for (i, a) in cand.iter().enumerate() {
            let mut ctx = Ctx { me, w, nodes: 0, budget, bonus: [0.0; MAX_PLAYERS], leaf: None };
            acc[i] += ctx.after(g, *a, lim.depth);
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
        bonus: [0.0; MAX_PLAYERS],
        leaf: None,
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


/// v2 探索の差し込み口。
pub struct SearchHooks<'a> {
    /// 脅威度への上乗せ（勝利ハザード × 係数）。木の中の枝刈り・交易評価に効く
    pub threat_bonus: [f32; MAX_PLAYERS],
    /// 根の候補ごとの補正（推定サンプル番号, その局面, 手）→ 評価値への加算。
    /// 相手の勝利ハザードを動かす手（交易・盗み・独占）に使う
    pub root_adjust: Option<&'a dyn Fn(usize, &Game, Action) -> f32>,
    /// 自分の手番が終わった葉の評価（相手の手番のロールアウト等）。無ければ静的評価
    pub leaf: Option<&'a dyn Fn(&Game) -> f32>,
}

/// 候補ごとの評価（説明・記録用）
#[derive(Clone, Debug)]
pub struct RootValue {
    pub action: Action,
    /// 推定サンプルにわたる平均値
    pub value: f32,
    /// うち根の補正（ハザード）の平均
    pub adjust: f32,
}

/// 推定サンプル（完全情報の局面）の列にわたって、根の候補を比較する。
///
/// [`best_action_worlds`] と同じ骨格（候補の和集合・候補ごとの等しい予算）に、
/// 脅威度の上乗せと根の補正を足したもの。旧ボットの挙動は変えない。
pub fn best_action_samples(
    samples: &[Game],
    me: PlayerId,
    actions: &[Action],
    w: &EvalWeights,
    lim: &SearchLimits,
    hooks: &SearchHooks,
) -> (Action, Vec<RootValue>) {
    let mut cand = Vec::new();
    for g in samples {
        let th = threats_bonus(g, w, &hooks.threat_bonus);
        for a in prune(g, me, actions, w, &th) {
            if !cand.contains(&a) {
                cand.push(a);
            }
        }
    }
    if cand.is_empty() {
        cand.push(actions[0]);
    }
    let k = samples.len().max(1) as f32;
    if cand.len() == 1 {
        return (cand[0], vec![RootValue { action: cand[0], value: 0.0, adjust: 0.0 }]);
    }
    let mut acc = vec![0.0f32; cand.len()];
    let mut adj = vec![0.0f32; cand.len()];
    let budget = (lim.max_nodes / cand.len().max(1) as u32).max(1);
    for (si, g) in samples.iter().enumerate() {
        for (i, a) in cand.iter().enumerate() {
            let mut ctx = Ctx { me, w, nodes: 0, budget, bonus: hooks.threat_bonus, leaf: hooks.leaf };
            acc[i] += ctx.after(g, *a, lim.depth);
            if let Some(f) = hooks.root_adjust {
                let d = f(si, g, *a);
                acc[i] += d;
                adj[i] += d;
            }
        }
    }
    let mut best = 0usize;
    for i in 1..cand.len() {
        if acc[i] > acc[best] {
            best = i;
        }
    }
    let values = cand
        .iter()
        .enumerate()
        .map(|(i, a)| RootValue { action: *a, value: acc[i] / k, adjust: adj[i] / k })
        .collect();
    (cand[best], values)
}
