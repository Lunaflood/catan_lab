//! 盤面の静的トポロジー（19 ヘクス / 54 頂点 / 72 辺）。
//!
//! ゲーム中に一切変化しないので 1 度だけ構築して共有する。
//! 頂点・辺の同一性は [`crate::coord`] の「共有ヘクスの組」で定義しており、
//! 隣接表を手で書き下していないため、盤の形を変えても破綻しない。

use crate::coord::{BorderKey, Coord, CornerKey, Direction};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const NUM_TILES: usize = 19;
pub const NUM_NODES: usize = 54;
pub const NUM_EDGES: usize = 72;

/// 陸ヘクスの層数。0..=2 の 19 枚が陸、3 が海の枠。
pub const LAND_RINGS: u8 = 2;

pub type TileId = u8;
pub type NodeId = u8;
pub type EdgeId = u8;

/// 小さな固定長の集合。ヒープを踏まないための最小実装。
#[derive(Clone, Copy, Debug)]
pub struct Slots<const N: usize> {
    buf: [u8; N],
    len: u8,
}

impl<const N: usize> Slots<N> {
    #[inline]
    pub const fn new() -> Self {
        Slots { buf: [0; N], len: 0 }
    }
    #[inline]
    fn push(&mut self, v: u8) {
        assert!((self.len as usize) < N, "Slots<{N}> があふれた");
        self.buf[self.len as usize] = v;
        self.len += 1;
    }
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn contains(&self, v: u8) -> bool {
        self.as_slice().contains(&v)
    }
}

impl<const N: usize> Default for Slots<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, const N: usize> IntoIterator for &'a Slots<N> {
    type Item = u8;
    type IntoIter = std::iter::Copied<std::slice::Iter<'a, u8>>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter().copied()
    }
}

pub struct Topology {
    /// 陸ヘクスの座標（座標の昇順で ID が振られる）
    pub tile_coord: [Coord; NUM_TILES],
    /// ヘクスの 6 頂点（[`Coord::corners`] と同じ巡回順）
    pub tile_nodes: [[NodeId; 6]; NUM_TILES],
    /// ヘクスの 6 辺（[`Coord::borders`] と同じ巡回順）
    pub tile_edges: [[EdgeId; 6]; NUM_TILES],

    /// 頂点に接する陸ヘクス（1〜3 枚。海に面するほど少ない）
    pub node_tiles: [Slots<3>; NUM_NODES],
    /// 頂点に接続する辺（2 または 3 本）
    pub node_edges: [Slots<3>; NUM_NODES],
    /// 頂点と 1 辺で結ばれた頂点（= 距離ルールで塞がる相手）
    pub node_neighbors: [Slots<3>; NUM_NODES],

    /// 辺の両端の頂点
    pub edge_nodes: [[NodeId; 2]; NUM_EDGES],
    /// 端点を共有する辺（最大 4 本）
    pub edge_neighbors: [Slots<4>; NUM_EDGES],

    node_key: [CornerKey; NUM_NODES],
    edge_key: [BorderKey; NUM_EDGES],
    coord_to_tile: BTreeMap<Coord, TileId>,
    corner_to_node: BTreeMap<CornerKey, NodeId>,
    border_to_edge: BTreeMap<BorderKey, EdgeId>,
}

static TOPOLOGY: OnceLock<Topology> = OnceLock::new();

impl Topology {
    /// 共有インスタンス。初回だけ構築する。
    pub fn get() -> &'static Topology {
        TOPOLOGY.get_or_init(Topology::build)
    }

    pub fn tile_of(&self, c: Coord) -> Option<TileId> {
        self.coord_to_tile.get(&c).copied()
    }
    pub fn node_of(&self, k: CornerKey) -> Option<NodeId> {
        self.corner_to_node.get(&k).copied()
    }
    pub fn edge_of(&self, k: BorderKey) -> Option<EdgeId> {
        self.border_to_edge.get(&k).copied()
    }
    pub fn node_key(&self, n: NodeId) -> CornerKey {
        self.node_key[n as usize]
    }
    pub fn edge_key(&self, e: EdgeId) -> BorderKey {
        self.edge_key[e as usize]
    }

    /// 海ヘクス `water` から方向 `dir` を向いた港が乗る辺。
    /// 港は「海ヘクスと、それが向いている陸ヘクスの境界」に置かれる。
    pub fn port_edge(&self, water: Coord, dir: Direction) -> Option<EdgeId> {
        let land = water.add(dir.unit());
        self.edge_of(BorderKey::new(water, land))
    }

    fn build() -> Topology {
        // --- 1. 陸ヘクスを列挙（座標の昇順 = ID 順） ---
        let mut land: Vec<Coord> = Vec::new();
        for x in -(LAND_RINGS as i8)..=(LAND_RINGS as i8) {
            for y in -(LAND_RINGS as i8)..=(LAND_RINGS as i8) {
                let z = -x - y;
                let c = Coord::new(x, y, z);
                if c.ring() <= LAND_RINGS {
                    land.push(c);
                }
            }
        }
        land.sort_unstable();
        assert_eq!(land.len(), NUM_TILES, "陸ヘクスの枚数が {NUM_TILES} でない");

        // --- 2. 陸に接する頂点・辺を列挙（キーの昇順 = ID 順） ---
        let mut corners: Vec<CornerKey> = Vec::new();
        let mut borders: Vec<BorderKey> = Vec::new();
        for &c in &land {
            corners.extend_from_slice(&c.corners());
            borders.extend_from_slice(&c.borders());
        }
        corners.sort_unstable();
        corners.dedup();
        borders.sort_unstable();
        borders.dedup();
        assert_eq!(corners.len(), NUM_NODES, "頂点数が {NUM_NODES} でない");
        assert_eq!(borders.len(), NUM_EDGES, "辺数が {NUM_EDGES} でない");

        let coord_to_tile: BTreeMap<Coord, TileId> = land
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, i as TileId))
            .collect();
        let corner_to_node: BTreeMap<CornerKey, NodeId> = corners
            .iter()
            .enumerate()
            .map(|(i, &k)| (k, i as NodeId))
            .collect();
        let border_to_edge: BTreeMap<BorderKey, EdgeId> = borders
            .iter()
            .enumerate()
            .map(|(i, &k)| (k, i as EdgeId))
            .collect();

        // --- 3. ヘクス → 頂点 / 辺 ---
        let mut tile_coord = [Coord::ORIGIN; NUM_TILES];
        let mut tile_nodes = [[0u8; 6]; NUM_TILES];
        let mut tile_edges = [[0u8; 6]; NUM_TILES];
        for (t, &c) in land.iter().enumerate() {
            tile_coord[t] = c;
            for (i, k) in c.corners().into_iter().enumerate() {
                tile_nodes[t][i] = corner_to_node[&k];
            }
            for (i, k) in c.borders().into_iter().enumerate() {
                tile_edges[t][i] = border_to_edge[&k];
            }
        }

        // --- 4. 頂点 → 陸ヘクス ---
        let mut node_tiles = [Slots::<3>::new(); NUM_NODES];
        for (t, &c) in land.iter().enumerate() {
            for k in c.corners() {
                node_tiles[corner_to_node[&k] as usize].push(t as u8);
            }
        }

        // --- 5. 辺 → 端点、頂点 → 辺 ---
        let mut edge_nodes = [[u8::MAX; 2]; NUM_EDGES];
        let mut node_edges = [Slots::<3>::new(); NUM_NODES];
        for (e, bk) in borders.iter().enumerate() {
            let mut found = 0;
            for (n, ck) in corners.iter().enumerate() {
                if bk.is_endpoint_of(ck) {
                    assert!(found < 2, "辺 {bk:?} の端点が 3 個以上見つかった");
                    edge_nodes[e][found] = n as u8;
                    node_edges[n].push(e as u8);
                    found += 1;
                }
            }
            assert_eq!(found, 2, "辺 {bk:?} の端点が 2 個でない");
        }

        // --- 6. 頂点 → 隣接頂点、辺 → 隣接辺 ---
        let mut node_neighbors = [Slots::<3>::new(); NUM_NODES];
        for n in 0..NUM_NODES {
            for e in &node_edges[n] {
                let [a, b] = edge_nodes[e as usize];
                node_neighbors[n].push(if a as usize == n { b } else { a });
            }
        }

        let mut edge_neighbors = [Slots::<4>::new(); NUM_EDGES];
        for e in 0..NUM_EDGES {
            for n in edge_nodes[e] {
                for other in &node_edges[n as usize] {
                    if other as usize != e && !edge_neighbors[e].contains(other) {
                        edge_neighbors[e].push(other);
                    }
                }
            }
        }

        let mut node_key = [CornerKey::new(Coord::ORIGIN, Coord::ORIGIN, Coord::ORIGIN); NUM_NODES];
        node_key.copy_from_slice(&corners);
        let mut edge_key = [BorderKey::new(Coord::ORIGIN, Coord::ORIGIN); NUM_EDGES];
        edge_key.copy_from_slice(&borders);

        Topology {
            tile_coord,
            tile_nodes,
            tile_edges,
            node_tiles,
            node_edges,
            node_neighbors,
            edge_nodes,
            edge_neighbors,
            node_key,
            edge_key,
            coord_to_tile,
            corner_to_node,
            border_to_edge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 基本の数え上げ() {
        let t = Topology::get();
        assert_eq!(t.tile_coord.len(), 19);
        assert_eq!(t.node_tiles.len(), 54);
        assert_eq!(t.edge_nodes.len(), 72);
    }

    #[test]
    fn 頂点の次数は2か3で総和は辺数の2倍() {
        let t = Topology::get();
        let mut deg2 = 0;
        let mut deg3 = 0;
        let mut sum = 0;
        for n in 0..NUM_NODES {
            let d = t.node_edges[n].len();
            assert!(d == 2 || d == 3, "頂点 {n} の次数が {d}");
            sum += d;
            if d == 2 {
                deg2 += 1
            } else {
                deg3 += 1
            }
        }
        assert_eq!(sum, 2 * NUM_EDGES);
        // sum = 144, 全次数が 2 か 3 なら分布は一意に決まる
        assert_eq!((deg2, deg3), (18, 36));
    }

    #[test]
    fn 頂点に接する陸ヘクスは1から3枚で総和は114() {
        let t = Topology::get();
        let mut sum = 0;
        let mut three = 0;
        for n in 0..NUM_NODES {
            let k = t.node_tiles[n].len();
            assert!((1..=3).contains(&k), "頂点 {n} に接する陸が {k} 枚");
            sum += k;
            if k == 3 {
                three += 1;
            }
        }
        assert_eq!(sum, NUM_TILES * 6);
        // 内陸の頂点（3 ヘクスすべてが陸）は 24 個
        assert_eq!(three, 24);
    }

    #[test]
    fn 隣接関係は対称() {
        let t = Topology::get();
        for n in 0..NUM_NODES {
            for m in &t.node_neighbors[n] {
                assert!(
                    t.node_neighbors[m as usize].contains(n as u8),
                    "頂点 {n}→{m} が非対称"
                );
                assert_ne!(m as usize, n, "自己ループ");
            }
        }
        for e in 0..NUM_EDGES {
            for f in &t.edge_neighbors[e] {
                assert!(
                    t.edge_neighbors[f as usize].contains(e as u8),
                    "辺 {e}→{f} が非対称"
                );
            }
        }
    }

    #[test]
    fn ヘクスの6頂点6辺は相異なり整合する() {
        let t = Topology::get();
        for tile in 0..NUM_TILES {
            let ns = t.tile_nodes[tile];
            let es = t.tile_edges[tile];
            for i in 0..6 {
                for j in (i + 1)..6 {
                    assert_ne!(ns[i], ns[j], "ヘクス {tile} の頂点が重複");
                    assert_ne!(es[i], es[j], "ヘクス {tile} の辺が重複");
                }
            }
            // 各辺の端点は、そのヘクスの頂点に含まれる
            for &e in &es {
                for n in t.edge_nodes[e as usize] {
                    assert!(ns.contains(&n), "ヘクス {tile} の辺 {e} の端点 {n} が頂点集合外");
                }
            }
            // 各頂点はそのヘクスに接している
            for &n in &ns {
                assert!(t.node_tiles[n as usize].contains(tile as u8));
            }
        }
    }

    #[test]
    fn 辺の隣接数は2から4() {
        let t = Topology::get();
        for e in 0..NUM_EDGES {
            let d = t.edge_neighbors[e].len();
            assert!((2..=4).contains(&d), "辺 {e} の隣接辺が {d} 本");
        }
    }

    #[test]
    fn 盤は連結() {
        let t = Topology::get();
        let mut seen = [false; NUM_NODES];
        let mut stack = vec![0u8];
        seen[0] = true;
        let mut count = 1;
        while let Some(n) = stack.pop() {
            for m in &t.node_neighbors[n as usize] {
                if !seen[m as usize] {
                    seen[m as usize] = true;
                    count += 1;
                    stack.push(m);
                }
            }
        }
        assert_eq!(count, NUM_NODES, "頂点グラフが連結でない");
    }
}
