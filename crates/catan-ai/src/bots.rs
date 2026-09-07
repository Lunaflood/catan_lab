//! ベースラインのボット群。
//!
//! ここにあるのは全部「弱い基準線」。強い探索ボットはこの上に載せる。
//! catanatron のリーダーボードでいう Random / WeightedRandom / VictoryPoint に相当し、
//! 新しいボットが「本当に強くなったのか」を測るための物差しとして必要。

use crate::eval::{eval_after_with_base, evaluate, threats, EvalWeights};
use crate::prune::prune;
use crate::search::{best_action, best_action_worlds, SearchLimits};
use crate::placement::{best_setup_road, best_setup_settlement, PlacementWeights};
use catan_core::action::{Action, Prompt};
use catan_core::rng::Rng;
use catan_core::topology::{EdgeId, NodeId};
use catan_core::view::View;

pub trait Bot {
    fn name(&self) -> String;
    /// `actions` は空でないことが保証される。
    ///
    /// 渡されるのは `Game` ではなく [`View`]。相手の発展カードの種類は見えない。
    /// 先読みしたいボットは [`View::determinize`] で完全情報の局面を作る。
    fn decide(&mut self, view: &View, actions: &[Action]) -> Action;
    /// 各対局の開始時。決定的にするため seed を配る。
    fn reset(&mut self, _seed: u64) {}
}

// =========================================================================

/// 合法手から一様ランダム。すべての比較の原点。
pub struct RandomBot {
    rng: Rng,
}

impl RandomBot {
    pub fn new(seed: u64) -> Self {
        RandomBot { rng: Rng::new(seed) }
    }
}

impl Bot for RandomBot {
    fn name(&self) -> String {
        "Random".into()
    }
    fn reset(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }
    fn decide(&mut self, _v: &View, actions: &[Action]) -> Action {
        actions[self.rng.below(actions.len() as u32) as usize]
    }
}

// =========================================================================

/// 行動の種類に重みを付けたランダム。
///
/// ⚠ Szita et al. 2010 の教訓: この手の重み付けは**素のランダムより弱くなることがある**。
/// 「建設を急ぎすぎて道による布石を軽視する」ため。重みは必ず実測で決めること。
pub struct WeightedRandomBot {
    rng: Rng,
}

impl WeightedRandomBot {
    pub fn new(seed: u64) -> Self {
        WeightedRandomBot { rng: Rng::new(seed) }
    }

    fn weight(a: &Action) -> u32 {
        match a {
            Action::BuildCity(_) => 10_000,
            Action::BuildSettlement(_) | Action::SetupSettlement(_) => 10_000,
            Action::BuyDevCard => 100,
            Action::PlayKnight => 100,
            Action::BuildRoad(_) | Action::SetupRoad(_) => 10,
            Action::PlayMonopoly(_) | Action::PlayYearOfPlenty(..) | Action::PlayRoadBuilding => 10,
            Action::EndTurn => 5,
            // 交易は「とりあえず提案する」を抑える。乱発は手番を空回りさせるだけ
            Action::OfferTrade { .. } => 2,
            _ => 1,
        }
    }
}

impl Bot for WeightedRandomBot {
    fn name(&self) -> String {
        "WeightedRandom".into()
    }
    fn reset(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }
    fn decide(&mut self, _v: &View, actions: &[Action]) -> Action {
        let total: u32 = actions.iter().map(Self::weight).sum();
        let mut k = self.rng.below(total);
        for a in actions {
            let w = Self::weight(a);
            if k < w {
                return *a;
            }
            k -= w;
        }
        actions[actions.len() - 1]
    }
}

// =========================================================================

/// 1 手先の勝利点だけを見る貪欲。catanatron の VictoryPointPlayer 相当。
///
/// これが「1 手読みで現在 VP をほぼそのまま評価値にする」典型例で、
/// HexMachina 論文が「盗賊の使い方も配置も壊滅する」と指摘した形そのもの。
/// **評価関数の悪い例としての基準線**であって、目標ではない。
pub struct VpGreedyBot {
    rng: Rng,
}

impl VpGreedyBot {
    pub fn new(seed: u64) -> Self {
        VpGreedyBot { rng: Rng::new(seed) }
    }
}

impl Bot for VpGreedyBot {
    fn name(&self) -> String {
        "VpGreedy".into()
    }
    fn reset(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }
    fn decide(&mut self, v: &View, actions: &[Action]) -> Action {
        let me = v.me;
        // 相手の発展カードは見えないので、あり得る局面を 1 つ作ってその上で読む
        let base = v.determinize(&mut self.rng);
        let mut best = actions[0];
        let mut best_score = i32::MIN;
        // 同点はランダムに散らす（決定的な偏りで席バイアスを作らないため）
        let salt = self.rng.next_u32();
        for (i, a) in actions.iter().enumerate() {
            let mut sim = base.clone();
            sim.apply(*a);
            let mut score = sim.actual_vp(me) as i32 * 1000;
            // 完全に同点だと EndTurn ばかり選ぶので、微差の指標を足す
            score += sim.players[me as usize].hand_size() as i32;
            let tie = ((salt.wrapping_add(i as u32)).wrapping_mul(2654435761) >> 16) as i32 & 0xFF;
            let score = score * 256 + tie;
            if score > best_score {
                best_score = score;
                best = *a;
            }
        }
        best
    }
}

/// 交易の諾否だけを扱う小さなヘルパー。
/// 「7 点以上のプレイヤーとは交易しない」という競技の基本規範を実装したもの。
pub fn should_accept_trade(v: &View) -> bool {
    debug_assert_eq!(v.prompt(), Prompt::DecideTrade);
    let t = v.trade().expect("交易中でない");
    // リーダーを勝たせる交易はしない（colonist の bot が批判されている点そのもの）
    if v.public_vp(t.proposer) >= 7 {
        return false;
    }
    true
}

// =========================================================================

/// 初期配置だけを評価関数で決め、それ以外は [`WeightedRandomBot`] のまま。
///
/// Guhe & Lascarides 2014 と同じ切り分け方: 配置以外を基準線と同一にしておけば、
/// 勝率の差はまるごと配置の効果になる。彼らはこれで 15.2% → 28.3% を測った。
pub struct PlacementBot {
    inner: WeightedRandomBot,
    w: PlacementWeights,
    label: String,
}

impl PlacementBot {
    pub fn new(seed: u64) -> Self {
        Self::with_weights(seed, PlacementWeights::default(), "Placement")
    }
    pub fn with_weights(seed: u64, w: PlacementWeights, label: &str) -> Self {
        PlacementBot {
            inner: WeightedRandomBot::new(seed),
            w,
            label: label.to_string(),
        }
    }
}

impl Bot for PlacementBot {
    fn name(&self) -> String {
        self.label.clone()
    }
    fn reset(&mut self, seed: u64) {
        self.inner.reset(seed);
    }
    fn decide(&mut self, v: &View, actions: &[Action]) -> Action {
        match v.prompt() {
            Prompt::SetupSettlement => {
                let nodes: Vec<NodeId> = actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupSettlement(n) => Some(*n),
                        _ => None,
                    })
                    .collect();
                Action::SetupSettlement(best_setup_settlement(v, &nodes, &self.w))
            }
            Prompt::SetupRoad => {
                let edges: Vec<EdgeId> = actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupRoad(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
                Action::SetupRoad(best_setup_road(v, &edges, &self.w))
            }
            _ => self.inner.decide(v, actions),
        }
    }
}

// =========================================================================

/// 評価関数で 1 手先を並べるボット。
///
/// 「ビルドプランの固定優先順位」を持たない。建てる・買う・盗む・交易するを
/// **すべて同じ物差しで比べる**。偶然が絡む手（ダイス・発展カードの購入・盗み）は
/// 1 回サンプルするのではなく期待値で評価する。
pub struct GreedyEvalBot {
    rng: Rng,
    w: EvalWeights,
    pw: PlacementWeights,
    label: String,
    /// 複数種類の束の提案を **考えない**。
    /// 「1 種類 ↔ 1 種類しか出せない」制限が強さにどう効くかを測るための刻み。
    /// ルールを変えるのではなく、このボットが見る手を狭めるだけなので、
    /// 同じ盤・同じ規則の下で純粋な A/B になる。
    narrow_offers: bool,
}

impl GreedyEvalBot {
    pub fn new(seed: u64) -> Self {
        Self::with_weights(seed, EvalWeights::default(), "GreedyEval")
    }
    /// 拡張余地を軽く見ていた頃の重み（比較用）
    pub fn v1(seed: u64) -> Self {
        Self::with_weights(seed, EvalWeights::v1(), "GreedyEval-v1")
    }
    pub fn with_weights(seed: u64, w: EvalWeights, label: &str) -> Self {
        GreedyEvalBot {
            rng: Rng::new(seed),
            w,
            pw: PlacementWeights::default(),
            label: label.to_string(),
            narrow_offers: false,
        }
    }

    /// 単一資源どうしの提案しか考えない版（比較用）
    pub fn narrow(seed: u64) -> Self {
        let mut b = Self::with_weights(seed, EvalWeights::default(), "GreedyEval-narrow");
        b.narrow_offers = true;
        b
    }
    /// 初期配置の重みだけ差し替える（配置の項目を強い下流の下で測り直すため）
    pub fn with_placement(seed: u64, pw: PlacementWeights, label: &str) -> Self {
        GreedyEvalBot {
            rng: Rng::new(seed),
            w: EvalWeights::default(),
            pw,
            label: label.to_string(),
            narrow_offers: false,
        }
    }
}

impl Bot for GreedyEvalBot {
    fn name(&self) -> String {
        self.label.clone()
    }
    fn reset(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }
    fn decide(&mut self, v: &View, actions: &[Action]) -> Action {
        // 狭い版は複数種類の束を手から外して考える
        let filtered: Vec<Action>;
        let actions = if self.narrow_offers {
            let single = |b: &catan_core::action::Bundle| b.iter().filter(|&&n| n > 0).count() <= 1;
            filtered = actions
                .iter()
                .filter(|a| match a {
                    Action::OfferTrade { give, want } | Action::CounterOffer { give, want } => {
                        single(give) && single(want)
                    }
                    _ => true,
                })
                .copied()
                .collect();
            // 全部消えることは無い（手番終了などが残る）が、念のため
            if filtered.is_empty() { actions } else { &filtered }
        } else {
            actions
        };

        // 初期配置は専用の評価関数（実測で詰めてある）
        match v.prompt() {
            Prompt::SetupSettlement => {
                let nodes: Vec<NodeId> = actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupSettlement(n) => Some(*n),
                        _ => None,
                    })
                    .collect();
                return Action::SetupSettlement(best_setup_settlement(v, &nodes, &self.pw));
            }
            Prompt::SetupRoad => {
                let edges: Vec<EdgeId> = actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupRoad(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
                return Action::SetupRoad(best_setup_road(v, &edges, &self.pw));
            }
            _ => {}
        }

        if actions.len() == 1 {
            return actions[0];
        }

        // 前に指したボットが枝刈りの倍率を変えているかもしれないので、必ず立て直す
        crate::prune::set_keep_mul(1.0);
        // 相手の発展カードは見えないので、あり得る局面を 1 つ立ててその上で読む
        let g = v.determinize(&mut self.rng);
        let me = v.me;
        // 盤面由来の項と脅威度は手ごとに変わらないので 1 度だけ
        let base = evaluate(&g, me, &self.w);
        let th = threats(&g, &self.w);
        // 安い物差しで候補を削ってから完全に評価する
        let cand = prune(&g, me, actions, &self.w, &th);
        let mut best = cand[0];
        let mut best_score = f32::NEG_INFINITY;
        let salt = self.rng.next_u32();
        for (i, a) in cand.iter().enumerate() {
            let mut s = eval_after_with_base(&g, *a, me, &self.w, base, &th);
            // 同点を決定的に割らない（席バイアスを作らないため）
            s += (((salt.wrapping_add(i as u32)).wrapping_mul(2654435761) >> 8) & 0xFF) as f32
                * 1e-4;
            if s > best_score {
                best_score = s;
                best = *a;
            }
        }
        best
    }
}

// =========================================================================

/// 手番内を先読みするボット。評価関数は [`GreedyEvalBot`] と同じ。
///
/// 違うのは「1 手ずつ最善を選ぶ」か「手番の中の**手順**を組むか」だけなので、
/// 勝率の差はまるごと先読みの効果になる。
pub struct SearchBot {
    rng: Rng,
    w: EvalWeights,
    pw: PlacementWeights,
    lim: SearchLimits,
    /// 枝刈りで残す数の倍率。1 が従来。**手ごとに立て直す**ので、
    /// 同じ試合の中で倍率の違うボットを並べても混ざらない
    keep_mul: f32,
    /// 伏せカードの引き直しを何回やって平均するか。1 が従来
    worlds: u32,
    /// 相手の資源の中身を覗く（実卓ではできない）。隠す代償を測るための刻み
    see_hands: bool,
    /// 多めに積む形（3:1 / 1:3 / 2:2）を **考えない**。
    /// 生成を広げたことが効いているかを、同じ盤・同じ規則の下で測るための刻み
    narrow_shapes: bool,
    label: String,
}

impl SearchBot {
    pub fn new(seed: u64, depth: u32) -> Self {
        SearchBot {
            rng: Rng::new(seed),
            w: EvalWeights::default(),
            pw: PlacementWeights::default(),
            lim: SearchLimits {
                depth,
                ..SearchLimits::default()
            },
            keep_mul: 1.0,
            worlds: 1,
            narrow_shapes: false,
            see_hands: false,
            label: format!("Search(d={depth})"),
        }
    }

    /// 相手の資源の中身が見える版（隠す前の振る舞い。代償の測定用）
    pub fn peeking(seed: u64, w: EvalWeights, pw: PlacementWeights) -> Self {
        let mut b = Self::with_weights(seed, 2, w, pw, "最強候補(手札を覗く)");
        b.see_hands = true;
        b
    }

    /// 多めに積む形を考えない版（生成を広げた効果を測る）
    pub fn narrow_shapes(seed: u64, w: EvalWeights, pw: PlacementWeights) -> Self {
        let mut b = Self::with_weights(seed, 2, w, pw, "最強候補(束は2枚まで)");
        b.narrow_shapes = true;
        b
    }

    /// 伏せカードの世界を何本か引いて平均する版
    pub fn with_worlds(seed: u64, depth: u32, worlds: u32) -> Self {
        let mut b = Self::new(seed, depth);
        b.worlds = worlds;
        b.label = format!("Search(d={depth},世界{worlds})");
        b
    }

    /// 世界の本数だけ変えた最強候補（他は同じ）
    pub fn tuned_worlds(seed: u64, worlds: u32, w: EvalWeights, pw: PlacementWeights) -> Self {
        let mut b = Self::with_weights(seed, 2, w, pw, "最強候補");
        b.worlds = worlds;
        b.label = format!("最強候補(世界{worlds})");
        b
    }

    /// 重みを差し替えた版（重みの自動調整に使う）
    pub fn with_weights(seed: u64, depth: u32, w: EvalWeights, pw: PlacementWeights, label: &str) -> Self {
        SearchBot {
            rng: Rng::new(seed),
            w,
            pw,
            lim: SearchLimits { depth, ..SearchLimits::default() },
            keep_mul: 1.0,
            worlds: 1,
            narrow_shapes: false,
            see_hands: false,
            label: label.to_string(),
        }
    }

    /// 枝刈りを緩めた版。残す数を `mul` 倍する
    pub fn with_keep(seed: u64, depth: u32, mul: f32) -> Self {
        let mut b = Self::new(seed, depth);
        b.keep_mul = mul;
        b.label = format!("Search(d={depth},keep×{mul})");
        b
    }

    /// 展開してよい局面数を変えた版。予算が足りているかを A/B で測るためのもの。
    /// 予算に当たっている間は、深さを増やしても葉で打ち切られて意味が無い。
    pub fn with_budget(seed: u64, depth: u32, max_nodes: u32) -> Self {
        SearchBot {
            rng: Rng::new(seed),
            w: EvalWeights::default(),
            pw: PlacementWeights::default(),
            lim: SearchLimits { depth, max_nodes },
            keep_mul: 1.0,
            worlds: 1,
            narrow_shapes: false,
            see_hands: false,
            label: format!("Search(d={depth},n={max_nodes})"),
        }
    }
}

impl Bot for SearchBot {
    fn name(&self) -> String {
        self.label.clone()
    }
    fn reset(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }
    fn decide(&mut self, v: &View, actions: &[Action]) -> Action {
        match v.prompt() {
            Prompt::SetupSettlement => {
                let nodes: Vec<NodeId> = actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupSettlement(n) => Some(*n),
                        _ => None,
                    })
                    .collect();
                return Action::SetupSettlement(best_setup_settlement(v, &nodes, &self.pw));
            }
            Prompt::SetupRoad => {
                let edges: Vec<EdgeId> = actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupRoad(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
                return Action::SetupRoad(best_setup_road(v, &edges, &self.pw));
            }
            _ => {}
        }
        if actions.len() == 1 {
            return actions[0];
        }
        // 多めに積む形を外して考える（A/B 用）。ルールは変えず、見る範囲だけ狭める
        let filtered: Vec<Action>;
        let actions = if self.narrow_shapes {
            filtered = actions
                .iter()
                .filter(|a| match a {
                    Action::OfferTrade { give, want } | Action::CounterOffer { give, want } => {
                        catan_core::game::canonical_offer_index(give, want).map_or(true, |i| i < 120)
                    }
                    _ => true,
                })
                .copied()
                .collect();
            if filtered.is_empty() { actions } else { &filtered }
        } else {
            actions
        };

        crate::prune::set_keep_mul(self.keep_mul);
        if self.worlds > 1 {
            return best_action_worlds(v, v.me, actions, &self.w, &self.lim, &mut self.rng, self.worlds);
        }
        let g = v.determinize_with(&mut self.rng, !self.see_hands);
        best_action(&g, v.me, actions, &self.w, &self.lim)
    }
}
