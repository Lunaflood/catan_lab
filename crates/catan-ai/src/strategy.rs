//! Symmetric strategic features: each rival is evaluated using its OWN sampled
//! resources, production, ports and routes. No live hidden state enters here.
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use catan_core::action::DevCard;
use catan_core::board::{BuildingKind, PlayerId};
use catan_core::game::{Game, COST_CITY, MAX_PLAYERS};
use catan_core::topology::{Topology, NUM_EDGES, NUM_NODES};
use crate::economy::{build_time, exchange_rates};

#[derive(Clone)]
struct PlayerFeatures {
    income: [f32; 5],
    exchange: [f32; 5],
    targets: Vec<(usize, u8, f32)>,
}

pub struct Strategy {
    cache: RefCell<HashMap<Vec<u8>, [PlayerFeatures; MAX_PLAYERS]>>,
}
impl Default for Strategy {
    fn default() -> Self { Self { cache: RefCell::new(HashMap::new()) } }
}

fn distances(g: &Game, p: PlayerId) -> [u8; NUM_NODES] {
    let topo = Topology::get();
    let mut d = [255u8; NUM_NODES];
    let mut queue = VecDeque::new();
    for n in 0..NUM_NODES {
        let own_building = matches!(g.board.building[n], Some(b) if b.owner == p);
        let own_road = topo.node_edges[n].into_iter().any(|e| g.board.road[e as usize] == Some(p));
        let blocked = matches!(g.board.building[n], Some(b) if b.owner != p);
        if !blocked && (own_building || own_road) { d[n] = 0; queue.push_back(n); }
    }
    while let Some(n) = queue.pop_front() {
        if matches!(g.board.building[n], Some(b) if b.owner != p) { continue; }
        for e in &topo.node_edges[n] {
            if matches!(g.board.road[e as usize], Some(q) if q != p) { continue; }
            let [a, b] = topo.edge_nodes[e as usize];
            let m = if a as usize == n { b as usize } else { a as usize };
            let cost = u8::from(g.board.road[e as usize].is_none());
            let nd = d[n].saturating_add(cost);
            if nd <= 3 && nd < d[m] {
                d[m] = nd;
                if cost == 0 { queue.push_front(m); } else { queue.push_back(m); }
            }
        }
    }
    d
}

fn features(g: &Game, p: PlayerId) -> PlayerFeatures {
    let prod = crate::eval::production_pips(&g.board, p);
    let d = distances(g, p);
    let mut targets: Vec<_> = (0..NUM_NODES).filter(|&n| d[n] <= 3 && g.can_place_settlement(n as u8, p, true))
        .map(|n| (n, d[n], g.board.node_pips(n as u8).iter().map(|&v| v as f32).sum::<f32>())).collect();
    targets.sort_by(|a, b| (b.2 / (b.1 as f32 + 1.0)).total_cmp(&(a.2 / (a.1 as f32 + 1.0))));
    PlayerFeatures {
        income: std::array::from_fn(|r| prod[r] as f32 / 36.0),
        exchange: exchange_rates(&g.board, p), targets,
    }
}

fn army_potential(g: &Game, p: usize) -> f32 {
    if g.largest_army_owner == Some(p as u8) { return 0.0; }
    let target = g.largest_army_owner.map_or(3, |q| (g.players[q as usize].played_knights() + 1).max(3)) as f32;
    let played = g.players[p].played_knights() as f32;
    let available = g.players[p].playable_dev(DevCard::Knight) as f32;
    let fresh = g.players[p].dev_bought_this_turn[DevCard::Knight.idx()] as f32;
    let progress = (played + 0.65 * available + 0.5 * fresh).min(target) / target;
    1500.0 * progress * progress
}

fn opportunity(g: &Game, p: usize, all: &[PlayerFeatures; MAX_PLAYERS]) -> f32 {
    let f = &all[p];
    let ps = &g.players[p];
    let time = |cost: &[u8; 5]| build_time(&ps.hand, &f.income, &f.exchange, cost);
    let mut best = 0.0f32;
    if ps.cities_left > 0 {
        let prod = (0..NUM_NODES).filter(|&n| matches!(g.board.building[n], Some(b) if b.owner == p as u8 && b.kind == BuildingKind::Settlement))
            .map(|n| g.board.node_pips(n as u8).iter().map(|&v| v as u16).sum::<u16>()).max();
        if let Some(pips) = prod {
            best = (1000.0 + 300.0 * pips as f32 / 36.0) / (1.0 + time(&COST_CITY) / 4.0);
        }
    }
    if ps.settlements_left > 0 {
        for &(node, roads, pips) in f.targets.iter().take(4) {
            if roads > ps.roads_left { continue; }
            let cost = [1 + roads, 1 + roads, 1, 1, 0];
            let eta = time(&cost);
            let mut secure = 1.0f32;
            for q in 0..g.n() {
                if q == p || g.players[q].settlements_left == 0 { continue; }
                if let Some(&(_, other_roads, _)) = all[q].targets.iter().find(|&&(n, _, _)| n == node) {
                    if other_roads > g.players[q].roads_left { continue; }
                    let rival_eta = build_time(&g.players[q].hand, &all[q].income, &all[q].exchange,
                        &[1 + other_roads, 1 + other_roads, 1, 1, 0]);
                    let before = (q + g.n() - g.turn_player as usize) % g.n()
                        < (p + g.n() - g.turn_player as usize) % g.n();
                    if rival_eta + 1.0 < eta || (before && rival_eta <= eta) { secure = secure.min(0.5); }
                }
            }
            let value = secure * (1000.0 + 300.0 * pips / 36.0) / (1.0 + eta / 4.0);
            best = best.max(value);
        }
    }
    0.20 * best
}

impl Strategy {
    pub fn value(&self, g: &Game, me: PlayerId) -> f32 {
        let mut key = Vec::with_capacity(NUM_NODES + NUM_EDGES + 1);
        key.extend(g.board.building.iter().map(|b| b.map_or(0, |b| 1 + b.owner + if b.kind == BuildingKind::City { 4 } else { 0 })));
        key.extend(g.board.road.iter().map(|p| p.map_or(0, |p| p + 1)));
        key.push(g.board.robber);
        // Terrain/number/port layout is fixed for this decision; ownership and
        // robber position are the only board changes during the turn search.
        let mut cache = self.cache.borrow_mut();
        let all = cache.entry(key).or_insert_with(|| std::array::from_fn(|p| features(g, p as u8)));
        let values: Vec<_> = (0..g.n()).map(|p| {
            let ps = &g.players[p];
            let progress = army_potential(g, p);
            let cards = 75.0 * ps.dev[DevCard::RoadBuilding.idx()] as f32
                + 65.0 * ps.dev[DevCard::YearOfPlenty.idx()] as f32
                + 75.0 * ps.dev[DevCard::Monopoly.idx()] as f32;
            progress + cards + opportunity(g, p, all)
        }).collect();
        let rivals: Vec<_> = (0..g.n()).filter(|&p| p != me as usize)
            .map(|p| values[p] * (1.0 + 0.05 * g.public_vp(p as u8) as f32)).collect();
        values[me as usize] - 0.25 * rivals.iter().copied().fold(0.0, f32::max)
            - 0.05 * rivals.iter().sum::<f32>()
    }
}

/// How much a public board action changes rivals' modeled next-turn win chance.
/// Uses the existing bounded reach solver on each sampled world. This includes
/// loss of a settlement site, an award, and robber-blocked production. It is a
/// model delta, not a proof: ReachSolver's documented approximations still apply.
pub fn board_hazard_delta(g: &Game, me: PlayerId, action: catan_core::action::Action) -> f32 {
    BoardHazards::default().delta(g, me, action)
}

#[derive(Default)]
pub struct BoardHazards {
    cache: HashMap<crate::belief::reach::ReachSignature, f32>,
}
impl BoardHazards {
    fn hazard(&mut self, state: &Game, p: usize) -> f32 {
        let mut others = [0u8; 5];
        for q in 0..state.n() {
            if p != q { for r in 0..5 { others[r] += state.players[q].hand[r]; } }
        }
        let mut solver = crate::belief::reach::ReachSolver::new(state, p as u8,
            state.players[p].dev, state.dev_deck, others, state.bank);
        let key = solver.signature(&state.players[p].hand);
        *self.cache.entry(key).or_insert_with(|| solver.win_prob_next_turn(&state.players[p].hand))
    }

    pub fn delta(&mut self, g: &Game, me: PlayerId, action: catan_core::action::Action) -> f32 {
        use catan_core::action::Action;
        let mut sim = g.clone();
        match action {
            Action::BuildRoad(_) | Action::BuildSettlement(_) | Action::BuildCity(_) | Action::PlayKnight => {
                sim.apply_forced(action, catan_core::game::Forced::No);
            }
            Action::MoveRobber { tile, .. } => sim.board.robber = tile,
            _ => return 0.0,
        }
        if sim.winner == Some(me) { return 0.0; }
        (0..g.n()).filter(|&p| p != me as usize && g.actual_vp(p as u8) >= 7)
            .map(|p| self.hazard(&sim, p) - self.hazard(g, p)).sum()
    }
}