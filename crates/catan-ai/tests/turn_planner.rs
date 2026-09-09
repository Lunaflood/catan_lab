use catan_ai::agent_v2::{AgentV2, V2Config};
use catan_ai::eval::EvalWeights;
use catan_ai::search::{best_action_samples, RootValue, SearchHooks, SearchLimits};
use catan_ai::turn_planner;
use catan_core::action::{Action, DevCard, Prompt};
use catan_core::board::{Building, BuildingKind, Resource};
use catan_core::game::{Game, GameConfig};
use catan_core::observation::LEGACY_APP;
use catan_core::observer::observe;
use catan_core::rng::Rng;
use catan_core::topology::NUM_NODES;

fn position() -> Game {
    let mut g = Game::with_config(4, 100, GameConfig {
        domestic_trade: false, turn_time_limit_ms: None, ..GameConfig::default()
    });
    g.setup_index = 8;
    g.prompt = Prompt::PlayTurn;
    g.rolled = true;
    for _ in 0..4 {
        let n = (0..NUM_NODES).find(|&n| g.board.node_port[n].is_none()
            && g.can_place_settlement(n as u8, 0, true)).unwrap();
        g.board.building[n] = Some(Building { owner: 0, kind: BuildingKind::Settlement });
        g.players[0].settlements_left -= 1;
    }
    g.players[0].dev[DevCard::VictoryPoint.idx()] = 5;
    g.dev_deck[DevCard::VictoryPoint.idx()] = 0;
    assert_eq!(g.actual_vp(0), 9);
    g
}

fn hand(g: &mut Game, p: usize, h: [u8; 5]) {
    for r in 0..5 { g.bank[r] += g.players[p].hand[r]; g.bank[r] -= h[r]; }
    g.players[p].hand = h;
}

fn score(g: &Game, depth: u32) -> (Action, Vec<RootValue>) {
    turn_planner::best_action(&[g.clone()], g.to_act, &g.legal_actions(), &EvalWeights::tuned(),
        &SearchLimits { depth, max_nodes: 60_000 },
        &SearchHooks { threat_bonus: [0.0; 4], root_adjust: None, leaf: None })
}

#[test]
fn two_bank_trades_then_city_is_seen_as_a_win() {
    let mut g = position();
    hand(&mut g, 0, [8, 0, 0, 0, 3]);
    let (_, old) = best_action_samples(&[g.clone()], 0, &g.legal_actions(), &EvalWeights::tuned(),
        &SearchLimits { depth: 2, max_nodes: 4000 },
        &SearchHooks { threat_bonus: [0.0; 4], root_adjust: None, leaf: None });
    assert!(old.iter().all(|x| x.value < 90_000.0));
    let (a, values) = score(&g, 4);
    assert_eq!(a, Action::MaritimeTrade { give: Resource::Wood, count: 4, take: Resource::Wheat });
    assert!(values.iter().find(|x| x.action == a).unwrap().value > 90_000.0);
    for _ in 0..3 {
        let (a, _) = score(&g, 4);
        assert!(g.can_apply(&a));
        g.apply(a);
    }
    assert_eq!(g.winner, Some(0));
}

#[test]
fn road_building_card_then_two_roads_then_settlement() {
    let mut g = position();
    hand(&mut g, 0, [1, 1, 1, 1, 0]);
    g.players[0].dev[DevCard::RoadBuilding.idx()] = 1;
    g.dev_deck[DevCard::RoadBuilding.idx()] -= 1;
    let (a, values) = score(&g, 4);
    assert_eq!(a, Action::PlayRoadBuilding);
    assert!(values.iter().find(|x| x.action == a).unwrap().value > 90_000.0);
    for _ in 0..4 {
        if g.is_over() { break; }
        let (a, _) = score(&g, 4);
        g.apply(a);
    }
    assert_eq!(g.winner, Some(0));
}

#[test]
fn domestic_trade_is_evaluated_through_the_resulting_build() {
    let mut g = position();
    g.cfg.domestic_trade = true;
    hand(&mut g, 0, [1, 0, 0, 1, 3]);
    hand(&mut g, 1, [0, 1, 1, 2, 0]);
    let offer = Action::OfferTrade { give: [1, 0, 0, 0, 0], want: [0, 0, 0, 1, 0] };
    assert!(g.legal_actions().contains(&offer));
    let (_, values) = score(&g, 4);
    assert!(values.iter().find(|v| v.action == offer).unwrap().value > 90_000.0);
}

#[test]
fn all_robber_choices_are_evaluated_and_input_is_unchanged() {
    let mut g = position();
    g.prompt = Prompt::MoveRobber;
    let original = g.clone();
    let legal = g.legal_actions();
    let (_, values) = score(&g, 4);
    assert_eq!(values.len(), legal.len());
    assert!(legal.iter().all(|a| values.iter().any(|v| &v.action == a)));
    assert_eq!(g, original);
}

#[test]
fn root_reordering_does_not_change_unique_winning_trade() {
    let mut g = position();
    hand(&mut g, 0, [8, 0, 0, 0, 3]);
    let (expected, _) = score(&g, 4);
    let mut legal = g.legal_actions();
    legal.reverse();
    let (actual, _) = turn_planner::best_action(&[g], 0, &legal, &EvalWeights::tuned(),
        &SearchLimits { depth: 4, max_nodes: 60_000 },
        &SearchHooks { threat_bonus: [0.0; 4], root_adjust: None, leaf: None });
    assert_eq!(actual, expected);
}

#[test]
fn live_hidden_cards_and_rng_do_not_change_v3_decision() {
    let mut g = position();
    // Leave hidden VP in the deck so the twin can redistribute them legally.
    g.players[0].dev[4] = 2;
    g.dev_deck[4] = 3;
    hand(&mut g, 0, [8, 0, 0, 0, 3]);
    hand(&mut g, 1, [1, 1, 0, 0, 0]);
    hand(&mut g, 2, [0, 0, 1, 1, 0]);
    g.players[1].dev[0] = 1;
    g.dev_deck[0] -= 1;
    let mut twin = g.clone();
    twin.players[1].hand = g.players[2].hand;
    twin.players[2].hand = g.players[1].hand;
    twin.players[1].dev[0] = 0;
    twin.players[1].dev[4] = 1;
    twin.dev_deck[0] += 1;
    twin.dev_deck[4] -= 1;
    twin.rng_dice = Rng::new(9876);
    twin.rng_dev = Rng::new(9877);
    twin.rng_steal = Rng::new(9878);
    let a = observe(&g, 0, &g.legal_actions(), LEGACY_APP, 0);
    let b = observe(&twin, 0, &twin.legal_actions(), LEGACY_APP, 0);
    assert_eq!(a, b);
    let mut aa = AgentV2::new(42, V2Config::v3());
    let mut bb = AgentV2::new(42, V2Config::v3());
    assert_eq!(aa.decide(&a), bb.decide(&b));
    assert_eq!(aa.explain(), bb.explain());
    // RNG has not advanced while planning.
    assert_eq!(g.rng_dice, Game::with_config(4, 100, g.cfg).rng_dice);
}

#[test]
fn strategic_evaluation_preserves_multi_action_winning_sequences() {
    for strategic in [false, true] {
        let mut g = position();
        hand(&mut g, 0, [8, 0, 0, 0, 3]);
        for _ in 0..3 {
            let (action, _) = turn_planner::best_action_with_strategy(&[g.clone()], 0, &g.legal_actions(), &EvalWeights::tuned(),
                &SearchLimits { depth: 4, max_nodes: 60_000 },
                &SearchHooks { threat_bonus: [0.0; 4], root_adjust: None, leaf: None }, strategic);
            g.apply(action);
        }
        assert_eq!(g.winner, Some(0));
    }
}