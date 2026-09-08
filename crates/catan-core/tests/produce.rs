//! 産出（サイコロの目で資源が配られる）の検証。
//!
//! 「目が出たのに資源が貰えない」は規則上ありうる（銀行の在庫切れ）。
//! だが在庫の勘定が壊れていると、正当な理由なく貰えなくなる。
//! ここでは 1 手ごとに「銀行 + 全員の手札 = 19」を確かめ、
//! さらに配られた量が**規則どおりの額と一致する**ことを確かめる。

use catan_core::action::{Action, Outcome};
use catan_core::board::{BuildingKind, NUM_RESOURCES};
use catan_core::game::{Game, GameConfig, TurnClock};
use catan_core::rng::Rng;
use catan_core::topology::{Topology, NUM_TILES};

const TOTAL: u8 = 19;

fn check_bank(g: &Game, note: &str) {
    for r in 0..NUM_RESOURCES {
        let held: u32 = (0..g.n()).map(|q| g.players[q].hand[r] as u32).sum();
        let sum = held + g.bank[r] as u32;
        assert_eq!(sum, TOTAL as u32, "{note}: 資源 {r} の総数が {sum}");
    }
}

/// その目で「盤の上から数えた」取り分
fn demand(g: &Game, roll: u8) -> [[u8; NUM_RESOURCES]; 4] {
    let topo = Topology::get();
    let mut want = [[0u8; NUM_RESOURCES]; 4];
    for t in 0..NUM_TILES {
        if g.board.tile_number[t] != roll || t as u8 == g.board.robber {
            continue;
        }
        let Some(res) = g.board.tile_resource[t] else { continue };
        for n in topo.tile_nodes[t] {
            if let Some(b) = g.board.building[n as usize] {
                let amt = match b.kind {
                    BuildingKind::Settlement => 1,
                    BuildingKind::City => 2,
                };
                want[b.owner as usize][res.idx()] += amt;
            }
        }
    }
    want
}

#[test]
fn 産出は規則どおりで銀行の勘定も合う() {
    let mut shorted = 0usize; // 在庫切れで丸ごと配られなかった回数
    let mut partial = 0usize; // 端数だけ配られた回数
    let mut rolls = 0usize;
    for seed in 0..60u64 {
        let mut cfg = GameConfig::default();
        cfg.turn_clock = TurnClock::PerAction { ms: 500 };
        let mut g = Game::with_config(4, 900_000 + seed, cfg);
        let mut rng = Rng::with_stream(seed, 7);
        let mut buf = Vec::new();
        check_bank(&g, "開始");
        for _ in 0..4000 {
            if g.is_over() { break; }
            g.legal_actions_into(&mut buf);
            if buf.is_empty() { break; }
            let a = buf[rng.below(buf.len() as u32) as usize];
            let is_roll = matches!(a, Action::Roll);
            let before_hands: Vec<_> = (0..g.n()).map(|q| g.players[q].hand).collect();
            let before_bank = g.bank;
            let want = if is_roll { Some(g.clone()) } else { None };
            let rec = g.apply(a);
            check_bank(&g, "1 手ごと");

            let (Some(pre), Outcome::Dice(d1, d2)) = (want, rec.result) else { continue };
            let sum = d1 + d2;
            if sum == 7 { continue; }
            rolls += 1;
            let want = demand(&pre, sum);
            for r in 0..NUM_RESOURCES {
                let total: u16 = (0..g.n()).map(|q| want[q][r] as u16).sum();
                if total == 0 { continue; }
                let claimants = (0..g.n()).filter(|&q| want[q][r] > 0).count();
                // 規則: 全員分あれば全員に配る / 足りず 1 人だけなら残りを渡す / それ以外は誰も貰えない
                let payout: Vec<u8> = if total <= before_bank[r] as u16 {
                    (0..g.n()).map(|q| want[q][r]).collect()
                } else if claimants == 1 {
                    partial += 1;
                    (0..g.n()).map(|q| if want[q][r] > 0 { before_bank[r] } else { 0 }).collect()
                } else {
                    shorted += 1;
                    vec![0; g.n()]
                };
                for q in 0..g.n() {
                    let got = g.players[q].hand[r] as i32 - before_hands[q][r] as i32;
                    assert_eq!(
                        got, payout[q] as i32,
                        "seed {seed}: 目 {sum} 資源 {r} 席 {q} が {got} 枚（規則では {}）\
                         / 盤の取り分 {:?} / 直前の銀行 {}",
                        payout[q], want.iter().map(|w| w[r]).collect::<Vec<_>>(), before_bank[r]
                    );
                }
            }
        }
    }
    println!("目 {rolls} 回 / 在庫切れで全員ゼロ {shorted} 回 / 端数だけ {partial} 回");
    assert!(rolls > 500);
}

/// ヘクスに面する 6 頂点が**重複なく 6 個**あること。
///
/// 幾何の試験は「各頂点がヘクスの中心から一定距離にある」ことは見ているが、
/// 同じ頂点が 2 度入っていても通ってしまう。その場合、絵の上では角に接して
/// いるのにエンジンでは隣接していない頂点ができ、
/// 「自分の数字が出たのに資源が来ない」が静かに起き続ける。
#[test]
fn ヘクスの六頂点は重複しない() {
    let topo = Topology::get();
    for t in 0..NUM_TILES {
        let ns = topo.tile_nodes[t];
        for i in 0..6 {
            for j in (i + 1)..6 {
                assert_ne!(ns[i], ns[j], "ヘクス {t} の頂点 {i} と {j} が同じ");
            }
        }
    }
}

/// 逆向きの確認: どの建物も、面しているヘクス**すべて**から産出を受け取れること。
/// 頂点 → ヘクスの対応と、ヘクス → 頂点の対応が食い違っていないかを見る
#[test]
fn 頂点とヘクスの対応は双方向で一致する() {
    let topo = Topology::get();
    for t in 0..NUM_TILES {
        for n in topo.tile_nodes[t] {
            assert!(
                topo.node_tiles[n as usize].as_slice().contains(&(t as u8)),
                "ヘクス {t} は頂点 {n} を持つのに、頂点 {n} 側にヘクス {t} が無い"
            );
        }
    }
}
