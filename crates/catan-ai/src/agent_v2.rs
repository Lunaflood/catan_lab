//! v2 エージェント（M1〜M3）: 観測だけを受け取り、推定（Belief）の上で判断する。
//!
//! 受け取るのは [`Observation`] と [`ObservedEvent`] だけ。本番 `Game` も `View` も受けない
//! （`View` → `Observation` の変換はホスト側の `View::observe` が行う）。
//!
//! 判断の骨格は現行の最高難易度と同じ「手番内 2 手読み × 推定サンプル」だが:
//! - 推定サンプルは一様な引き直しではなく、履歴で更新した粒子から重みに比例して引く
//! - 相手の脅威度に、粒子ごとに通した**勝利ハザード**（次の手番で勝つ確率）を上乗せする
//! - 相手の手札を動かす手（交易・盗み・独占）は、その相手のハザードの増減で補正する

use catan_core::action::{Action, Bundle, Prompt};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::{Game, MAX_PLAYERS};
use catan_core::observation::{Observation, ObservedEvent};
use catan_core::rng::Rng;
use catan_core::topology::{EdgeId, NodeId};
use catan_core::view::View;

use crate::belief::reach::ReachSolver;
use crate::belief::{Belief, BeliefConfig};
use crate::bots::Bot;
use crate::eval::{threats_bonus, EvalWeights};
use crate::placement::{best_setup_road, best_setup_settlement, PlacementWeights};
use crate::search::{best_action_samples, RootValue, SearchHooks, SearchLimits};

#[derive(Clone, Copy, Debug)]
pub struct V2Config {
    /// v3 selective turn-sequence planner; false preserves the frozen v2.
    pub turn_planner: bool,
    pub setup_search: bool,
    pub strategic: bool,
    pub belief: BeliefConfig,
    /// 推定サンプル（粒子）の本数
    pub samples: usize,
    pub depth: u32,
    pub max_nodes: u32,
    /// 勝利ハザードの変化 1.0（= 相手の勝率 100%）を評価値でいくらに換算するか。
    /// 評価値の単位は 1 勝利点 ≈ 1000
    pub hazard_weight: f32,
    /// 脅威度への上乗せ係数（ハザード × これ）
    pub hazard_threat: f32,
    /// ハザードを使う（false で M2 相当: 推定サンプルだけ）
    pub use_hazard: bool,
    /// M4: 自分の手番が終わった葉を、相手の手番のロールアウトで評価する割合（0 = 静的評価だけ）
    pub rollout_mix: f32,
    /// 葉 1 つあたりのロールアウト本数
    pub rollouts: u32,
    pub w: EvalWeights,
    pub pw: PlacementWeights,
}

impl Default for V2Config {
    fn default() -> Self {
        V2Config {
            turn_planner: false,
            setup_search: false,
            strategic: false,
            belief: BeliefConfig::default(),
            samples: 3,
            depth: 2,
            max_nodes: 4000,
            hazard_weight: 20_000.0,
            hazard_threat: 3.0,
            use_hazard: true,
            rollout_mix: 0.0,
            rollouts: 1,
            w: EvalWeights::tuned(),
            pw: PlacementWeights::tuned(),
        }
    }
}

/// 1 判断の根拠（説明・記録用）。秘密の正解は含まない
#[derive(Clone, Debug, Default)]
pub struct DecisionEvidence {
    pub chosen: Option<Action>,
    pub top: Vec<RootValue>,
    /// 相手ごとの勝利ハザード（次の自分の手番で勝つ確率）
    pub hazard: [f32; MAX_PLAYERS],
    /// 相手ごとの隠れ勝利点の期待値
    pub expected_hidden_vp: [f32; MAX_PLAYERS],
    pub ess: f32,
    pub samples: usize,
    pub candidates: usize,
}

pub struct AgentV2 {
    pub cfg: V2Config,
    pub belief: Option<Belief>,
    rng: Rng,
    seed: u64,
    pub last: DecisionEvidence,
    pub decisions: u64,
}

impl AgentV2 {
    pub fn new(seed: u64, cfg: V2Config) -> Self {
        AgentV2 {
            cfg,
            belief: None,
            rng: Rng::with_stream(seed, 0xA6E7_0002),
            seed,
            last: DecisionEvidence::default(),
            decisions: 0,
        }
    }

    pub fn reset(&mut self, seed: u64) {
        self.seed = seed;
        self.rng = Rng::with_stream(seed, 0xA6E7_0002);
        self.belief = None;
        self.decisions = 0;
    }

    fn ensure_belief(&mut self, viewer: PlayerId, n: usize) -> &mut Belief {
        if self.belief.as_ref().map_or(true, |b| b.viewer != viewer || b.n != n) {
            self.belief = Some(Belief::new(viewer, n, self.cfg.belief, self.seed ^ 0x5EED_B1F));
        }
        self.belief.as_mut().unwrap()
    }

    /// 他人の手番中も含めて、席に投影された出来事を取り込む
    pub fn observe(&mut self, ev: &ObservedEvent) {
        let n = ev.num_players as usize;
        self.ensure_belief(ev.viewer, n).observe(ev);
    }

    /// 相手ごとの勝利ハザード（粒子で周辺化）
    fn hazards(&self, obs: &Observation) -> [f32; MAX_PLAYERS] {
        let mut out = [0.0f32; MAX_PLAYERS];
        let Some(b) = self.belief.as_ref() else { return out };
        let g = &obs.public.game;
        let bank = obs.public.bank.unwrap_or([0; NUM_RESOURCES]);
        for q in 0..obs.num_players() {
            let p = q as PlayerId;
            if p == obs.viewer {
                continue;
            }
            let mut acc = 0.0f32;
            for (hand, dev, deck, others, w) in b.distinct_states(p) {
                let mut solver = ReachSolver::new(g, p, dev, deck, others, bank);
                let v = if g.turn_player == p && g.rolled { solver.win_prob_now(&hand) } else { solver.win_prob_next_turn(&hand) };
                acc += w * v;
            }
            out[q] = acc;
        }
        out
    }

    pub fn decide(&mut self, obs: &Observation) -> Action {
        self.decisions += 1;
        let me = obs.viewer;
        let n = obs.num_players();
        let legal = &obs.legal;
        assert!(!legal.is_empty(), "合法手が空");
        if self.cfg.setup_search && matches!(obs.public.game.prompt, Prompt::SetupSettlement | Prompt::SetupRoad) {
            let mut values = crate::setup_search::rank(obs);
            values.sort_by(|a, b| b.value.total_cmp(&a.value));
            let chosen = values[0].action;
            let candidates = values.len();
            values.truncate(8);
            self.last = DecisionEvidence { chosen: Some(chosen), top: values, candidates, ..Default::default() };
            return chosen;
        }
        // 秘密を消した局面の上に View を作る（配置の評価関数は公開情報しか読まない）
        let pub_view = View::new(&obs.public.game, me);
        match obs.public.game.prompt {
            Prompt::SetupSettlement => {
                let nodes: Vec<NodeId> = legal
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupSettlement(n) => Some(*n),
                        _ => None,
                    })
                    .collect();
                let a = Action::SetupSettlement(best_setup_settlement(&pub_view, &nodes, &self.cfg.pw));
                self.last = DecisionEvidence { chosen: Some(a), ..Default::default() };
                return a;
            }
            Prompt::SetupRoad => {
                let edges: Vec<EdgeId> = legal
                    .iter()
                    .filter_map(|a| match a {
                        Action::SetupRoad(e) => Some(*e),
                        _ => None,
                    })
                    .collect();
                let a = Action::SetupRoad(best_setup_road(&pub_view, &edges, &self.cfg.pw));
                self.last = DecisionEvidence { chosen: Some(a), ..Default::default() };
                return a;
            }
            _ => {}
        }
        if legal.len() == 1 {
            self.last = DecisionEvidence { chosen: Some(legal[0]), ..Default::default() };
            return legal[0];
        }

        let cfg = self.cfg;
        let mut rng = self.rng;
        {
            let b = self.ensure_belief(me, n);
            b.resync_if_needed(obs);
        }
        let hazard = if cfg.use_hazard { self.hazards(obs) } else { [0.0; MAX_PLAYERS] };
        let belief = self.belief.as_mut().unwrap();
        let idxs = belief.sample_indices(cfg.samples, &mut rng);
        let samples: Vec<Game> = idxs.iter().map(|&ix| belief.materialize(obs, ix, &mut rng)).collect();
        let mut ehv = [0.0f32; MAX_PLAYERS];
        for q in 0..n {
            ehv[q] = belief.expected_hidden_vp(q as PlayerId);
        }
        let ess = belief.ess();
        self.rng = rng;

        let mut bonus = [0.0f32; MAX_PLAYERS];
        for q in 0..n {
            bonus[q] = cfg.hazard_threat * hazard[q];
        }
        let lim = SearchLimits { depth: cfg.depth, max_nodes: cfg.max_nodes };
        let w = cfg.w;
        let bank = obs.public.bank.unwrap_or([0; NUM_RESOURCES]);
        let pub_game = &obs.public.game;
        // 根の補正: 相手の手札を動かす手は、その相手の勝利ハザードの増減で評価を直す
        let board_hazards = std::cell::RefCell::new(crate::strategy::BoardHazards::default());
        let adjust = |_si: usize, g: &Game, a: Action| -> f32 {
            if !cfg.use_hazard || cfg.hazard_weight == 0.0 {
                return 0.0;
            }
            let th = threats_bonus(g, &w, &bonus);
            let mut delta = 0.0f32;
            let hz = |p: PlayerId, before: &Bundle, after: &Bundle| -> f32 {
                if before == after {
                    return 0.0;
                }
                let mut others = [0u8; NUM_RESOURCES];
                for q in 0..n {
                    if q != p as usize {
                        for r in 0..NUM_RESOURCES {
                            others[r] += g.players[q].hand[r];
                        }
                    }
                }
                let mut s = ReachSolver::new(pub_game, p, g.players[p as usize].dev, g.dev_deck, others, bank);
                let now = g.turn_player == p && g.rolled;
                let (b, a2) = if now {
                    (s.win_prob_now(before), s.win_prob_now(after))
                } else {
                    (s.win_prob_next_turn(before), s.win_prob_next_turn(after))
                };
                a2 - b
            };
            match a {
                Action::OfferTrade { .. } | Action::AcceptTrade | Action::CounterOffer { .. } => {
                    if let Some((from, to, give, want)) = crate::trade::resolve(g, me, a, &w, &th) {
                        let partner = if from == me { to } else { from };
                        let before = g.players[partner as usize].hand;
                        let mut after = before;
                        for r in 0..NUM_RESOURCES {
                            if partner == to {
                                after[r] = after[r] + give[r] - want[r];
                            } else {
                                after[r] = after[r] - give[r] + want[r];
                            }
                        }
                        delta += hz(partner, &before, &after);
                    }
                }
                Action::ConfirmTrade(partner) => {
                    if let Some(t) = g.trade {
                        let before = g.players[partner as usize].hand;
                        let mut after = before;
                        for r in 0..NUM_RESOURCES {
                            after[r] = after[r] + t.give[r] - t.want[r];
                        }
                        delta += hz(partner, &before, &after);
                    }
                }
                Action::AcceptCounter { from, alt } => {
                    if let Some(t) = g.trade {
                        if let Some((cgive, cwant)) = t.counters[from as usize][alt as usize] {
                            let before = g.players[from as usize].hand;
                            let mut after = before;
                            for r in 0..NUM_RESOURCES {
                                after[r] = after[r] - cgive[r] + cwant[r];
                            }
                            delta += hz(from, &before, &after);
                        }
                    }
                }
                Action::MoveRobber { victim: Some(v), .. } => {
                    let before = g.players[v as usize].hand;
                    let total: u8 = before.iter().sum();
                    if total > 0 {
                        for r in 0..NUM_RESOURCES {
                            if before[r] == 0 {
                                continue;
                            }
                            let mut after = before;
                            after[r] -= 1;
                            delta += (before[r] as f32 / total as f32) * hz(v, &before, &after);
                        }
                    }
                }
                Action::PlayMonopoly(res) => {
                    for q in 0..n {
                        if q as PlayerId == me {
                            continue;
                        }
                        let before = g.players[q].hand;
                        let mut after = before;
                        after[res.idx()] = 0;
                        delta += hz(q as PlayerId, &before, &after);
                    }
                }
                _ => {}
            }
            if cfg.strategic { delta += board_hazards.borrow_mut().delta(g, me, a); }
            -cfg.hazard_weight * delta
        };
        // M4: 葉のロールアウト（相手の手番を自分の次の手番まで）。RNG はセル越しに使う
        let roll_rng = std::cell::RefCell::new(Rng::new(self.rng.next_u32() as u64));
        let leaf = |g: &Game| -> f32 {
            let st = crate::eval::evaluate(g, me, &w);
            if cfg.rollout_mix <= 0.0 || cfg.rollouts == 0 {
                return st;
            }
            let mut r = roll_rng.borrow_mut();
            let mut acc = 0.0f32;
            for _ in 0..cfg.rollouts {
                acc += crate::search_v2::rollout_value(g, me, &w, &mut r, 1);
            }
            let ro = acc / cfg.rollouts as f32;
            (1.0 - cfg.rollout_mix) * st + cfg.rollout_mix * ro
        };
        let hooks = SearchHooks {
            threat_bonus: bonus,
            root_adjust: Some(&adjust),
            leaf: if cfg.rollout_mix > 0.0 { Some(&leaf) } else { None },
        };
        let (chosen, mut values) = if cfg.turn_planner {
            crate::turn_planner::best_action_with_strategy(&samples, me, legal, &w, &lim, &hooks, cfg.strategic)
        } else {
            best_action_samples(&samples, me, legal, &w, &lim, &hooks)
        };
        values.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
        let candidates = values.len();
        values.truncate(8);
        self.last = DecisionEvidence {
            chosen: Some(chosen),
            top: values,
            hazard,
            expected_hidden_vp: ehv,
            ess,
            samples: samples.len(),
            candidates,
        };
        chosen
    }

    /// 直前の判断の説明（実際に計算した根拠だけ。秘密の正解は出さない）
    pub fn explain(&self) -> String {
        let e = &self.last;
        let mut s = String::new();
        if let Some(a) = e.chosen {
            s.push_str(&format!("選択: {a:?}"));
        }
        if let Some(b) = &self.belief {
            let n = b.n;
            for q in 0..n {
                if q as PlayerId == b.viewer {
                    continue;
                }
                let d = b.vp_dist(q as PlayerId);
                s.push_str(&format!(
                    "\n P{q}: 隠れVP期待 {:.2} [0:{:.0}% 1:{:.0}% 2+:{:.0}%] 次手番の勝利ハザード {:.1}%",
                    e.expected_hidden_vp[q],
                    d[0] * 100.0,
                    d[1] * 100.0,
                    (1.0 - d[0] - d[1]) * 100.0,
                    e.hazard[q] * 100.0
                ));
            }
            s.push_str(&format!("\n 有効粒子数 {:.0}/{} サンプル {} 候補 {}", e.ess, b.dev.len(), e.samples, e.candidates));
        }
        for v in &e.top {
            s.push_str(&format!("\n  {:?}: {:.0} (ハザード補正 {:+.0})", v.action, v.value, v.adjust));
        }
        s
    }
}

/// [`Bot`] への適合。`View` は `View::observe` で観測に変換してから渡す。
/// **v2 は `View` の他のメソッドを呼ばない**（ここが `Observation -> 新Agent` の唯一の入口）。
pub struct AgentV2Bot {
    pub agent: AgentV2,
    label: String,
    seq: u64,
}

impl AgentV2Bot {
    pub fn new(seed: u64, cfg: V2Config, label: &str) -> Self {
        AgentV2Bot { agent: AgentV2::new(seed, cfg), label: label.to_string(), seq: 0 }
    }
    pub fn explain(&self) -> String {
        self.agent.explain()
    }
}

impl Bot for AgentV2Bot {
    fn name(&self) -> String {
        self.label.clone()
    }
    fn reset(&mut self, seed: u64) {
        self.agent.reset(seed);
        self.seq = 0;
    }
    fn wants_events(&self) -> bool {
        true
    }
    fn observe(&mut self, ev: &ObservedEvent) {
        self.seq = ev.seq;
        self.agent.observe(ev);
    }
    fn decide(&mut self, view: &View, actions: &[Action]) -> Action {
        let obs = view.observe(actions, self.seq);
        self.agent.decide(&obs)
    }
}

impl V2Config {
    pub fn v3() -> Self {
        Self { turn_planner: true, depth: 4, max_nodes: 60_000, ..Self::default() }
    }
}

impl V2Config {
    pub fn v4() -> Self {
        Self { setup_search: true, strategic: true, ..Self::v3() }
    }
}
