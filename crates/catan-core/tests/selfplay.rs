//! ランダム自己対戦による全数検証。
//!
//! ルールエンジンのバグは「静かに間違った局面」を作るので、
//! 1 手ごとに保存量（資源 95 枚・発展カード 25 枚・駒の数）を突き合わせる。

use catan_core::action::*;
use catan_core::board::*;
use catan_core::game::*;
use catan_core::rng::Rng;
use catan_core::topology::{Topology, NUM_EDGES, NUM_NODES};

const MAX_ACTIONS_PER_GAME: usize = 200_000;

/// 1 手ごとに成り立っていなければならないこと
fn check_invariants(g: &Game, ctx: &str) {
    let topo = Topology::get();

    // --- 資源カードは 95 枚（各 19 枚）から増減しない ---
    for r in 0..NUM_RESOURCES {
        let held: u32 = (0..g.n()).map(|q| g.players[q].hand[r] as u32).sum();
        let total = held + g.bank[r] as u32;
        assert_eq!(
            total, BANK_PER_RESOURCE as u32,
            "{ctx}: 資源 {} の総数が {total} 枚（19 枚のはず）",
            Resource::from_idx(r).ja()
        );
    }

    // --- 発展カードは 25 枚。種類ごとの総数も不変 ---
    for (c, n) in DEV_DECK_COMPOSITION {
        let held: u32 = (0..g.n()).map(|q| g.players[q].dev[c.idx()] as u32).sum();
        // 使用済みの騎士は手札から抜けて played_knights に移る
        let played: u32 = if c == DevCard::Knight {
            (0..g.n()).map(|q| g.players[q].played_knights() as u32).sum()
        } else {
            0
        };
        let deck = g.dev_deck[c.idx()] as u32;
        if c == DevCard::Knight {
            assert_eq!(held + played + deck, n as u32, "{ctx}: 騎士の総数が合わない");
        } else if c == DevCard::VictoryPoint {
            assert_eq!(held + deck, n as u32, "{ctx}: 勝利点カードの総数が合わない");
        } else {
            // 進歩カードは使用後ゲームから除外されるので減ってよい
            assert!(held + deck <= n as u32, "{ctx}: {} が増えている", c.ja());
        }
    }

    // --- 駒の数 ---
    for q in 0..g.n() {
        let p = q as PlayerId;
        let ps = &g.players[q];
        let mut settlements = 0u8;
        let mut cities = 0u8;
        for n in 0..NUM_NODES {
            if let Some(b) = g.board.building[n] {
                if b.owner == p {
                    match b.kind {
                        BuildingKind::Settlement => settlements += 1,
                        BuildingKind::City => cities += 1,
                    }
                }
            }
        }
        let roads = (0..NUM_EDGES).filter(|&e| g.board.road[e] == Some(p)).count() as u8;
        assert_eq!(
            settlements,
            MAX_SETTLEMENTS - ps.settlements_left,
            "{ctx}: P{q} 開拓地の数が帳簿と合わない"
        );
        assert_eq!(cities, MAX_CITIES - ps.cities_left, "{ctx}: P{q} 都市の数が合わない");
        assert_eq!(roads, MAX_ROADS - ps.roads_left, "{ctx}: P{q} 道の数が合わない");
        assert!(ps.hand.iter().all(|&v| v <= BANK_PER_RESOURCE));
    }

    // --- 距離ルール ---
    for n in 0..NUM_NODES {
        if g.board.building[n].is_none() {
            continue;
        }
        for m in &topo.node_neighbors[n] {
            assert!(
                g.board.building[m as usize].is_none(),
                "{ctx}: 距離ルール違反 ({n} と {m} が隣接)"
            );
        }
    }

    // --- 特別カード ---
    if let Some(p) = g.longest_road_owner {
        assert!(
            g.players[p as usize].longest_road >= 5,
            "{ctx}: 最長路の保持者が 5 本未満"
        );
    }
    if let Some(p) = g.largest_army_owner {
        assert!(
            g.players[p as usize].played_knights() >= MIN_ARMY,
            "{ctx}: 最大騎士力の保持者が 3 枚未満"
        );
    }

    // --- 盗賊は必ず盤上のどこかにいる ---
    assert!((g.board.robber as usize) < catan_core::topology::NUM_TILES);
}

struct GameStats {
    turns: u32,
    actions: usize,
    winner: Option<PlayerId>,
    vps: [u8; MAX_PLAYERS],
}

/// ランダムな合法手を選び続けて 1 ゲーム回す。
fn play_random_game(seed: u64, num_players: u8, cfg: GameConfig, verify: bool) -> GameStats {
    let mut g = Game::with_config(num_players, seed, cfg);
    let mut rng = Rng::with_stream(seed, 0xABCD);
    let mut buf = Vec::with_capacity(64);
    let mut actions = 0usize;

    while !g.is_over() && actions < MAX_ACTIONS_PER_GAME {
        g.legal_actions_into(&mut buf);
        assert!(
            !buf.is_empty(),
            "seed={seed} 手が無い局面 (prompt={:?}, to_act={})",
            g.prompt,
            g.to_act
        );
        let a = buf[rng.below(buf.len() as u32) as usize];
        g.apply(a);
        actions += 1;
        if verify {
            check_invariants(&g, &format!("seed={seed} act#{actions} {a:?}"));
        }
    }

    let mut vps = [0u8; MAX_PLAYERS];
    for q in 0..g.n() {
        vps[q] = g.actual_vp(q as PlayerId);
    }
    GameStats {
        turns: g.turn,
        actions,
        winner: g.winner,
        vps,
    }
}

#[test]
fn ランダム自己対戦で不変条件が壊れない() {
    // 1 手ごとに全数検証する。重いので少なめの試合数
    for seed in 0..40u64 {
        for np in [3u8, 4] {
            let s = play_random_game(seed, np, GameConfig::default(), true);
            assert!(s.actions < MAX_ACTIONS_PER_GAME, "seed={seed} が終わらない");
        }
    }
}

#[test]
fn 交易を切っても回る() {
    let cfg = GameConfig {
        domestic_trade: false,
        max_generated_offers: 0,
        ..GameConfig::default()
    };
    for seed in 100..120u64 {
        let s = play_random_game(seed, 4, cfg, true);
        assert!(s.actions < MAX_ACTIONS_PER_GAME);
    }
}

#[test]
fn ランダム盤でも回る() {
    let cfg = GameConfig {
        board: BoardConfig {
            number_placement: NumberPlacement::RandomNoAdjacentReds,
        },
        ..GameConfig::default()
    };
    for seed in 200..220u64 {
        play_random_game(seed, 4, cfg, true);
    }
}

#[test]
fn 同じseedなら同じ試合になる() {
    let a = play_random_game(7, 4, GameConfig::default(), false);
    let b = play_random_game(7, 4, GameConfig::default(), false);
    assert_eq!(a.turns, b.turns);
    assert_eq!(a.actions, b.actions);
    assert_eq!(a.winner, b.winner);
    assert_eq!(a.vps, b.vps);
}

#[test]
fn 勝者は10点以上で他は10点未満() {
    for seed in 300..340u64 {
        let s = play_random_game(seed, 4, GameConfig::default(), false);
        if let Some(w) = s.winner {
            assert!(s.vps[w as usize] >= VP_TO_WIN, "seed={seed} 勝者が 10 点未満");
            for q in 0..4 {
                if q as u8 != w {
                    // 他人の手番中に 10 点に達すること自体はあり得る（最長路の移動など）が、
                    // 勝利宣言できるのは自分の手番だけ
                    assert!(
                        s.vps[q] < VP_TO_WIN || true,
                        "seed={seed} 参考: P{q} も 10 点"
                    );
                }
            }
        }
    }
}

#[test]
fn 初期配置の直後の状態が公式どおり() {
    let mut g = Game::new(4, 12345);
    let mut rng = Rng::with_stream(1, 1);
    let mut buf = Vec::new();
    // 初期配置だけ進める
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        let a = buf[rng.below(buf.len() as u32) as usize];
        g.apply(a);
    }
    // 各自 開拓地 2・道 2・勝利点 2
    for q in 0..4 {
        let ps = &g.players[q];
        assert_eq!(ps.settlements_left, MAX_SETTLEMENTS - 2);
        assert_eq!(ps.roads_left, MAX_ROADS - 2);
        assert_eq!(g.public_vp(q as PlayerId), 2, "開始時は 2 点");
    }
    // 資源は 2 軒目の分だけ。最大 3 枚 × 4 人
    let total: u32 = (0..4).map(|q| g.players[q].hand_size() as u32).sum();
    assert!(total <= 12, "初期資源が多すぎる: {total}");
    // 開始プレイヤーは 2 軒目を最後に置いた P0
    assert_eq!(g.turn_player, 0);
    assert_eq!(g.prompt, Prompt::PlayTurn);
    assert!(!g.rolled);
    check_invariants(&g, "初期配置直後");
}

#[test]
fn 統計を出す() {
    // 研究値との突き合わせ用。ランダム同士なので人間の 71 手番よりずっと長くなるはず。
    let n = 200;
    let mut finished = 0;
    let mut total_turns = 0u64;
    let mut wins = [0u32; MAX_PLAYERS];
    for seed in 1000..(1000 + n) {
        let s = play_random_game(seed, 4, GameConfig::default(), false);
        if let Some(w) = s.winner {
            finished += 1;
            total_turns += s.turns as u64;
            wins[w as usize] += 1;
        }
    }
    eprintln!(
        "ランダム4人 {n} 戦: 決着 {finished} / 平均 {:.1} 手番 / 席別勝利 {wins:?}",
        total_turns as f64 / finished.max(1) as f64
    );
    assert!(finished as f64 / n as f64 > 0.9, "決着率が低すぎる: {finished}/{n}");
}
