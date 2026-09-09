use catan_core::action::{Action, DevCard, Outcome, Prompt};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::{Game, GameConfig, Forced};
use catan_core::rng::Rng;
use catan_core::topology::{Topology, NUM_TILES};

fn ready(n: u8, seed: u64) -> Game {
    let mut g = Game::with_config(n, seed, GameConfig::default());
    let mut rng = Rng::new(seed + 91);
    while g.is_setup() {
        let acts = g.legal_actions();
        g.apply(acts[rng.below(acts.len() as u32) as usize]);
    }
    for p in 0..g.n() {
        for r in 0..NUM_RESOURCES {
            g.bank[r] += g.players[p].hand[r];
            g.players[p].hand[r] = 0;
        }
    }
    // P1 は資源ゼロで発展カードのみ。手札の種類を混同しないこと。
    g.dev_deck[DevCard::VictoryPoint.idx()] -= 1;
    g.players[1].dev[DevCard::VictoryPoint.idx()] = 1;
    for p in 0..g.n() {
        if p == 1 { continue; }
        for r in 0..NUM_RESOURCES {
            let k = rng.below(4) as u8;
            g.players[p].hand[r] = k;
            g.bank[r] -= k;
        }
    }
    g
}

#[test]
fn robber_targets_and_transfers_match_adjacent_buildings_for_every_seat_and_tile() {
    let topo = Topology::get();
    let mut steals = 0;
    let mut choices = 0;
    for n in [3,4] {
        for seed in 0..40 {
            let base = ready(n, seed);
            for me in 0..n {
                let mut g = base.clone();
                g.to_act = me; g.turn_player = me; g.rolled = true;
                g.prompt = Prompt::MoveRobber;
                let acts = g.legal_actions();
                for tile in 0..NUM_TILES {
                    let mut expected = Vec::new();
                    for q in 0..n {
                        if q != me && g.players[q as usize].hand_size() > 0 &&
                            topo.tile_nodes[tile].iter().any(|&node| g.board.building[node as usize].is_some_and(|b| b.owner == q)) {
                            expected.push(Some(q));
                        }
                    }
                    if expected.is_empty() { expected.push(None); }
                    if tile as u8 == g.board.robber { expected.clear(); }
                    let actual: Vec<_> = acts.iter().filter_map(|a| match a {
                        Action::MoveRobber { tile: t, victim } if *t as usize == tile => Some(*victim),
                        _ => None,
                    }).collect();
                    assert_eq!(actual, expected, "seed {seed}, seat {me}, tile {tile}");
                    choices += usize::from(actual.len() >= 2);
                }
                for a in acts {
                    let Action::MoveRobber { tile, victim } = a else { panic!() };
                    let mut next = g.clone();
                    let rec = next.apply(a);
                    assert_eq!(next.board.robber, tile);
                    assert_eq!(next.bank, g.bank);
                    assert_eq!(next.to_act, me);
                    assert_eq!(next.prompt, Prompt::PlayTurn);
                    if let Some(v) = victim {
                        let Outcome::Stole(Some(r)) = rec.result else { panic!("Expected one resource") };
                        for q in 0..g.n() { for k in 0..NUM_RESOURCES {
                            let delta = if k != r.idx() { 0 } else if q == me as usize { 1 } else if q == v as usize { -1 } else { 0 };
                            assert_eq!(next.players[q].hand[k] as i16, g.players[q].hand[k] as i16 + delta);
                        }}
                        steals += 1;
                    } else { assert!(matches!(rec.result, Outcome::Stole(None))); }
                }
            }
        }
    }
    assert!(steals > 1000 && choices > 100);
}

#[test]
fn knight_before_roll_and_seven_after_roll_resume_the_right_phase() {
    for knight in [true,false] {
        let mut g = ready(4, 21);
        if knight {
            g.dev_deck[DevCard::Knight.idx()] -= 1;
            g.players[0].dev[DevCard::Knight.idx()] = 1;
            g.apply(Action::PlayKnight);
        } else {
            g.apply_forced(Action::Roll, Forced::Dice(3,4));
            while g.prompt == Prompt::Discard { g.apply(g.legal_actions()[0]); }
        }
        assert_eq!(g.prompt, Prompt::MoveRobber);
        let a = g.legal_actions().into_iter().find(|a| matches!(a, Action::MoveRobber { victim: Some(_), .. })).unwrap();
        g.apply(a);
        assert_eq!(g.legal_actions().contains(&Action::Roll), knight);
        assert_eq!(g.to_act, 0 as PlayerId);
    }
}
