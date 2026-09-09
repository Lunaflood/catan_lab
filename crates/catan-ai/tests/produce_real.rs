//! 「サイコロの目が出たのに資源が貰えない」の追跡。
//!
//! 乱数で適当に打つ対局ではなく、**実際に採用している CPU** に打たせる。
//! こちらの方が手札を溜め込むので、銀行の在庫切れが現実的な頻度で起きる。

use catan_ai::agent_v2::{AgentV2Bot, V2Config};
use catan_ai::bots::Bot;
use catan_core::action::{Action, Outcome};
use catan_core::board::{BuildingKind, NUM_RESOURCES};
use catan_core::game::{Game, GameConfig};
use catan_core::topology::{Topology, NUM_TILES};
use catan_core::view::View;

fn demand(g: &Game, roll: u8) -> [[u8; NUM_RESOURCES]; 4] {
    let topo = Topology::get();
    let mut want = [[0u8; NUM_RESOURCES]; 4];
    for t in 0..NUM_TILES {
        if g.board.tile_number[t] != roll || t as u8 == g.board.robber { continue; }
        let Some(res) = g.board.tile_resource[t] else { continue };
        for n in topo.tile_nodes[t] {
            if let Some(b) = g.board.building[n as usize] {
                let amt = match b.kind { BuildingKind::Settlement => 1, BuildingKind::City => 2 };
                want[b.owner as usize][res.idx()] += amt;
            }
        }
    }
    want
}

#[test]
fn 実戦の進行でも産出は規則どおり() {
    let cfg = GameConfig::default();
    let mut bots: Vec<Box<dyn Bot>> = (0..4)
        .map(|i| Box::new(AgentV2Bot::new(100 + i, V2Config::default(), "v2")) as Box<dyn Bot>)
        .collect();

    let (mut rolls, mut shorted, mut partial, mut shorted_games) = (0usize, 0usize, 0usize, 0usize);
    let mut min_bank = [19u8; NUM_RESOURCES];
    let games = 60;
    for gi in 0..games {
        let seed = 500_000 + gi as u64;
        for (b, bot) in bots.iter_mut().enumerate() { bot.reset(seed ^ ((b as u64 + 1) << 32)); }
        let mut g = Game::with_config(4, seed, cfg);
        let mut buf = Vec::new();
        let mut hit = false;
        for step in 0..3000 {
            if g.is_over() { break; }
            g.legal_actions_into(&mut buf);
            if buf.is_empty() { break; }
            let seat = g.to_act as usize;
            let a = bots[seat].decide(&View::new(&g, seat as u8), &buf);
            let is_roll = matches!(a, Action::Roll);
            let before_hands: Vec<_> = (0..g.n()).map(|q| g.players[q].hand).collect();
            let before_bank = g.bank;
            let before = g.clone();
            let pre = if is_roll { Some(before.clone()) } else { None };
            let rec = g.apply(a);
            catan_ai::harness::notify_bots(&mut bots, &[0, 1, 2, 3], &before, &g, &rec, step + 1);

            for r in 0..NUM_RESOURCES {
                let held: u32 = (0..g.n()).map(|q| g.players[q].hand[r] as u32).sum();
                assert_eq!(held + g.bank[r] as u32, 19, "資源 {r} の総数が合わない");
                min_bank[r] = min_bank[r].min(g.bank[r]);
            }

            let (Some(pre), Outcome::Dice(d1, d2)) = (pre, rec.result) else { continue };
            let sum = d1 + d2;
            if sum == 7 { continue; }
            rolls += 1;
            let want = demand(&pre, sum);
            for r in 0..NUM_RESOURCES {
                let total: u16 = (0..g.n()).map(|q| want[q][r] as u16).sum();
                if total == 0 { continue; }
                let claimants = (0..g.n()).filter(|&q| want[q][r] > 0).count();
                let payout: Vec<u8> = if total <= before_bank[r] as u16 {
                    (0..g.n()).map(|q| want[q][r]).collect()
                } else if claimants == 1 {
                    partial += 1; hit = true;
                    (0..g.n()).map(|q| if want[q][r] > 0 { before_bank[r] } else { 0 }).collect()
                } else {
                    shorted += 1; hit = true;
                    vec![0; g.n()]
                };
                for q in 0..g.n() {
                    let got = g.players[q].hand[r] as i32 - before_hands[q][r] as i32;
                    assert_eq!(got, payout[q] as i32,
                        "目 {sum} 資源 {r} 席 {q}: {got} 枚（規則では {}）銀行 {}",
                        payout[q], before_bank[r]);
                }
            }
        }
        if hit { shorted_games += 1; }
    }
    println!(
        "対局 {games} / 目 {rolls} 回\n\
         在庫切れで全員ゼロ {shorted} 回 / 端数だけ {partial} 回\n\
         起きた対局 {shorted_games} / {games}（{:.0}%）\n\
         銀行の最小在庫 {:?}",
        100.0 * shorted_games as f64 / games as f64, min_bank
    );
}
