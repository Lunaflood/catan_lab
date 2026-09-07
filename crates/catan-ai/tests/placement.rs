//! 初期配置の評価関数が「何をしているか」を固定する振る舞いテスト。
//!
//! 勝率はハーネスで測る。ここで見るのは、勝率の裏で実際に何が起きているか
//! （産出量・資源の種類・海に面した死に頂点を掴んでいないか）。

use catan_ai::bots::{Bot, PlacementBot, RandomBot};
use catan_ai::placement::{my_production, PlacementWeights};
use catan_core::action::{Action, Prompt};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::{Game, GameConfig};
use catan_core::topology::{NodeId, Topology, NUM_NODES};
use catan_core::view::View;

/// 4 人全員を同じボットで初期配置だけ進める。
fn run_setup(seed: u64, make: &dyn Fn(u64) -> Box<dyn Bot>) -> Game {
    let mut g = Game::with_config(4, seed, GameConfig::default());
    let mut bots: Vec<Box<dyn Bot>> = (0..4).map(|i| make(seed * 10 + i)).collect();
    let mut buf = Vec::new();
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        let seat = g.to_act;
        let v = View::new(&g, seat);
        let a = bots[seat as usize].decide(&v, &buf);
        g.apply(a);
    }
    g
}

/// プレイヤーの初期 2 軒がもたらす「総 pip」と「資源の種類数」
fn quality(g: &Game, p: PlayerId) -> (u32, usize) {
    let prod = my_production(&g.board, p);
    let pips: u32 = prod.iter().map(|&v| v as u32).sum();
    let kinds = prod.iter().filter(|&&v| v > 0).count();
    (pips, kinds)
}

#[test]
fn 配置ボットはランダムより産出も種類も明確に上() {
    let n = 200u64;
    let (mut pl_pips, mut pl_kinds) = (0u32, 0usize);
    let (mut rd_pips, mut rd_kinds) = (0u32, 0usize);

    for seed in 0..n {
        let g = run_setup(seed, &|s| Box::new(PlacementBot::new(s)));
        for p in 0..4u8 {
            let (a, b) = quality(&g, p);
            pl_pips += a;
            pl_kinds += b;
        }
        let g = run_setup(seed, &|s| Box::new(RandomBot::new(s)));
        for p in 0..4u8 {
            let (a, b) = quality(&g, p);
            rd_pips += a;
            rd_kinds += b;
        }
    }

    let m = (n * 4) as f64;
    let (pl_pips, pl_kinds) = (pl_pips as f64 / m, pl_kinds as f64 / m);
    let (rd_pips, rd_kinds) = (rd_pips as f64 / m, rd_kinds as f64 / m);
    eprintln!("配置ボット: {pl_pips:.1} pip / {pl_kinds:.2} 種");
    eprintln!("ランダム  : {rd_pips:.1} pip / {rd_kinds:.2} 種");

    assert!(
        pl_pips > rd_pips * 1.25,
        "産出で差が出ていない: {pl_pips:.1} vs {rd_pips:.1}"
    );
    assert!(
        pl_kinds > rd_kinds + 0.3,
        "資源の種類で差が出ていない: {pl_kinds:.2} vs {rd_kinds:.2}"
    );
    // 初期 2 軒で 4 種以上を押さえるのが標準的な良い配置
    assert!(pl_kinds >= 4.0, "種類が少なすぎる: {pl_kinds:.2}");
}

#[test]
fn 海に面した1ヘクスだけの頂点は掴まない() {
    let topo = Topology::get();
    for seed in 0..100u64 {
        let g = run_setup(seed, &|s| Box::new(PlacementBot::new(s)));
        for n in 0..NUM_NODES {
            let Some(b) = g.board.building[n] else { continue };
            let tiles = topo.node_tiles[n].len();
            assert!(
                tiles >= 2,
                "seed={seed}: P{} が 1 ヘクスしか接しない頂点 {n} を掴んだ",
                b.owner
            );
        }
    }
}

#[test]
fn 砂漠だけの死に頂点を選ばない() {
    for seed in 0..100u64 {
        let g = run_setup(seed, &|s| Box::new(PlacementBot::new(s)));
        for p in 0..4u8 {
            let (pips, _) = quality(&g, p);
            assert!(pips > 0, "seed={seed}: P{p} の初期産出がゼロ");
        }
    }
}

#[test]
fn 二軒目は一軒目と資源が被りにくい() {
    // 2 軒目は「まだ持っていない資源」を足す方が高く出る設計になっている。
    // 4 人分・100 盤で、2 軒とも同じ 1 種類しか産まない配置が出ないことを見る。
    for seed in 0..100u64 {
        let g = run_setup(seed, &|s| Box::new(PlacementBot::new(s)));
        for p in 0..4u8 {
            let (_, kinds) = quality(&g, p);
            assert!(kinds >= 3, "seed={seed}: P{p} の初期資源が {kinds} 種しかない");
        }
    }
}

#[test]
fn 手で決めた重みより実測で残した重みの方が産出が高い() {
    // ablation で落とした項目（港・ETB・数字の散らし）は、
    // 産出そのものを削ってまで取りに行く価値が無かった、という関係を固定する。
    let n = 200u64;
    let mut tuned = 0u32;
    let mut hand = 0u32;
    for seed in 0..n {
        let g = run_setup(seed, &|s| Box::new(PlacementBot::new(s)));
        for p in 0..4u8 {
            tuned += quality(&g, p).0;
        }
        let g = run_setup(seed, &|s| {
            Box::new(PlacementBot::with_weights(
                s,
                PlacementWeights::handmade(),
                "handmade",
            ))
        });
        for p in 0..4u8 {
            hand += quality(&g, p).0;
        }
    }
    eprintln!(
        "実測で残した重み: {:.1} pip / 手で決めた重み: {:.1} pip",
        tuned as f64 / (n * 4) as f64,
        hand as f64 / (n * 4) as f64
    );
    assert!(tuned > hand, "実測版の方が産出が低い: {tuned} vs {hand}");
}

#[test]
fn 道は空いている方角へ伸びる() {
    // 初期配置の道は「2 歩先の良い空き頂点」を向く。
    // 行き止まり（先に空き頂点が無い辺）を選んでいないことを確認する。
    let topo = Topology::get();
    for seed in 0..60u64 {
        let mut g = Game::with_config(4, seed, GameConfig::default());
        let mut bot = PlacementBot::new(seed);
        let mut buf = Vec::new();
        while g.is_setup() {
            g.legal_actions_into(&mut buf);
            let seat = g.to_act;
            let v = View::new(&g, seat);
            let a = bot.decide(&v, &buf);

            if g.prompt == Prompt::SetupRoad {
                let Action::SetupRoad(e) = a else { unreachable!() };
                // 選んだ辺の先に、まだ建てられる頂点があるか
                let near = g.last_setup_node;
                let [x, y] = topo.edge_nodes[e as usize];
                let far = if x == near { y } else { x };
                let has_target = topo.node_neighbors[far as usize].into_iter().any(|m| {
                    m != near
                        && g.board.building[m as usize].is_none()
                        && topo.node_neighbors[m as usize]
                            .into_iter()
                            .all(|k| g.board.building[k as usize].is_none())
                });
                // 全候補が行き止まりのこともあるので、その場合だけ許す
                let any_target = buf.iter().any(|act| {
                    let Action::SetupRoad(e2) = act else { return false };
                    let [x, y] = topo.edge_nodes[*e2 as usize];
                    let f2 = if x == near { y } else { x };
                    topo.node_neighbors[f2 as usize].into_iter().any(|m| {
                        m != near
                            && g.board.building[m as usize].is_none()
                            && topo.node_neighbors[m as usize]
                                .into_iter()
                                .all(|k| g.board.building[k as usize].is_none())
                    })
                });
                assert!(
                    has_target || !any_target,
                    "seed={seed}: 空きがあるのに行き止まりへ道を伸ばした"
                );
            }
            g.apply(a);
        }
    }
}

#[test]
fn 評価は決定的() {
    let g = Game::with_config(4, 5, GameConfig::default());
    let v = View::new(&g, 0);
    let w = PlacementWeights::default();
    let nodes: Vec<NodeId> = (0..NUM_NODES as NodeId).collect();
    let a: Vec<f32> = nodes
        .iter()
        .map(|&n| catan_ai::placement::score_settlement(&v, n, &w))
        .collect();
    let b: Vec<f32> = nodes
        .iter()
        .map(|&n| catan_ai::placement::score_settlement(&v, n, &w))
        .collect();
    assert_eq!(a, b);
    let _ = NUM_RESOURCES;
}
