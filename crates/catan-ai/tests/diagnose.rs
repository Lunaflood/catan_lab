//! 1 局を細かく観察する診断。勝率ではなく「何が起きているか」を見る。
//!
//! 4 体とも思考ボットにすると対局が終わらなくなる件を切り分けるために書いた。

use catan_ai::bots::{Bot, GreedyEvalBot};
use catan_core::action::{Action, Prompt};
use catan_core::game::{Game, GameConfig};
use catan_core::view::View;
use std::collections::BTreeMap;

fn label(a: &Action) -> &'static str {
    match a {
        Action::SetupSettlement(_) => "SetupSettlement",
        Action::SetupRoad(_) => "SetupRoad",
        Action::Roll => "Roll",
        Action::EndTurn => "EndTurn",
        Action::BuildRoad(_) => "BuildRoad",
        Action::BuildSettlement(_) => "BuildSettlement",
        Action::BuildCity(_) => "BuildCity",
        Action::BuyDevCard => "BuyDevCard",
        Action::PlayKnight => "PlayKnight",
        Action::PlayRoadBuilding => "PlayRoadBuilding",
        Action::PlayYearOfPlenty(..) => "PlayYearOfPlenty",
        Action::PlayMonopoly(_) => "PlayMonopoly",
        Action::Discard(_) => "Discard",
        Action::MoveRobber { .. } => "MoveRobber",
        Action::MaritimeTrade { .. } => "MaritimeTrade",
        Action::OfferTrade { .. } => "OfferTrade",
        Action::AcceptTrade => "AcceptTrade",
        Action::RejectTrade => "RejectTrade",
        Action::CounterAlt { .. } => "CounterAlt",
        Action::CounterAltRemove(_) => "CounterAltRemove",
        Action::CounterOffer { .. } => "CounterOffer",
        Action::ConfirmTrade(_) => "ConfirmTrade",
        Action::AcceptCounter { .. } => "AcceptCounter",
        Action::CancelTrade => "CancelTrade",
    }
}

#[test]
fn 一局の中身を数える() {
    let cap = 60_000usize;
    for seed in [1u64, 2, 3] {
        let mut g = Game::with_config(4, seed, GameConfig::default());
        let mut bots: Vec<Box<dyn Bot>> = (0..4)
            .map(|i| Box::new(GreedyEvalBot::new(seed * 10 + i)) as Box<dyn Bot>)
            .collect();
        let mut buf = Vec::new();
        let mut hist: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut legal_total = 0usize;
        let mut steps = 0usize;
        let t0 = std::time::Instant::now();

        while !g.is_over() && steps < cap {
            g.legal_actions_into(&mut buf);
            legal_total += buf.len();
            let seat = g.to_act;
            let v = View::new(&g, seat);
            let a = bots[seat as usize].decide(&v, &buf);
            *hist.entry(label(&a)).or_default() += 1;
            g.apply(a);
            steps += 1;
        }

        let dt = t0.elapsed();
        eprintln!(
            "\n=== seed {seed}: {} 手番 / {steps} 行動 / {:?} / 決着 {} ===",
            g.turn,
            dt,
            g.winner.is_some()
        );
        eprintln!(
            "  1 行動あたりの合法手 {:.1} / 1 手番あたりの行動 {:.1}",
            legal_total as f64 / steps as f64,
            steps as f64 / g.turn.max(1) as f64
        );
        let mut rows: Vec<_> = hist.iter().collect();
        rows.sort_by_key(|(_, &c)| std::cmp::Reverse(c));
        for (k, c) in rows.iter().take(10) {
            eprintln!("  {k:<18} {c}");
        }
        // 勝利点の内訳
        for q in 0..4u8 {
            eprintln!(
                "  P{q}: VP {} / 開拓地残 {} / 都市残 {} / 道残 {}",
                g.actual_vp(q),
                g.players[q as usize].settlements_left,
                g.players[q as usize].cities_left,
                g.players[q as usize].roads_left
            );
        }
        let _ = Prompt::PlayTurn;
    }
}
