//! Official FAQ: existing road pieces beyond a break remain usable.
//! https://www.catan.com/faq/basegame
use catan_core::action::{Action, Prompt};
use catan_core::board::{Building, BuildingKind};
use catan_core::game::{Game, GameConfig};
use catan_core::longest_road::longest_road_length;
use catan_core::topology::{Topology, NUM_NODES};

fn path() -> Vec<u8> {
    fn extend(nodes: &mut Vec<u8>) -> bool {
        if nodes.len() == 7 { return true; }
        let topo = Topology::get();
        for next in &topo.node_neighbors[*nodes.last().unwrap() as usize] {
            if nodes.contains(&next) || nodes[..nodes.len()-1].iter().any(|&n| topo.node_neighbors[n as usize].as_slice().contains(&next)) { continue; }
            nodes.push(next);
            if extend(nodes) { return true; }
            nodes.pop();
        }
        false
    }
    for start in 0..NUM_NODES {
        let mut nodes = vec![start as u8];
        if extend(&mut nodes) { return nodes; }
    }
    panic!("No induced path");
}
fn edge(a: u8, b: u8) -> u8 {
    let topo = Topology::get();
    topo.node_edges[a as usize].into_iter().find(|&e| topo.edge_nodes[e as usize].contains(&b)).unwrap()
}
fn fixture(players: u8, me: u8) -> Game {
    let mut g = Game::with_config(players, 42, GameConfig { domestic_trade: false, ..Default::default() });
    g.setup_index = players * 2;
    g.to_act = me; g.turn_player = me; g.rolled = true; g.prompt = Prompt::PlayTurn;
    g.players[me as usize].hand = [5,5,2,2,0];
    for r in 0..5 { g.bank[r] -= g.players[me as usize].hand[r]; }
    g
}
#[test]
fn opponent_buildings_block_new_roads_in_paid_and_free_road_phases() {
    let ns = path();
    for players in [3,4] { for me in 0..players { for kind in [BuildingKind::Settlement, BuildingKind::City] {
        let mut g = fixture(players, me);
        g.board.road[edge(ns[0],ns[1]) as usize] = Some(me);
        g.players[me as usize].roads_left -= 1;
        g.board.building[ns[1] as usize] = Some(Building { owner: (me+1)%players, kind });
        let forbidden = Action::BuildRoad(edge(ns[1],ns[2]));
        for free in [false,true] {
            g.prompt = if free { Prompt::FreeRoad } else { Prompt::PlayTurn };
            g.free_roads = if free { 2 } else { 0 };
            assert!(!g.can_build_road(edge(ns[1],ns[2]),me));
            assert!(!g.legal_actions().contains(&forbidden));
            assert!(!g.can_apply(&forbidden));
        }
    }}}
}
#[test]
fn preexisting_roads_beyond_a_break_allow_extension_and_settlement() {
    let ns = path();
    for players in [3,4] { for me in 0..players { for kind in [BuildingKind::Settlement, BuildingKind::City] {
        let mut g = fixture(players, me);
        for pair in ns[..6].windows(2) { g.board.road[edge(pair[0],pair[1]) as usize] = Some(me); }
        g.players[me as usize].roads_left -= 5;
        g.board.building[ns[0] as usize] = Some(Building { owner: me, kind: BuildingKind::Settlement });
        g.players[me as usize].settlements_left -= 1;
        assert_eq!(longest_road_length(&g.board,me),5);
        g.board.building[ns[2] as usize] = Some(Building { owner: (me+1)%players, kind });
        assert_eq!(longest_road_length(&g.board,me),3);
        assert!(!g.can_place_settlement(ns[3],me,false)); // Distance rule still applies.
        let extension = Action::BuildRoad(edge(ns[5],ns[6]));
        assert!(g.can_apply(&extension));
        g.apply(extension);
        assert_eq!(longest_road_length(&g.board,me),4);
        let settlement = Action::BuildSettlement(ns[6]);
        assert!(g.can_apply(&settlement));
        g.apply(settlement);
        assert_eq!(g.board.building[ns[6] as usize].unwrap().owner,me);
    }}}
}
