//! 盤面（地形・数字チップ・港・建造物）。
//!
//! 公式ルールブック 5th ed. (2020) の「Set-up, Variable」に準拠:
//! - 地形 19 枚をシャッフルして配置
//! - 港 9 枚を枠上にランダム配置
//! - 数字チップは盤の角から反時計回りに渦巻き状、裏のアルファベット順(A〜R)。砂漠はスキップ
//! - 完全ランダムにする場合は「赤数字(6と8)を隣接させない」

use crate::coord::{Coord, Direction, DIRECTIONS};
use crate::rng::Rng;
use crate::topology::{EdgeId, NodeId, TileId, Topology, NUM_EDGES, NUM_NODES, NUM_TILES};

pub type PlayerId = u8;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Resource {
    Wood = 0,
    Brick = 1,
    Sheep = 2,
    Wheat = 3,
    Ore = 4,
}

pub const NUM_RESOURCES: usize = 5;
pub const RESOURCES: [Resource; NUM_RESOURCES] = [
    Resource::Wood,
    Resource::Brick,
    Resource::Sheep,
    Resource::Wheat,
    Resource::Ore,
];

impl Resource {
    #[inline]
    pub const fn idx(self) -> usize {
        self as usize
    }
    #[inline]
    pub const fn from_idx(i: usize) -> Resource {
        RESOURCES[i]
    }
    pub const fn ja(self) -> &'static str {
        match self {
            Resource::Wood => "木材",
            Resource::Brick => "土",
            Resource::Sheep => "羊毛",
            Resource::Wheat => "小麦",
            Resource::Ore => "鉄鉱",
        }
    }
}

/// 港の種類。`Generic` が 3:1、`Specific` が該当資源のみ 2:1。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortKind {
    Generic,
    Specific(Resource),
}

impl PortKind {
    /// その港で `res` を出す時の交換レート。港が無い場合の 4:1 は呼び出し側で扱う。
    #[inline]
    pub fn rate_for(self, res: Resource) -> u8 {
        match self {
            PortKind::Generic => 3,
            // 特化港は「その資源だけ」2:1。他は 3:1 にすらならない（公式アルマナック）
            PortKind::Specific(r) if r == res => 2,
            PortKind::Specific(_) => 4,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildingKind {
    Settlement,
    City,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Building {
    pub owner: PlayerId,
    pub kind: BuildingKind,
}

/// 地形の枚数（公式）: 森4・丘陵3・牧草地4・畑4・山地3・砂漠1
pub const TILE_RESOURCES: [Option<Resource>; NUM_TILES] = [
    Some(Resource::Wood),
    Some(Resource::Wood),
    Some(Resource::Wood),
    Some(Resource::Wood),
    Some(Resource::Brick),
    Some(Resource::Brick),
    Some(Resource::Brick),
    Some(Resource::Sheep),
    Some(Resource::Sheep),
    Some(Resource::Sheep),
    Some(Resource::Sheep),
    Some(Resource::Wheat),
    Some(Resource::Wheat),
    Some(Resource::Wheat),
    Some(Resource::Wheat),
    Some(Resource::Ore),
    Some(Resource::Ore),
    Some(Resource::Ore),
    None, // 砂漠
];

/// 数字チップ 18 枚を、裏面のアルファベット A〜R の順に並べたもの。
/// 渦巻き順に置いていくと公式の「初心者以外向け」配置になる。
pub const NUMBERS_IN_SPIRAL_ORDER: [u8; 18] =
    [5, 2, 6, 3, 8, 10, 9, 12, 11, 4, 8, 10, 9, 4, 5, 6, 3, 11];

/// 数字チップの全種類（渦巻きを使わず完全ランダムに置く場合用）
pub const NUMBER_MULTISET: [u8; 18] = [2, 3, 3, 4, 4, 5, 5, 6, 6, 8, 8, 9, 9, 10, 10, 11, 11, 12];

/// 港 9 か所。(海ヘクスの座標, その港が向いている方向)。
/// 港は「海ヘクスと、向いた先の陸ヘクスの境界の辺」に乗る。
pub const PORT_SLOTS: [(Coord, Direction); 9] = [
    (Coord::new(3, -3, 0), Direction::West),
    (Coord::new(1, -3, 2), Direction::NorthWest),
    (Coord::new(-1, -2, 3), Direction::NorthWest),
    (Coord::new(-3, 0, 3), Direction::NorthEast),
    (Coord::new(-3, 2, 1), Direction::East),
    (Coord::new(-2, 3, -1), Direction::East),
    (Coord::new(0, 3, -3), Direction::SouthEast),
    (Coord::new(2, 1, -3), Direction::SouthWest),
    (Coord::new(3, -1, -2), Direction::SouthWest),
];

/// 港の内訳: 2:1 が 5 種各 1、3:1 が 4 枚
pub const PORT_KINDS: [PortKind; 9] = [
    PortKind::Specific(Resource::Wood),
    PortKind::Specific(Resource::Brick),
    PortKind::Specific(Resource::Sheep),
    PortKind::Specific(Resource::Wheat),
    PortKind::Specific(Resource::Ore),
    PortKind::Generic,
    PortKind::Generic,
    PortKind::Generic,
    PortKind::Generic,
];

/// 出目の出現通り数（36 分の何か）。7 は生産しないので 0 を返す。
#[inline]
pub const fn pips(number: u8) -> u8 {
    match number {
        2 | 12 => 1,
        3 | 11 => 2,
        4 | 10 => 3,
        5 | 9 => 4,
        6 | 8 => 5,
        _ => 0,
    }
}

/// 数字チップの置き方
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NumberPlacement {
    /// 公式の渦巻き順（A〜R）。盤の形が同じなら数字の並びも常に同じになる。
    OfficialSpiral,
    /// 完全ランダム。赤数字(6/8)が隣接しないように置き直す。
    RandomNoAdjacentReds,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BoardConfig {
    pub number_placement: NumberPlacement,
}

impl Default for BoardConfig {
    fn default() -> Self {
        BoardConfig {
            number_placement: NumberPlacement::OfficialSpiral,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Board {
    // --- 地図（ゲーム中は不変） ---
    pub tile_resource: [Option<Resource>; NUM_TILES],
    /// 数字チップ。砂漠は 0。
    pub tile_number: [u8; NUM_TILES],
    pub node_port: [Option<PortKind>; NUM_NODES],

    // --- 局面（変化する） ---
    pub robber: TileId,
    pub building: [Option<Building>; NUM_NODES],
    pub road: [Option<PlayerId>; NUM_EDGES],
}

impl Board {
    /// 公式の可変セットアップで盤を作る。
    pub fn generate(rng: &mut Rng, cfg: BoardConfig) -> Board {
        let topo = Topology::get();

        // --- 地形をシャッフルして配置 ---
        let mut tiles = TILE_RESOURCES;
        rng.shuffle(&mut tiles);

        // --- 数字チップ ---
        let mut tile_number = [0u8; NUM_TILES];
        match cfg.number_placement {
            NumberPlacement::OfficialSpiral => {
                let mut i = 0;
                for t in spiral_order() {
                    if tiles[t as usize].is_none() {
                        continue; // 砂漠はスキップ
                    }
                    tile_number[t as usize] = NUMBERS_IN_SPIRAL_ORDER[i];
                    i += 1;
                }
                debug_assert_eq!(i, 18);
            }
            NumberPlacement::RandomNoAdjacentReds => {
                // 赤数字が隣接しない配置が出るまで引き直す。
                // 18 枚中 6/8 は 4 枚しかないので、数回で通る。
                loop {
                    let mut nums = NUMBER_MULTISET;
                    rng.shuffle(&mut nums);
                    let mut i = 0;
                    tile_number = [0u8; NUM_TILES];
                    for t in 0..NUM_TILES {
                        if tiles[t].is_none() {
                            continue;
                        }
                        tile_number[t] = nums[i];
                        i += 1;
                    }
                    if !has_adjacent_reds(&tile_number) {
                        break;
                    }
                }
            }
        }

        // --- 港 ---
        let mut kinds = PORT_KINDS;
        rng.shuffle(&mut kinds);
        let mut node_port = [None; NUM_NODES];
        for (slot, kind) in PORT_SLOTS.iter().zip(kinds) {
            let edge = topo
                .port_edge(slot.0, slot.1)
                .expect("港のスロットが盤上の辺に対応していない");
            for n in topo.edge_nodes[edge as usize] {
                node_port[n as usize] = Some(kind);
            }
        }

        // --- 盗賊は砂漠から ---
        let robber = tiles
            .iter()
            .position(|r| r.is_none())
            .expect("砂漠が無い") as TileId;

        Board {
            tile_resource: tiles,
            tile_number,
            node_port,
            robber,
            building: [None; NUM_NODES],
            road: [None; NUM_EDGES],
        }
    }

    #[inline]
    pub fn tile_pips(&self, t: TileId) -> u8 {
        pips(self.tile_number[t as usize])
    }

    /// その頂点に開拓地を建てた時の、資源別の期待産出（単位は pip = 1/36）。
    /// 盗賊は考慮しない。AI の配置評価の土台。
    pub fn node_pips(&self, n: NodeId) -> [u8; NUM_RESOURCES] {
        let topo = Topology::get();
        let mut out = [0u8; NUM_RESOURCES];
        for t in &topo.node_tiles[n as usize] {
            if let Some(r) = self.tile_resource[t as usize] {
                out[r.idx()] += self.tile_pips(t);
            }
        }
        out
    }

    /// プレイヤーが押さえている港の一覧（重複あり）。
    pub fn ports_of(&self, p: PlayerId) -> Vec<PortKind> {
        let mut out = Vec::new();
        for n in 0..NUM_NODES {
            if let (Some(b), Some(port)) = (self.building[n], self.node_port[n]) {
                if b.owner == p {
                    out.push(port);
                }
            }
        }
        out
    }

    /// `p` が `res` を銀行に出す時の最良レート（4:1 / 3:1 / 2:1）。
    pub fn best_maritime_rate(&self, p: PlayerId, res: Resource) -> u8 {
        let mut best = 4;
        for port in self.ports_of(p) {
            best = best.min(port.rate_for(res));
        }
        best
    }
}

/// 数字チップを置く順序（盤の角から反時計回りに、外周→内周）。
/// 公式ルールの「渦巻き状にアルファベット順」に対応する。
pub fn spiral_order() -> Vec<TileId> {
    let topo = Topology::get();
    let mut out = Vec::with_capacity(NUM_TILES);
    for r in (0..=crate::topology::LAND_RINGS).rev() {
        if r == 0 {
            out.push(topo.tile_of(Coord::ORIGIN).unwrap());
            continue;
        }
        // 各リングは「East の角」から始め、反時計回りに 6 辺を歩く。
        let mut c = Coord::ORIGIN;
        for _ in 0..r {
            c = c.add(Direction::East.unit());
        }
        for i in 0..6 {
            let step = DIRECTIONS[(i + 2) % 6].unit();
            for _ in 0..r {
                out.push(topo.tile_of(c).expect("渦巻きが盤外に出た"));
                c = c.add(step);
            }
        }
    }
    debug_assert_eq!(out.len(), NUM_TILES);
    out
}

/// 赤数字(6/8)が隣接しているか
fn has_adjacent_reds(tile_number: &[u8; NUM_TILES]) -> bool {
    let topo = Topology::get();
    for t in 0..NUM_TILES {
        let n = tile_number[t];
        if n != 6 && n != 8 {
            continue;
        }
        for nb in topo.tile_coord[t].neighbors() {
            if let Some(u) = topo.tile_of(nb) {
                let m = tile_number[u as usize];
                if m == 6 || m == 8 {
                    return true;
                }
            }
        }
    }
    false
}

/// 港が乗る 9 本の辺（描画・検証用）
pub fn port_edges() -> Vec<EdgeId> {
    let topo = Topology::get();
    PORT_SLOTS
        .iter()
        .map(|(c, d)| topo.port_edge(*c, *d).expect("港の辺が無い"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn 渦巻き順は19枚を1回ずつ通る() {
        let order = spiral_order();
        assert_eq!(order.len(), NUM_TILES);
        let uniq: HashSet<_> = order.iter().copied().collect();
        assert_eq!(uniq.len(), NUM_TILES);
        // 外周(12枚) → 中周(6枚) → 中心(1枚)
        let topo = Topology::get();
        let rings: Vec<u8> = order.iter().map(|&t| topo.tile_coord[t as usize].ring()).collect();
        assert_eq!(&rings[..12], &[2; 12]);
        assert_eq!(&rings[12..18], &[1; 6]);
        assert_eq!(rings[18], 0);
        // 渦巻きは隣接ヘクスを辿る（外周内・中周内で切れていない）
        for w in order[..12].windows(2) {
            let a = topo.tile_coord[w[0] as usize];
            let b = topo.tile_coord[w[1] as usize];
            let d = Coord::new(a.x - b.x, a.y - b.y, a.z - b.z);
            assert_eq!(d.ring(), 1, "外周の渦巻きが飛んでいる");
        }
    }

    #[test]
    fn 港は9本の相異なる辺に乗り18頂点を覆う() {
        let edges = port_edges();
        assert_eq!(edges.len(), 9);
        let uniq: HashSet<_> = edges.iter().copied().collect();
        assert_eq!(uniq.len(), 9, "港の辺が重複");

        let topo = Topology::get();
        let mut nodes = HashSet::new();
        for e in edges {
            for n in topo.edge_nodes[e as usize] {
                nodes.insert(n);
            }
        }
        assert_eq!(nodes.len(), 18, "港に接する頂点が 18 個でない");
    }

    #[test]
    fn 港の辺はすべて海岸線() {
        // 港の辺は「陸1枚だけに接する」= 外周でなければならない
        let topo = Topology::get();
        for e in port_edges() {
            let mut land = 0;
            for t in 0..NUM_TILES {
                if topo.tile_edges[t].contains(&e) {
                    land += 1;
                }
            }
            assert_eq!(land, 1, "辺 {e} は内陸にある");
        }
    }

    #[test]
    fn 生成した盤の内訳が公式どおり() {
        for seed in 0..200u64 {
            let mut rng = Rng::new(seed);
            let b = Board::generate(&mut rng, BoardConfig::default());

            let mut counts = [0usize; NUM_RESOURCES];
            let mut desert = 0;
            for t in b.tile_resource {
                match t {
                    Some(r) => counts[r.idx()] += 1,
                    None => desert += 1,
                }
            }
            assert_eq!(counts, [4, 3, 4, 4, 3], "地形の枚数が違う (seed={seed})");
            assert_eq!(desert, 1);

            // 数字チップ: 砂漠以外の 18 枚に、規定の多重集合がちょうど乗る
            let mut nums: Vec<u8> = (0..NUM_TILES)
                .filter(|&t| b.tile_resource[t].is_some())
                .map(|t| b.tile_number[t])
                .collect();
            assert_eq!(nums.len(), 18);
            nums.sort_unstable();
            assert_eq!(nums, NUMBER_MULTISET.to_vec(), "数字チップの内訳が違う (seed={seed})");
            // 砂漠には数字が乗らない
            for t in 0..NUM_TILES {
                if b.tile_resource[t].is_none() {
                    assert_eq!(b.tile_number[t], 0);
                }
            }
            // 盗賊は砂漠から
            assert!(b.tile_resource[b.robber as usize].is_none());

            // 港: 2:1 が 5 種 1 枚ずつ、3:1 が 4 枚
            let mut generic = 0;
            let mut specific = HashSet::new();
            let mut seen_edges = HashSet::new();
            for (c, d) in PORT_SLOTS {
                let e = Topology::get().port_edge(c, d).unwrap();
                seen_edges.insert(e);
                let n = Topology::get().edge_nodes[e as usize][0];
                match b.node_port[n as usize].unwrap() {
                    PortKind::Generic => generic += 1,
                    PortKind::Specific(r) => {
                        assert!(specific.insert(r), "同じ 2:1 港が 2 つある");
                    }
                }
            }
            assert_eq!(generic, 4);
            assert_eq!(specific.len(), 5);
            assert_eq!(seen_edges.len(), 9);
        }
    }

    #[test]
    fn ランダム配置では赤数字が隣接しない() {
        let cfg = BoardConfig {
            number_placement: NumberPlacement::RandomNoAdjacentReds,
        };
        for seed in 0..100u64 {
            let mut rng = Rng::new(seed);
            let b = Board::generate(&mut rng, cfg);
            assert!(!has_adjacent_reds(&b.tile_number), "赤数字が隣接 (seed={seed})");
        }
    }

    #[test]
    fn 特化港は他資源を優遇しない() {
        let p = PortKind::Specific(Resource::Ore);
        assert_eq!(p.rate_for(Resource::Ore), 2);
        // 公式アルマナック: 特化港は他資源では 3:1 にすらならない
        assert_eq!(p.rate_for(Resource::Wood), 4);
        assert_eq!(PortKind::Generic.rate_for(Resource::Wood), 3);
    }

    #[test]
    fn 頂点のpipsは隣接ヘクスの合計() {
        let mut rng = Rng::new(1);
        let b = Board::generate(&mut rng, BoardConfig::default());
        let topo = Topology::get();
        // 盤全体の pip 合計 = 各ヘクスの pip × 接する頂点数(6)
        let total: u32 = (0..NUM_NODES).map(|n| b.node_pips(n as NodeId).iter().map(|&v| v as u32).sum::<u32>()).sum();
        let expect: u32 = (0..NUM_TILES)
            .filter(|&t| b.tile_resource[t].is_some())
            .map(|t| b.tile_pips(t as TileId) as u32 * 6)
            .sum();
        assert_eq!(total, expect);
        let _ = topo;
    }
}
