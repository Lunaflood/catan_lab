use catan_ai::economy::build_time;
use catan_ai::setup_search::{portfolio, rank};
use catan_ai::agent_v2::{AgentV2, V2Config};
use catan_ai::eval::{chance_outcomes, EvalWeights};
use catan_ai::search::{SearchHooks, SearchLimits};
use catan_ai::turn_planner::best_action_with_strategy;
use catan_core::action::{Action, DevCard, Prompt};
use catan_core::board::{Building, BuildingKind, PortKind, Resource};
use catan_core::game::{Game, GameConfig};
use catan_core::observation::LEGACY_APP;
use catan_core::observer::observe;
use catan_core::rng::Rng;

#[test]
fn one_surplus_pool_cannot_pay_two_deficits_twice() {
    let time = build_time(&[0; 5], &[1.0, 0.0, 0.0, 0.0, 0.0], &[4.0; 5], &[0, 0, 0, 1, 1]);
    assert!((time - 8.0).abs() < 0.01);
}

#[test]
fn specific_port_exchanges_the_given_resource() {
    let income = [1.0, 0.0, 0.0, 0.0, 0.0];
    let cost = [0, 0, 0, 1, 1];
    let wood_port = build_time(&[0; 5], &income, &[2.0, 4.0, 4.0, 4.0, 4.0], &cost);
    let ore_port = build_time(&[0; 5], &income, &[4.0, 4.0, 4.0, 4.0, 2.0], &cost);
    assert!((wood_port - 4.0).abs() < 0.01);
    assert!((ore_port - 8.0).abs() < 0.01);
    assert!(build_time(&[0; 5], &[0.0; 5], &[4.0; 5], &cost).is_infinite());
}

#[test]
fn portfolio_values_matching_ports_and_number_coverage() {
    let mut g = Game::new(4, 31);
    let node = (0..54).find(|&n| catan_core::topology::Topology::get().node_tiles[n].len() > 1).unwrap();
    g.board.building[node] = Some(Building { owner: 0, kind: BuildingKind::Settlement });
    g.board.tile_resource.fill(Some(Resource::Wood));
    g.board.tile_number.fill(6);
    g.board.node_port[node] = Some(PortKind::Specific(Resource::Ore));
    let wrong = portfolio(&g, 0);
    g.board.node_port[node] = Some(PortKind::Specific(Resource::Wood));
    assert!(portfolio(&g, 0) > wrong);
    // Same pips and same resources; spreading across 6 and 8 is more reliable.
    let before = portfolio(&g, 0);
    let tiles = catan_core::topology::Topology::get().node_tiles[node].as_slice();
    if tiles.len() > 1 {
        g.board.tile_number[tiles[0] as usize] = 8;
        assert!(portfolio(&g, 0) > before);
    }
}

#[test]
fn setup_search_uses_all_candidates_without_reading_live_rng() {
    for n in [3, 4] {
        let g = Game::new(n, 23);
        let original = g.clone();
        let mut twin = g.clone();
        twin.rng_dice = Rng::new(987);
        twin.rng_steal = Rng::new(988);
        let a = observe(&g, 0, &g.legal_actions(), LEGACY_APP, 0);
        let b = observe(&twin, 0, &twin.legal_actions(), LEGACY_APP, 0);
        assert_eq!(a, b);
        let values = rank(&a);
        assert_eq!(values.len(), a.legal.len());
        let twin_values = rank(&b);
        for (x, y) in values.iter().zip(twin_values.iter()) {
            assert_eq!(x.action, y.action);
            assert_eq!(x.value, y.value);
            assert!(x.value.is_finite());
        }
        let mut agent = AgentV2::new(1, V2Config::v4());
        let choice = agent.decide(&a);
        assert!(g.can_apply(&choice));
        assert_eq!(g, original);
    }
}

#[test]
fn complete_snake_setup_is_legal_for_three_and_four_players() {
    for n in [3, 4] {
        let mut g = Game::new(n, 99);
        let mut agents: Vec<_> = (0..n).map(|p| AgentV2::new(p as u64, V2Config::v4())).collect();
        while g.is_setup() {
            let p = g.to_act;
            let obs = observe(&g, p, &g.legal_actions(), LEGACY_APP, 0);
            let action = agents[p as usize].decide(&obs);
            assert!(g.can_apply(&action));
            g.apply(action);
        }
        for p in 0..n { assert_eq!(g.public_vp(p), 2); }
    }
}

#[test]
fn development_draws_use_remaining_counts_and_deplete_without_replacement() {
    let mut g = Game::new(4, 1);
    let initial = chance_outcomes(&g, Action::BuyDevCard);
    let probability = |out: &Vec<(catan_core::game::Forced, f32)>, card| out.iter().find_map(|(f, p)|
        if *f == catan_core::game::Forced::Draw(card) { Some(*p) } else { None }).unwrap_or(0.0);
    assert!((probability(&initial, DevCard::Knight) - 14.0 / 25.0).abs() < 1e-6);
    assert!((probability(&initial, DevCard::VictoryPoint) - 5.0 / 25.0).abs() < 1e-6);
    g.dev_deck[DevCard::Knight.idx()] -= 1;
    let next = chance_outcomes(&g, Action::BuyDevCard);
    assert!((probability(&next, DevCard::Knight) - 13.0 / 24.0).abs() < 1e-6);
    g.dev_deck[DevCard::VictoryPoint.idx()] = 0;
    assert_eq!(probability(&chance_outcomes(&g, Action::BuyDevCard), DevCard::VictoryPoint), 0.0);
}

#[test]
fn guaranteed_city_win_beats_a_speculative_development_draw() {
    let mut g = Game::with_config(4, 100, GameConfig { domestic_trade: false, turn_time_limit_ms: None, ..Default::default() });
    g.setup_index = 8;
    g.prompt = Prompt::PlayTurn;
    g.rolled = true;
    for i in 0..4 {
        let n = (0..54).find(|&n| g.can_place_settlement(n, 0, true)).unwrap();
        g.board.building[n as usize] = Some(Building { owner: 0, kind: if i == 0 { BuildingKind::City } else { BuildingKind::Settlement } });
    }
    g.players[0].cities_left = 3;
    g.players[0].settlements_left = 2;
    g.players[0].dev[4] = 4;
    g.dev_deck[4] = 1;
    g.players[0].hand = [0, 0, 1, 2, 3];
    for r in 0..5 { g.bank[r] -= g.players[0].hand[r]; }
    assert_eq!(g.actual_vp(0), 9);
    assert!(g.legal_actions().contains(&Action::BuyDevCard));
    let (action, _) = best_action_with_strategy(&[g.clone()], 0, &g.legal_actions(), &EvalWeights::tuned(),
        &SearchLimits { depth: 4, max_nodes: 60_000 },
        &SearchHooks { threat_bonus: [0.0; 4], root_adjust: None, leaf: None }, true);
    assert!(matches!(action, Action::BuildCity(_)));
    g.apply(action);
    assert_eq!(g.winner, Some(0));
}

#[test]
fn blocking_the_only_settlement_site_removes_a_rivals_win() {
    use catan_core::topology::{Topology, NUM_NODES};
    let topo = Topology::get();
    let mut g = Game::with_config(4, 8, GameConfig { domestic_trade: false, turn_time_limit_ms: None, ..Default::default() });
    g.setup_index = 8;
    g.prompt = Prompt::PlayTurn;
    g.rolled = true;
    // Sparse, zero-production tactical puzzle: only current resources matter.
    g.board.tile_resource.fill(None);
    let target = 0usize;
    let edges = topo.node_edges[target].as_slice();
    let enemy_edge = edges[0];
    let [a, b] = topo.edge_nodes[enemy_edge as usize];
    let far = if a as usize == target { b } else { a };
    let anchor = topo.node_neighbors[far as usize].into_iter().find(|&n| n as usize != target).unwrap();
    g.board.building[anchor as usize] = Some(Building { owner: 0, kind: BuildingKind::Settlement });
    g.players[0].settlements_left -= 1;
    for _ in 0..4 {
        let n = (0..NUM_NODES).find(|&n| n != target && !topo.node_neighbors[target].as_slice().contains(&(n as u8))
            && g.can_place_settlement(n as u8, 1, true)).unwrap();
        g.board.building[n] = Some(Building { owner: 1, kind: BuildingKind::Settlement });
        g.players[1].settlements_left -= 1;
    }
    g.board.road[enemy_edge as usize] = Some(1);
    g.board.road[edges[1] as usize] = Some(0);
    g.players[0].roads_left -= 1;
    g.players[1].roads_left -= 1;
    g.players[1].dev[4] = 5;
    g.dev_deck[4] = 0;
    for p in 0..2 {
        g.players[p].hand = [1, 1, 1, 1, 0];
        for r in 0..5 { g.bank[r] -= g.players[p].hand[r]; }
    }
    assert!(g.can_place_settlement(target as u8, 0, false));
    assert!(g.can_place_settlement(target as u8, 1, false));
    let delta = catan_ai::strategy::board_hazard_delta(&g, 0, Action::BuildSettlement(target as u8));
    assert!(delta < -0.9, "Blocking the only affordable win must reduce hazard: {delta}");
}

#[test]
fn robber_blocks_the_resource_that_would_complete_a_winning_city() {
    use catan_core::topology::{Topology, NUM_NODES};
    let mut g = Game::with_config(4, 8, GameConfig { domestic_trade: false, turn_time_limit_ms: None, ..Default::default() });
    g.setup_index = 8;
    g.prompt = Prompt::MoveRobber;
    g.rolled = true;
    g.board.tile_resource.fill(None);
    let mut nodes = Vec::new();
    for _ in 0..4 {
        let n = (0..NUM_NODES).find(|&n| g.board.node_port[n].is_none() && g.can_place_settlement(n as u8, 1, true)).unwrap();
        g.board.building[n] = Some(Building { owner: 1, kind: BuildingKind::Settlement });
        g.players[1].settlements_left -= 1;
        nodes.push(n);
    }
    g.players[1].dev[4] = 5;
    g.dev_deck[4] = 0;
    g.players[1].hand = [0, 0, 0, 1, 3];
    for r in 0..5 { g.bank[r] -= g.players[1].hand[r]; }
    let tile = Topology::get().node_tiles[nodes[0]].as_slice()[0];
    g.board.tile_resource[tile as usize] = Some(Resource::Wheat);
    g.board.tile_number[tile as usize] = 6;
    g.board.robber = (tile + 1) % 19;
    let delta = catan_ai::strategy::board_hazard_delta(&g, 0, Action::MoveRobber { tile, victim: Some(1) });
    assert!(delta < -0.1, "Blocking the winning wheat roll must reduce hazard: {delta}");
}
#[test]
fn hazard_cache_preserves_exact_values_across_actions() {
    let mut g = Game::new(4, 77);
    g.setup_index = 8;
    g.prompt = Prompt::MoveRobber;
    g.players[1].dev[4] = 5;
    g.dev_deck[4] = 0;
    for _ in 0..4 {
        let n = (0..54).find(|&n| g.can_place_settlement(n, 1, true)).unwrap();
        g.board.building[n as usize] = Some(Building { owner: 1, kind: BuildingKind::Settlement });
        g.players[1].settlements_left -= 1;
    }
    g.players[1].hand = [0, 0, 0, 1, 3];
    for r in 0..5 { g.bank[r] -= g.players[1].hand[r]; }
    let mut cache = catan_ai::strategy::BoardHazards::default();
    for action in g.legal_actions().into_iter().rev() {
        let fresh = catan_ai::strategy::board_hazard_delta(&g, 0, action);
        assert_eq!(cache.delta(&g, 0, action), fresh);
        assert_eq!(cache.delta(&g, 0, action), fresh);
    }
}