//! 盤面を画面座標に落とす。
//!
//! 立方体座標のまま計算できる。ヘクスは尖り上（左右に隣がある向き）で、
//! `East = (1,-1,0)` が真横になるように置くと:
//!
//! ```text
//! px = (√3/2)·s·(x − y)
//! py = 1.5·s·z
//! ```
//!
//! 頂点は「その頂点を共有する 3 ヘクスの中心の平均」で出る（正六角格子の性質）。
//! 辺は両端の中点。海のヘクスも座標としては同じ格子上にあるので、
//! 港の位置もこの式だけで出せる。

use catan_core::coord::Coord;
use catan_core::topology::{EdgeId, NodeId, TileId, Topology};

/// ヘクスの中心から頂点までの距離（画面上のピクセル）
pub const HEX_SIZE: f32 = 56.0;

const SQRT3_2: f32 = 0.866_025_4;

#[inline]
pub fn hex_center(c: Coord) -> (f32, f32) {
    (
        SQRT3_2 * HEX_SIZE * (c.x as f32 - c.y as f32),
        1.5 * HEX_SIZE * c.z as f32,
    )
}

pub fn tile_center(t: TileId) -> (f32, f32) {
    hex_center(Topology::get().tile_coord[t as usize])
}

/// 頂点の位置 = 共有する 3 ヘクスの中心の平均
pub fn node_pos(n: NodeId) -> (f32, f32) {
    let k = Topology::get().node_key(n);
    let mut x = 0.0;
    let mut y = 0.0;
    for c in k.0 {
        let (a, b) = hex_center(c);
        x += a;
        y += b;
    }
    (x / 3.0, y / 3.0)
}

/// 辺の両端
pub fn edge_ends(e: EdgeId) -> ((f32, f32), (f32, f32)) {
    let [a, b] = Topology::get().edge_nodes[e as usize];
    (node_pos(a), node_pos(b))
}

/// 港の印（船）を陸からどれだけ沖に出すか。
///
/// 陸ヘクスの中心から海ヘクスの中心までを 1 とした比。
/// 0.62 だと船が陸のヘクスに被る（船は上下に 50px 以上ある）ので、
/// ヘクスの縁（0.5 相当）から十分に離す。桟橋はその分だけ長くなる。
pub const PORT_OFFSHORE: f32 = 0.97;

/// 港の絵が印の位置から広がる大きさ（帆の先からレートの札まで）
pub const PORT_EXTENT: f32 = 31.0;

/// 港の印を置く位置（陸ヘクスの中心から、海側へ [`PORT_OFFSHORE`] だけ進んだ点）
pub fn port_marker(e: EdgeId) -> (f32, f32) {
    let topo = Topology::get();
    let k = topo.edge_key(e);
    // 辺を共有する 2 ヘクスのうち、陸でない方が海
    let (sea, land) = if topo.tile_of(k.0[0]).is_some() {
        (k.0[1], k.0[0])
    } else {
        (k.0[0], k.0[1])
    };
    let (sx, sy) = hex_center(sea);
    let (lx, ly) = hex_center(land);
    (lx + (sx - lx) * PORT_OFFSHORE, ly + (sy - ly) * PORT_OFFSHORE)
}

/// 描画範囲。陸の頂点と港の絵が全部入る矩形に余白を足したもの。
///
/// 枠（木の縁）は置かない方針にしたので、外接六角形の半径から逆算するのではなく
/// **中身の実際の広がりから** 決める。海は端まで続く。
pub fn bounds() -> (f32, f32, f32, f32) {
    let mut minx = f32::MAX;
    let mut miny = f32::MAX;
    let mut maxx = f32::MIN;
    let mut maxy = f32::MIN;

    let mut grow = |x: f32, y: f32, pad: f32| {
        minx = minx.min(x - pad);
        maxx = maxx.max(x + pad);
        miny = miny.min(y - pad);
        maxy = maxy.max(y + pad);
    };

    for n in 0..catan_core::topology::NUM_NODES {
        let (x, y) = node_pos(n as NodeId);
        grow(x, y, 0.0);
    }
    for e in catan_core::board::port_edges() {
        let (x, y) = port_marker(e);
        grow(x, y, PORT_EXTENT);
    }

    let m = HEX_SIZE * 0.30;
    (minx - m, miny - m, maxx + m, maxy + m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use catan_core::topology::{NUM_EDGES, NUM_NODES, NUM_TILES};

    #[test]
    fn 隣り合うヘクスの中心は等距離() {
        let topo = Topology::get();
        let mut dists = Vec::new();
        for t in 0..NUM_TILES {
            let c = topo.tile_coord[t];
            let (x0, y0) = hex_center(c);
            for nb in c.neighbors() {
                let (x1, y1) = hex_center(nb);
                dists.push(((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt());
            }
        }
        let d0 = dists[0];
        for d in &dists {
            assert!((d - d0).abs() < 0.01, "隣接距離がばらついている {d} vs {d0}");
        }
        // 尖り上ヘクスの中心間距離は √3·s
        assert!((d0 - 3f32.sqrt() * HEX_SIZE).abs() < 0.01);
    }

    #[test]
    fn 頂点はヘクス中心から一定距離() {
        let topo = Topology::get();
        for t in 0..NUM_TILES {
            let (cx, cy) = tile_center(t as TileId);
            for n in topo.tile_nodes[t] {
                let (x, y) = node_pos(n);
                let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                assert!(
                    (d - HEX_SIZE).abs() < 0.01,
                    "ヘクス {t} の頂点 {n} までの距離が {d}（{HEX_SIZE} のはず）"
                );
            }
        }
    }

    #[test]
    fn 頂点は重ならない() {
        let mut pts: Vec<(f32, f32)> = (0..NUM_NODES).map(|n| node_pos(n as NodeId)).collect();
        for i in 0..pts.len() {
            for j in (i + 1)..pts.len() {
                let d = ((pts[i].0 - pts[j].0).powi(2) + (pts[i].1 - pts[j].1).powi(2)).sqrt();
                assert!(d > 1.0, "頂点 {i} と {j} が重なっている");
            }
        }
        pts.clear();
    }

    #[test]
    fn 辺の長さは全部同じ() {
        let mut lens = Vec::new();
        for e in 0..NUM_EDGES {
            let ((x1, y1), (x2, y2)) = edge_ends(e as EdgeId);
            lens.push(((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt());
        }
        let l0 = lens[0];
        for l in &lens {
            assert!((l - l0).abs() < 0.01, "辺の長さがばらついている");
        }
        assert!((l0 - HEX_SIZE).abs() < 0.01, "辺の長さはヘクスの一辺と同じはず");
    }

    #[test]
    fn 描画範囲に陸と港が全部入る() {
        let (minx, miny, maxx, maxy) = bounds();
        for n in 0..NUM_NODES {
            let (x, y) = node_pos(n as NodeId);
            assert!(x > minx && x < maxx && y > miny && y < maxy, "頂点 {n} が範囲外");
        }
        for e in catan_core::board::port_edges() {
            let (x, y) = port_marker(e);
            assert!(
                x - PORT_EXTENT > minx && x + PORT_EXTENT < maxx
                    && y - PORT_EXTENT > miny && y + PORT_EXTENT < maxy,
                "港 {e} の絵が範囲外"
            );
        }
    }

    #[test]
    fn 港の船は陸のヘクスに被らない() {
        // 港の印から陸のどのヘクスの中心までも、
        // 「ヘクスの外接円 + 船の高さの半分」より遠いこと
        let topo = Topology::get();
        for e in catan_core::board::port_edges() {
            let (px, py) = port_marker(e);
            for t in 0..NUM_TILES {
                let (cx, cy) = tile_center(t as TileId);
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                assert!(
                    d > HEX_SIZE + 6.0,
                    "港 {e} がヘクス {t} に寄りすぎ（距離 {d:.1}）"
                );
            }
        }
        let _ = topo;
    }
}
