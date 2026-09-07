use catan_ai::bots::{bot_for_level, Bot, SearchBot};
use catan_ai::eval::EvalWeights;
use catan_ai::placement::PlacementWeights;
use catan_core::action::{Action, Prompt, DevCard};
use catan_core::game::{Game, GameConfig, TradeState};
use catan_core::rng::Rng;
use catan_core::view::View;

fn ready(seed: u64) -> Game {
    let mut g = Game::with_config(4, seed, GameConfig::default());
    while g.is_setup() { let a=g.legal_actions()[0]; g.apply(a); }
    g.prompt=Prompt::PlayTurn; g.rolled=true; g.turn_player=0; g.to_act=0;
    g.players[0].hand=[2,2,2,2,3];
    g.players[1].hand=[2,0,1,0,0];
    g.players[2].hand=[0,2,0,1,0];
    g.players[3].hand=[0,0,0,0,0];
    for r in 0..5 { g.bank[r]=19-g.players.iter().map(|p|p.hand[r]).sum::<u8>(); }
    g
}

#[test]
fn hidden_resources_development_and_future_do_not_change_decision() {
    for seed in 0..24 {
        let mut a=ready(seed);
        a.players[1].dev[DevCard::Knight.idx()]=1;
        a.dev_deck[DevCard::Knight.idx()]-=1;
        let mut b=a.clone();
        // Preserve public information; alter only hidden resources and development cards.
        b.players[1].hand=a.players[2].hand;
        b.players[2].hand=a.players[1].hand;
        b.players[1].dev=[0;5]; b.players[1].dev[DevCard::VictoryPoint.idx()]=1;
        b.dev_deck[DevCard::VictoryPoint.idx()]-=1; b.dev_deck[DevCard::Knight.idx()]+=1;
        b.rng_dice=Rng::new(991); b.rng_dev=Rng::new(992); b.rng_steal=Rng::new(993);
        let acts=a.legal_actions(); assert_eq!(acts,b.legal_actions());
        let sa=View::new(&a,0).determinize(&mut Rng::new(77));
        let sb=View::new(&b,0).determinize(&mut Rng::new(77));
        assert_eq!(sa.players,sb.players);
        assert_eq!(sa.dev_deck,sb.dev_deck);
        assert_eq!(sa.rng_dice,sb.rng_dice); assert_eq!(sa.rng_dev,sb.rng_dev); assert_eq!(sa.rng_steal,sb.rng_steal);
        for level in 0..4 {
            assert_eq!(bot_for_level(level,123).decide(&View::new(&a,0),&acts),bot_for_level(level,123).decide(&View::new(&b,0),&acts));
        }
        let decide=|g:&Game| SearchBot::tuned_worlds(123,3,EvalWeights::tuned(),PlacementWeights::tuned()).decide(&View::new(g,0),&acts);
        assert_eq!(decide(&a),decide(&b));
    }
}

#[test]
fn every_alternative_and_acceptance_is_payable_in_every_sample() {
    let mut g=ready(9);
    g.players[1].hand=[2,2,1,0,0];
    g.players[2].hand=[0,0,2,0,0];
    for r in 0..5 { g.bank[r]=19-g.players.iter().map(|p|p.hand[r]).sum::<u8>(); }
    let mut t=TradeState {proposer:0,give:[1,0,0,0,0],want:[0,0,1,0,0],responder:3,accepted:[false;4],counters:[[None;4];4],responded:[false;4],answered:3};
    t.accepted[2]=true;
    t.counters[1][0]=Some(([2,0,0,0,0],[0,0,1,0,0]));
    t.counters[1][1]=Some(([0,2,0,0,0],[0,0,1,0,0]));
    t.counters[1][2]=Some(([1,1,0,0,0],[0,0,1,0,0]));
    g.trade=Some(t);g.prompt=Prompt::DecideAcceptees;
    for seed in 0..100 {
        let sim=View::new(&g,0).determinize(&mut Rng::new(seed));
        for a in g.legal_actions() { let mut s=sim.clone();s.apply(a); for r in 0..5 {assert_eq!(s.bank[r]+s.players.iter().map(|p|p.hand[r]).sum::<u8>(),19);} }
        assert!(sim.players[1].hand[0]>=2 && sim.players[1].hand[1]>=2);
        assert!(sim.players[2].hand[2]>=1);
    }
    let _=Action::EndTurn;
}
