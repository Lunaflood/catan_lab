//! 最長交易路の長さ計算。
//!
//! 公式ルール:
//! - 「連続した道」で 5 本以上。分岐は数えず、最長の 1 本の枝のみ。
//! - 相手の開拓地/都市が道の途中にあると、そこで道が**分断**される。
//!
//! 実装上の解釈（ここは実装ごとにブレる箇所なので明示しておく）:
//! - 同じ**道**を 2 度通らない（= 辺素な小道 / trail）。頂点の再訪は許す。
//!   これにより 6 本の輪は 6 と数えられる（一般的な解釈）。
//! - 相手の建物のある頂点には**入れるが、そこから先へは進めない**。
//!   よって「相手の集落で終わる 5 本の道」は 5 本のまま数える。
//!   （catanatron はこの辺を丸ごと除外していて 1 本短く出る。こちらは公式解釈を採る）
//! - 自分の建物は道を分断しない。

use crate::board::{Board, PlayerId};
use crate::topology::{NodeId, Topology, NUM_EDGES, NUM_NODES};

/// 使用済みの道を表すビット集合（辺は 72 本なので u128 に収まる）
type EdgeMask = u128;

/// `player` の最長交易路の長さ（本数）。
pub fn longest_road_length(board: &Board, player: PlayerId) -> u8 {
    let topo = Topology::get();

    // 自分の道のビット集合と、道が接する頂点
    let mut owned: EdgeMask = 0;
    let mut touched = [false; NUM_NODES];
    let mut any = false;
    for e in 0..NUM_EDGES {
        if board.road[e] == Some(player) {
            owned |= 1u128 << e;
            any = true;
            for n in topo.edge_nodes[e] {
                touched[n as usize] = true;
            }
        }
    }
    if !any {
        return 0;
    }

    // 相手の建物がある頂点 = 通り抜け禁止
    let mut blocked = [false; NUM_NODES];
    for n in 0..NUM_NODES {
        if let Some(b) = board.building[n] {
            blocked[n] = b.owner != player;
        }
    }

    let mut best = 0u8;
    for start in 0..NUM_NODES {
        if !touched[start] {
            continue;
        }
        let mut ctx = Walk {
            topo,
            board,
            player,
            blocked: &blocked,
            owned,
            best: 0,
        };
        ctx.dfs(start as NodeId, 0, 0);
        best = best.max(ctx.best);
    }
    best
}

struct Walk<'a> {
    topo: &'static Topology,
    board: &'a Board,
    player: PlayerId,
    blocked: &'a [bool; NUM_NODES],
    owned: EdgeMask,
    best: u8,
}

impl Walk<'_> {
    fn dfs(&mut self, node: NodeId, used: EdgeMask, len: u8) {
        if len > self.best {
            self.best = len;
        }
        // 相手の建物に「到達した」ならここで打ち切り。ただし出発点としては使える
        // （= 相手の集落で分断された向こう側は、別の道として別途数えられる）。
        if len > 0 && self.blocked[node as usize] {
            return;
        }
        for e in &self.topo.node_edges[node as usize] {
            let bit = 1u128 << e;
            if self.owned & bit == 0 || used & bit != 0 {
                continue;
            }
            debug_assert_eq!(self.board.road[e as usize], Some(self.player));
            let [a, b] = self.topo.edge_nodes[e as usize];
            let next = if a == node { b } else { a };
            self.dfs(next, used | bit, len + 1);
        }
    }
}

/// 全プレイヤーの最長路を測り直し、「最長交易路」カードの持ち主を決める。
///
/// 公式の同点処理:
/// - 保持者が分断されても、まだ同点最長なら保持したまま。
/// - 保持者が最長でなくなり、新たな最長が 2 人以上で同点なら、カードは脇に置かれる（誰も持たない）。
/// - 5 本以上が誰もいなくなった場合も脇に置かれる。
pub fn resolve_longest_road(
    lengths: &[u8],
    current_owner: Option<PlayerId>,
) -> Option<PlayerId> {
    const MIN: u8 = 5;
    let max = lengths.iter().copied().max().unwrap_or(0);
    if max < MIN {
        return None;
    }
    let leaders: Vec<PlayerId> = lengths
        .iter()
        .enumerate()
        .filter(|(_, &l)| l == max)
        .map(|(i, _)| i as PlayerId)
        .collect();

    // 保持者が同点最長に残っているなら保持し続ける
    if let Some(owner) = current_owner {
        if leaders.contains(&owner) {
            return Some(owner);
        }
    }
    // 単独最長なら交代、同点なら誰も持たない
    if leaders.len() == 1 {
        Some(leaders[0])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Building, BuildingKind};
    use crate::board::{BoardConfig, Resource};
    use crate::rng::Rng;
    use crate::topology::TileId;

    fn empty_board() -> Board {
        let mut rng = Rng::new(1);
        let mut b = Board::generate(&mut rng, BoardConfig::default());
        b.building = [None; NUM_NODES];
        b.road = [None; NUM_EDGES];
        b
    }

    /// 頂点 `from` から、まだ使っていない辺を辿って長さ `n` の一本道を敷く。
    /// 戻り値は通った頂点列。
    fn lay_path(b: &mut Board, p: PlayerId, from: NodeId, n: usize) -> Vec<NodeId> {
        let topo = Topology::get();
        let mut cur = from;
        let mut nodes = vec![cur];
        for _ in 0..n {
            let e = topo.node_edges[cur as usize]
                .into_iter()
                .find(|&e| b.road[e as usize].is_none())
                .expect("敷ける辺が無い");
            b.road[e as usize] = Some(p);
            let [x, y] = topo.edge_nodes[e as usize];
            cur = if x == cur { y } else { x };
            nodes.push(cur);
        }
        nodes
    }

    #[test]
    fn 道が無ければ0() {
        let b = empty_board();
        assert_eq!(longest_road_length(&b, 0), 0);
    }

    #[test]
    fn 一本道はその本数() {
        for n in 1..=8 {
            let mut b = empty_board();
            lay_path(&mut b, 0, 0, n);
            assert_eq!(longest_road_length(&b, 0) as usize, n, "{n} 本の一本道");
        }
    }

    #[test]
    fn 他人の道は数えない() {
        let mut b = empty_board();
        lay_path(&mut b, 0, 0, 5);
        assert_eq!(longest_road_length(&b, 1), 0);
    }

    #[test]
    fn 分岐は最長の枝だけ数える() {
        let topo = Topology::get();
        let mut b = empty_board();
        // 内陸の次数 3 の頂点を探し、そこから 3 方向へ 4/2/1 本ずつ伸ばす
        let hub = (0..NUM_NODES)
            .find(|&n| topo.node_edges[n].len() == 3)
            .unwrap() as NodeId;
        let mut lens = [4usize, 2, 1];
        lens.sort_unstable();
        let mut laid = 0;
        for (i, e) in topo.node_edges[hub as usize].into_iter().enumerate() {
            b.road[e as usize] = Some(0);
            laid += 1;
            // 枝の続きを伸ばす
            let [x, y] = topo.edge_nodes[e as usize];
            let tip = if x == hub { y } else { x };
            let extra = lens[i] - 1;
            lay_path(&mut b, 0, tip, extra);
            laid += extra;
        }
        assert_eq!(laid, lens.iter().sum::<usize>());
        // 最長は「最長の枝 + 2番目に長い枝」= hub を通り抜ける一本道
        let mut sorted = lens;
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(
            longest_road_length(&b, 0) as usize,
            sorted[0] + sorted[1],
            "分岐の扱いが違う"
        );
    }

    #[test]
    fn 六角形の輪は6() {
        let topo = Topology::get();
        let mut b = empty_board();
        // 中心ヘクスの 6 辺で輪を作る
        let center = topo.tile_of(crate::coord::Coord::ORIGIN).unwrap() as usize;
        for e in topo.tile_edges[center] {
            b.road[e as usize] = Some(0);
        }
        assert_eq!(longest_road_length(&b, 0), 6, "閉じた輪は 6 と数える");
    }

    #[test]
    fn 相手の建物は道を分断する() {
        let mut b = empty_board();
        let path = lay_path(&mut b, 0, 0, 6);
        assert_eq!(longest_road_length(&b, 0), 6);

        // 真ん中(3本目の先)の頂点に相手の開拓地
        let cut = path[3];
        b.building[cut as usize] = Some(Building {
            owner: 1,
            kind: BuildingKind::Settlement,
        });
        assert_eq!(longest_road_length(&b, 0), 3, "6 本が 3+3 に割れるはず");
    }

    #[test]
    fn 相手の建物で終わる道はその辺まで数える() {
        // catanatron はここを 1 本短く数えてしまう。公式解釈では 5 のまま。
        let mut b = empty_board();
        let path = lay_path(&mut b, 0, 0, 5);
        let end = *path.last().unwrap();
        b.building[end as usize] = Some(Building {
            owner: 1,
            kind: BuildingKind::Settlement,
        });
        assert_eq!(longest_road_length(&b, 0), 5, "終端の相手建物で短くなってはいけない");
    }

    #[test]
    fn 自分の建物は分断しない() {
        let mut b = empty_board();
        let path = lay_path(&mut b, 0, 0, 6);
        b.building[path[3] as usize] = Some(Building {
            owner: 0,
            kind: BuildingKind::City,
        });
        assert_eq!(longest_road_length(&b, 0), 6);
    }

    #[test]
    fn 最長路カードの帰属() {
        // 5 本未満なら誰も持たない
        assert_eq!(resolve_longest_road(&[4, 4, 3, 0], None), None);
        // 単独最長が取る
        assert_eq!(resolve_longest_road(&[5, 4, 3, 0], None), Some(0));
        // 同点なら誰も持たない（初取得時）
        assert_eq!(resolve_longest_road(&[5, 5, 3, 0], None), None);
        // 保持者が同点最長なら保持し続ける
        assert_eq!(resolve_longest_road(&[5, 5, 3, 0], Some(1)), Some(1));
        // 保持者を追い抜いたら交代
        assert_eq!(resolve_longest_road(&[6, 5, 3, 0], Some(1)), Some(0));
        // 保持者が最長でなくなり、新最長が同点 → 脇に置かれる
        assert_eq!(resolve_longest_road(&[6, 4, 6, 0], Some(1)), None);
        // 5 本未満に落ちたら手放す
        assert_eq!(resolve_longest_road(&[3, 4, 2, 0], Some(1)), None);
    }

    #[test]
    fn 道15本でも十分速い() {
        // 探索から毎秒何万回も呼ばれるので、最悪ケースの時間を見ておく
        let topo = Topology::get();
        let mut b = empty_board();
        // 中心付近の辺を 15 本、密に敷く（分岐だらけ = DFS の最悪形）
        let mut n = 0;
        'outer: for t in 0..crate::topology::NUM_TILES {
            for e in topo.tile_edges[t] {
                if b.road[e as usize].is_none() {
                    b.road[e as usize] = Some(0);
                    n += 1;
                    if n == 15 {
                        break 'outer;
                    }
                }
            }
        }
        let t0 = std::time::Instant::now();
        let mut acc = 0u64;
        for _ in 0..10_000 {
            acc += longest_road_length(&b, 0) as u64;
        }
        let dt = t0.elapsed();
        assert!(acc > 0);
        assert!(
            dt.as_millis() < 3000,
            "最長路計算が遅すぎる: 1 万回で {dt:?}"
        );
        eprintln!("最長路 1 万回: {dt:?}");
        let _ = Resource::Wood;
        let _: TileId = 0;
    }
}
