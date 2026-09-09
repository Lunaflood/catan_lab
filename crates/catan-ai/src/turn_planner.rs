//! Selective expectimax over complete build/trade sequences. The input consists
//! only of worlds sampled from the observation belief, never the live game.
//! All root actions get an expected-value score. Construction, maritime trades,
//! development cards and robber moves remain eligible for deeper search; only
//! negotiations are shortlisted. Descendants use a diverse beam and equal
//! budgets, so action enumeration cannot starve later siblings.
use catan_core::action::{Action, Prompt};
use catan_core::board::PlayerId;
use catan_core::game::Game;
use crate::eval::{chance_outcomes, evaluate, threats_bonus, EvalWeights};
use crate::search::{RootValue, SearchHooks, SearchLimits};

fn family(a: Action) -> u8 {
    match a {
        Action::BuildRoad(_) => 0,
        Action::BuildSettlement(_) => 1,
        Action::BuildCity(_) => 2,
        Action::MaritimeTrade { .. } => 3,
        Action::BuyDevCard => 4,
        Action::PlayKnight => 5,
        Action::PlayRoadBuilding => 6,
        Action::PlayYearOfPlenty(..) => 7,
        Action::PlayMonopoly(_) => 8,
        Action::MoveRobber { .. } => 9,
        _ => 10,
    }
}

struct Planner<'a> {
    me: PlayerId,
    strategy: Option<crate::strategy::Strategy>,
    w: &'a EvalWeights,
    hooks: &'a SearchHooks<'a>,
}

impl Planner<'_> {
    fn leaf(&self, g: &Game) -> f32 {
        if g.is_over() { return evaluate(g, self.me, self.w); }
        let mut score = evaluate(g, self.me, self.w);
        // Removing an opponent's longest road / army is a real score swing.
        // The old evaluation only counted our points and their production.
        let enemy = (0..g.n()).filter(|&p| p != self.me as usize)
            .map(|p| g.public_vp(p as u8) as f32).fold(0.0, f32::max);
        score -= self.w.vp * 0.5 * enemy;
        if let Some(strategy) = &self.strategy { score += strategy.value(g, self.me); }
        score
    }

    fn after(&self, g: &Game, a: Action, depth: u32, budget: u32) -> f32 {
        if crate::trade::is_negotiation(&a) {
            let th = threats_bonus(g, self.w, &self.hooks.threat_bonus);
            let Some((from, to, give, want)) = crate::trade::resolve(g, self.me, a, self.w, &th) else {
                return self.leaf(g) - crate::trade::FAILED_OFFER_PENALTY;
            };
            let mut sim = g.clone();
            for r in 0..5 {
                sim.players[from as usize].hand[r] = sim.players[from as usize].hand[r] - give[r] + want[r];
                sim.players[to as usize].hand[r] = sim.players[to as usize].hand[r] - want[r] + give[r];
            }
            // A prediction of acceptance, not an assertion that the opponent
            // must accept. Never chain further hypothetical negotiations.
            sim.trade = None;
            sim.prompt = Prompt::PlayTurn;
            sim.to_act = sim.turn_player;
            return self.value(&sim, depth.saturating_sub(1), budget) - self.w.trade_cost - 1.0;
        }
        let outcomes = chance_outcomes(g, a);
        let share = budget / outcomes.len().max(1) as u32;
        outcomes.into_iter().map(|(forced, probability)| {
            let mut sim = g.clone();
            sim.apply_forced(a, forced);
            probability * self.value(&sim, depth.saturating_sub(1), share)
        }).sum::<f32>() - if a == Action::EndTurn { 0.0 } else { 1.0 }
    }

    fn value(&self, g: &Game, depth: u32, budget: u32) -> f32 {
        if g.is_over() || g.to_act != self.me || depth == 0 || budget == 0 {
            return self.leaf(g);
        }
        let actions: Vec<_> = g.legal_actions().into_iter().filter(|a| {
            !crate::trade::is_negotiation(a)
                && !matches!(a, Action::CounterAlt { .. } | Action::CounterAltRemove(_))
        }).collect();
        if actions.is_empty() { return self.leaf(g); }
        // Fully score each sibling before assigning further work. Keep an
        // anytime lower bound: deeper search cannot erase a good short plan.
        let mut scored: Vec<_> = actions.iter().map(|&a| (a, self.after(g, a, 1, 0))).collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut best = scored[0].1;
        if depth <= 1 || budget <= actions.len() as u32 { return best; }
        let mut seen = [false; 11];
        let selected: Vec<_> = scored.iter().enumerate().filter_map(|(i, &(a, _))| {
            if a == Action::EndTurn { return None; }
            let f = family(a) as usize;
            let first = !seen[f];
            seen[f] = true;
            if i < 3 || first { Some(a) } else { None }
        }).collect();
        let share = (budget - actions.len() as u32) / selected.len().max(1) as u32;
        for a in selected {
            best = best.max(self.after(g, a, depth, share));
        }
        best
    }
}

pub fn best_action(
    samples: &[Game], me: PlayerId, actions: &[Action], w: &EvalWeights,
    limits: &SearchLimits, hooks: &SearchHooks,
) -> (Action, Vec<RootValue>) {
    best_action_with_strategy(samples, me, actions, w, limits, hooks, false)
}

pub fn best_action_with_strategy(
    samples: &[Game], me: PlayerId, actions: &[Action], w: &EvalWeights,
    limits: &SearchLimits, hooks: &SearchHooks, strategic: bool,
) -> (Action, Vec<RootValue>) {
    assert!(!samples.is_empty() && !actions.is_empty());
    let planner = Planner { me, w, hooks, strategy: strategic.then(crate::strategy::Strategy::default) };
    let k = samples.len() as f32;
    let mut values: Vec<_> = actions.iter().map(|&action| {
        let mut value = 0.0;
        let mut adjust = 0.0;
        for (si, g) in samples.iter().enumerate() {
            let d = hooks.root_adjust.map_or(0.0, |f| f(si, g, action));
            value += planner.after(g, action, 1, 0) + d;
            adjust += d;
        }
        RootValue { action, value: value / k, adjust: adjust / k }
    }).collect();
    let mut ranked: Vec<_> = (0..values.len()).collect();
    ranked.sort_by(|&a, &b| values[b].value.total_cmp(&values[a].value));
    let mut negotiations = 0;
    let selected: Vec<_> = ranked.into_iter().filter(|&i| {
        if crate::trade::is_negotiation(&actions[i]) {
            negotiations += 1;
            negotiations <= 8
        } else { true }
    }).collect();
    let share = limits.max_nodes / selected.len().max(1) as u32;
    for i in selected {
        let mut value = 0.0;
        for g in samples {
            value += planner.after(g, actions[i], limits.depth.max(1), share);
        }
        values[i].value = values[i].value.max(value / k + values[i].adjust);
    }
    let best = values.iter().enumerate().max_by(|(ia, a), (ib, b)| {
        a.value.total_cmp(&b.value).then_with(|| ib.cmp(ia))
    }).unwrap().0;
    (actions[best], values)
}
