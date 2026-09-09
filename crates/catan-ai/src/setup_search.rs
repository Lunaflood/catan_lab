//! Public-information setup search through the snake placement order.
//! Three opponent models compete for locations from THEIR own perspective.
//! The root chooses a pair/road plan robust to those responses, not merely the
//! highest-pip vertex. Predictions are heuristics, not a solved equilibrium.
use catan_core::action::{Action, Prompt};
use catan_core::board::{Building, BuildingKind, PlayerId};
use catan_core::game::{Game, Forced, COST_CITY, COST_DEV};
use catan_core::observation::Observation;
use catan_core::topology::{Topology, NUM_NODES};
use catan_core::view::View;
use crate::economy::{build_time, exchange_rates};
use crate::placement::{best_setup_road, score_settlement, my_production, PlacementWeights};
use crate::search::RootValue;

/// Joint production, numbers and ports. The city/development/expansion rates
/// share one resource budget per goal; missing resources are not conjured up.
pub fn portfolio(g: &Game, p: PlayerId) -> f32 {
    let prod = my_production(&g.board, p);
    let income = std::array::from_fn(|r| prod[r] as f32 / 36.0);
    let exchange = exchange_rates(&g.board, p);
    let speed = |cost: &[u8; 5]| 1.0 / build_time(&[0; 5], &income, &exchange, cost).max(1.0);
    let mut score = prod.iter().sum::<u16>() as f32 * 2.0;
    score += 65.0 * speed(&COST_CITY) + 45.0 * speed(&COST_DEV)
        + 55.0 * speed(&[2, 2, 1, 1, 0]);
    score += 1.5 * prod.iter().filter(|&&x| x > 0).count() as f32;
    // Probability of receiving something on a roll, counted once per number.
    let topo = Topology::get();
    let mut numbers = [false; 13];
    for n in 0..NUM_NODES {
        if matches!(g.board.building[n], Some(b) if b.owner == p) {
            for t in &topo.node_tiles[n] {
                if g.board.tile_resource[t as usize].is_some() {
                    numbers[g.board.tile_number[t as usize] as usize] = true;
                }
            }
        }
    }
    score += (2..=12).filter(|&n| numbers[n])
        .map(|n| catan_core::board::pips(n as u8) as f32).sum::<f32>() * 0.15;
    score
}

fn local_node(g: &Game, p: PlayerId, node: u8, style: usize) -> f32 {
    if style == 0 {
        return score_settlement(&View::new(g, p), node, &PlacementWeights::tuned());
    }
    let mut sim = g.clone();
    sim.board.building[node as usize] = Some(Building { owner: p, kind: BuildingKind::Settlement });
    let mut score = portfolio(&sim, p);
    if style == 2 {
        // A resource-diversifying rival is a different plausible response to
        // the production/ore-wheat-oriented policy, not random noise.
        score += 4.0 * my_production(&sim.board, p).iter().filter(|&&v| v > 0).count() as f32;
    }
    score
}

fn choose_local(g: &Game, style: usize) -> Action {
    let legal = g.legal_actions();
    match g.prompt {
        Prompt::SetupSettlement => legal.into_iter().max_by(|a, b| {
            let val = |a: &Action| match a { Action::SetupSettlement(n) => local_node(g, g.to_act, *n, style), _ => f32::NEG_INFINITY };
            val(a).total_cmp(&val(b))
        }).unwrap(),
        Prompt::SetupRoad => {
            let edges: Vec<_> = legal.iter().filter_map(|a| match a { Action::SetupRoad(e) => Some(*e), _ => None }).collect();
            Action::SetupRoad(best_setup_road(&View::new(g, g.to_act), &edges, &PlacementWeights::tuned()))
        }
        _ => unreachable!(),
    }
}

fn expansion_quality(g: &Game, p: PlayerId) -> f32 {
    let topo = Topology::get();
    let mut best = 0.0f32;
    for e in 0..catan_core::topology::NUM_EDGES {
        if g.board.road[e] != Some(p) { continue; }
        for n in topo.edge_nodes[e] {
            if matches!(g.board.building[n as usize], Some(b) if b.owner != p) { continue; }
            for m in &topo.node_neighbors[n as usize] {
                if g.can_place_settlement(m, p, true) {
                    let edge = topo.node_edges[n as usize].into_iter().find(|&e| topo.edge_nodes[e as usize].contains(&m)).unwrap();
                    if g.board.road[edge as usize].is_none() || g.board.road[edge as usize] == Some(p) {
                        best = best.max(g.board.node_pips(m).iter().map(|&v| v as f32).sum());
                    }
                }
            }
        }
    }
    best
}

fn result_value(g: &Game, me: PlayerId) -> f32 {
    let own = portfolio(g, me) + 0.4 * expansion_quality(g, me);
    let rivals: Vec<_> = (0..g.n()).filter(|&p| p != me as usize)
        .map(|p| portfolio(g, p as u8) + 0.4 * expansion_quality(g, p as u8)).collect();
    let strongest = rivals.iter().copied().fold(0.0, f32::max);
    own - 0.20 * strongest - 0.05 * rivals.iter().sum::<f32>()
}

pub fn rank(obs: &Observation) -> Vec<RootValue> {
    assert!(matches!(obs.public.game.prompt, Prompt::SetupSettlement | Prompt::SetupRoad));
    obs.legal.iter().map(|&action| {
        let mut sum = 0.0;
        let mut worst = f32::INFINITY;
        for style in 0..3 {
            let mut sim = obs.public.game.clone();
            sim.players[obs.viewer as usize].hand = obs.own.hand;
            sim.apply_forced(action, Forced::No);
            while sim.is_setup() {
                let policy = if sim.to_act == obs.viewer { 1 } else { style };
                let a = choose_local(&sim, policy);
                sim.apply_forced(a, Forced::No);
            }
            let value = result_value(&sim, obs.viewer);
            sum += value;
            worst = worst.min(value);
        }
        RootValue { action, value: 0.75 * sum / 3.0 + 0.25 * worst, adjust: 0.0 }
    }).collect()
}
