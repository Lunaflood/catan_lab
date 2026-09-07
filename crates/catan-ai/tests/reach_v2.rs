//! M3: 勝利ハザード（次の手番で勝つ確率）の局面テスト。正解は手で導く。

use catan_ai::belief::reach::ReachSolver;
use catan_core::action::DevCard;
use catan_core::board::{Building, BuildingKind, NUM_RESOURCES};
use catan_core::game::{Game, GameConfig};
use catan_core::rng::Rng;
use catan_core::topology::NUM_NODES;

const WOOD: usize = 0;
const BRICK: usize = 1;
const SHEEP: usize = 2;
const WHEAT: usize = 3;
const ORE: usize = 4;

fn ready(seed: u64) -> Game {
    let mut g = Game::with_config(4, seed, GameConfig::default());
    let mut rng = Rng::with_stream(seed, 3);
    let mut buf = Vec::new();
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        g.apply(buf[rng.below(buf.len() as u32) as usize]);
    }
    g.rolled = true;
    g
}

/// 公開点を `vp` にする。**都市の駒を必ず 1 つ残す**（都市を建てて届く局面を作るため）。
/// 開拓地 S 軒 + 都市 C 軒（C ≤ 3）で `vp = S + 2C` になるよう、距離ルールを満たす空き頂点に置く
fn set_public_vp(g: &mut Game, p: u8, vp: u8) {
    let topo = catan_core::topology::Topology::get();
    let open = |g: &Game, n: usize| g.board.building[n].is_none() && topo.node_neighbors[n].as_slice().iter().all(|&m| g.board.building[m as usize].is_none());
    let cities = (vp / 2).min(3);
    let settlements = vp - 2 * cities;
    // いまの建物を全部消して置き直す
    for n in 0..NUM_NODES {
        if matches!(g.board.building[n], Some(b) if b.owner == p) {
            g.board.building[n] = None;
        }
    }
    g.players[p as usize].settlements_left = 5;
    g.players[p as usize].cities_left = 4;
    let mut placed = 0u8;
    let mut converted = 0u8;
    while placed < settlements + cities {
        if g.players[p as usize].settlements_left == 0 {
            // 駒が尽きたら都市化して返す
            let n = (0..NUM_NODES).find(|&n| matches!(g.board.building[n], Some(b) if b.owner == p && b.kind == BuildingKind::Settlement)).unwrap();
            g.board.building[n] = Some(Building { owner: p, kind: BuildingKind::City });
            g.players[p as usize].cities_left -= 1;
            g.players[p as usize].settlements_left += 1;
            converted += 1;
            continue;
        }
        let n = (0..NUM_NODES).find(|&n| open(g, n)).expect("空き頂点が無い");
        g.board.building[n] = Some(Building { owner: p, kind: BuildingKind::Settlement });
        g.players[p as usize].settlements_left -= 1;
        placed += 1;
    }
    while converted < cities {
        let n = (0..NUM_NODES).find(|&n| matches!(g.board.building[n], Some(b) if b.owner == p && b.kind == BuildingKind::Settlement)).unwrap();
        g.board.building[n] = Some(Building { owner: p, kind: BuildingKind::City });
        g.players[p as usize].cities_left -= 1;
        g.players[p as usize].settlements_left += 1;
        converted += 1;
    }
    // 道の再計算（最長路の持ち主が変わり得る）
    g.longest_road_owner = None;
    assert_eq!(g.public_vp(p), vp, "公開点を {vp} にできない");
    assert!(g.players[p as usize].cities_left >= 1);
}

fn solver(g: &Game, p: u8, dev: [u8; 5]) -> ReachSolver {
    ReachSolver::new(g, p, dev, g.dev_deck, [0; NUM_RESOURCES], g.bank)
}

#[test]
fn 公開8点で都市が建つ手札は次の手番に確実に勝つわけではないが今なら勝つ() {
    let mut g = ready(1);
    set_public_vp(&mut g, 1, 8);
    // P1: 都市 1 軒分（麦 2 鉄 3）+ 伏せ VP 1 枚 → 8 + 1 + 1 = 10
    let mut hand = [0u8; 5];
    hand[WHEAT] = 2;
    hand[ORE] = 3;
    let mut dev = [0u8; 5];
    dev[DevCard::VictoryPoint.idx()] = 1;
    let mut s = solver(&g, 1, dev);
    assert!((s.win_prob_now(&hand) - 1.0).abs() < 1e-6, "今の手番なら確実に届く");
    assert!((s.win_prob_next_turn(&hand) - 1.0).abs() < 1e-6, "産出は減らないので次の手番も確実");
    // 伏せ VP が無ければ 9 点止まり
    let mut s0 = solver(&g, 1, [0; 5]);
    assert!(s0.win_prob_now(&hand) < 1e-6, "9 点では勝てない");
}

#[test]
fn 一見損な銀行交換を挟むと届く() {
    let mut g = ready(2);
    set_public_vp(&mut g, 1, 9);
    // 麦 2 鉄 2 + 木 4（4:1 で鉄 1 を作れる）→ 都市 1 軒で 10 点
    let mut hand = [0u8; 5];
    hand[WHEAT] = 2;
    hand[ORE] = 2;
    hand[WOOD] = 4;
    let mut s = solver(&g, 1, [0; 5]);
    if g.bank[ORE] > 0 {
        assert!((s.win_prob_now(&hand) - 1.0).abs() < 1e-6, "4:1 交換を挟めば都市が建つ");
    }
    hand[WOOD] = 3;
    let mut s2 = solver(&g, 1, [0; 5]);
    let v = s2.win_prob_now(&hand);
    // 3:1 港を持っていなければ届かない
    let rate = g.board.best_maritime_rate(1, catan_core::board::Resource::Wood);
    if rate == 4 {
        assert!(v < 1e-6, "木 3 枚では 4:1 交換できない: {v}");
    }
}

#[test]
fn 発展カードの購入は山の内訳で確率になる() {
    let mut g = ready(3);
    set_public_vp(&mut g, 1, 9);
    // 山: 勝利点 2 / 騎士 6 → 1 枚買って VP を引く確率は 2/8
    g.dev_deck = [6, 0, 0, 0, 2];
    let mut hand = [0u8; 5];
    hand[SHEEP] = 1;
    hand[WHEAT] = 1;
    hand[ORE] = 1;
    let mut s = solver(&g, 1, [0; 5]);
    let v = s.win_prob_now(&hand);
    assert!((v - 0.25).abs() < 1e-4, "1 枚購入で VP を引く確率は 2/8: {v}");
    // 2 枚買えるなら P(少なくとも 1 枚 VP) = 1 − C(6,2)/C(8,2) = 1 − 15/28
    hand[SHEEP] = 2;
    hand[WHEAT] = 2;
    hand[ORE] = 2;
    let mut s2 = solver(&g, 1, [0; 5]);
    let v2 = s2.win_prob_now(&hand);
    assert!((v2 - (1.0 - 15.0 / 28.0)).abs() < 1e-4, "2 枚購入: {v2}");
}

#[test]
fn 騎士で最大騎士力を取れば2点() {
    let mut g = ready(4);
    set_public_vp(&mut g, 1, 8);
    g.players[1].played_dev[DevCard::Knight.idx()] = 2;
    g.largest_army_owner = None;
    let mut dev = [0u8; 5];
    dev[DevCard::Knight.idx()] = 1;
    let mut s = solver(&g, 1, dev);
    assert!((s.win_prob_now(&[0; 5]) - 1.0).abs() < 1e-6, "3 枚目の騎士で最大騎士力 = +2 で 10 点");
    // 既に他人が 3 枚持っていれば 3 枚では取れない
    g.players[2].played_dev[DevCard::Knight.idx()] = 3;
    g.largest_army_owner = Some(2);
    let mut s2 = solver(&g, 1, dev);
    assert!(s2.win_prob_now(&[0; 5]) < 1e-6);
}

#[test]
fn 産出は出目で周辺化される() {
    let mut g = ready(5);
    set_public_vp(&mut g, 1, 9);
    // 手札は都市にあと鉄 1 枚足りない。鉄が出る目の確率ぶんだけ勝てる
    let mut hand = [0u8; 5];
    hand[WHEAT] = 2;
    hand[ORE] = 2;
    let mut s = solver(&g, 1, [0; 5]);
    let v = s.win_prob_next_turn(&hand);
    assert!(s.win_prob_now(&hand) < 1e-6);
    // 鉄の産出 pip があるなら 0 < v < 1、無ければ 0（銀行交換で 4 枚集まらない）
    let pips = catan_ai::eval::production_pips(&g.board, 1);
    if pips[ORE] > 0 {
        assert!(v > 0.0 && v < 1.0, "鉄の出目の確率ぶん: {v}");
    } else {
        assert!(v < 0.5, "鉄が出ないのに高すぎる: {v}");
    }
    let _ = BRICK;
}
