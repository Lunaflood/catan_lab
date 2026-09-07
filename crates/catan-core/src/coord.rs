//! 立方体座標(cube coordinates)によるヘクス格子。
//!
//! `x + y + z == 0` を常に満たす。方向ベクトルは反時計回りの巡回順で並べてあり、
//! 「隣り合う 2 方向 + 自分」が必ず 1 つの頂点を成す、という性質に依存している
//! （[`Coord::corners`] / [`crate::topology`] がこれを使う）。

/// ヘクスの立方体座標。`x + y + z == 0`。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Coord {
    pub x: i8,
    pub y: i8,
    pub z: i8,
}

impl Coord {
    pub const ORIGIN: Coord = Coord::new(0, 0, 0);

    #[inline]
    pub const fn new(x: i8, y: i8, z: i8) -> Self {
        Coord { x, y, z }
    }

    #[inline]
    pub const fn add(self, o: Coord) -> Coord {
        Coord::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    /// 原点からの距離（= 何層目か）。半径 2 までが陸、3 が海の枠。
    #[inline]
    pub const fn ring(self) -> u8 {
        let a = self.x.unsigned_abs();
        let b = self.y.unsigned_abs();
        let c = self.z.unsigned_abs();
        // (|x| + |y| + |z|) / 2
        ((a as u16 + b as u16 + c as u16) / 2) as u8
    }

    /// 6 方向の隣接ヘクス。返る順序は [`DIRECTIONS`] と同じ巡回順。
    #[inline]
    pub fn neighbors(self) -> [Coord; 6] {
        let mut out = [Coord::ORIGIN; 6];
        let mut i = 0;
        while i < 6 {
            out[i] = self.add(DIRECTIONS[i].unit());
            i += 1;
        }
        out
    }

    /// このヘクスの 6 頂点。各頂点は「自分 + 隣接する 2 ヘクス」の正規化済み三つ組。
    ///
    /// 無限格子上では任意の頂点はちょうど 3 ヘクスに共有されるので、
    /// この三つ組は頂点の一意な識別子になる（海側のヘクスも識別子として使う）。
    pub fn corners(self) -> [CornerKey; 6] {
        let n = self.neighbors();
        let mut out = [CornerKey::DUMMY; 6];
        for i in 0..6 {
            out[i] = CornerKey::new(self, n[i], n[(i + 1) % 6]);
        }
        out
    }

    /// このヘクスの 6 辺。各辺は「自分 + 隣接 1 ヘクス」の正規化済み二つ組。
    ///
    /// 無限格子上では任意の辺はちょうど 2 ヘクスに共有される。
    pub fn borders(self) -> [BorderKey; 6] {
        let n = self.neighbors();
        let mut out = [BorderKey::DUMMY; 6];
        for i in 0..6 {
            out[i] = BorderKey::new(self, n[i]);
        }
        out
    }
}

/// ヘクスの 6 方向。反時計回りの巡回順（隣り合う 2 つは互いに隣接する）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    East,
    NorthEast,
    NorthWest,
    West,
    SouthWest,
    SouthEast,
}

pub const DIRECTIONS: [Direction; 6] = [
    Direction::East,
    Direction::NorthEast,
    Direction::NorthWest,
    Direction::West,
    Direction::SouthWest,
    Direction::SouthEast,
];

impl Direction {
    #[inline]
    pub const fn unit(self) -> Coord {
        match self {
            Direction::East => Coord::new(1, -1, 0),
            Direction::NorthEast => Coord::new(1, 0, -1),
            Direction::NorthWest => Coord::new(0, 1, -1),
            Direction::West => Coord::new(-1, 1, 0),
            Direction::SouthWest => Coord::new(-1, 0, 1),
            Direction::SouthEast => Coord::new(0, -1, 1),
        }
    }
}

/// 頂点の一意キー: 頂点を共有する 3 ヘクスの座標を昇順に並べたもの。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CornerKey(pub [Coord; 3]);

impl CornerKey {
    const DUMMY: CornerKey = CornerKey([Coord::ORIGIN; 3]);

    pub fn new(a: Coord, b: Coord, c: Coord) -> Self {
        let mut v = [a, b, c];
        v.sort_unstable();
        CornerKey(v)
    }

    /// 3 ヘクスのうち 2 つを共有していれば、その 2 頂点は 1 辺で結ばれている。
    pub fn shares_two_with(&self, other: &CornerKey) -> bool {
        let mut n = 0;
        for a in &self.0 {
            if other.0.contains(a) {
                n += 1;
            }
        }
        n == 2
    }
}

/// 辺の一意キー: 辺を共有する 2 ヘクスの座標を昇順に並べたもの。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BorderKey(pub [Coord; 2]);

impl BorderKey {
    const DUMMY: BorderKey = BorderKey([Coord::ORIGIN; 2]);

    pub fn new(a: Coord, b: Coord) -> Self {
        let mut v = [a, b];
        v.sort_unstable();
        BorderKey(v)
    }

    /// この辺が、与えられた頂点の 3 ヘクスのうち 2 つで構成されているか
    /// （= その頂点が辺の端点か）。
    pub fn is_endpoint_of(&self, corner: &CornerKey) -> bool {
        corner.0.contains(&self.0[0]) && corner.0.contains(&self.0[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 立方体座標の不変条件() {
        for d in DIRECTIONS {
            let u = d.unit();
            assert_eq!(u.x + u.y + u.z, 0, "{d:?} が x+y+z=0 を破っている");
            assert_eq!(u.ring(), 1);
        }
    }

    #[test]
    fn 方向は巡回順に並んでいる() {
        // 隣り合う 2 方向のヘクスは互いに隣接していなければならない。
        // これが崩れると corners() が実在しない頂点を作る。
        for i in 0..6 {
            let a = DIRECTIONS[i].unit();
            let b = DIRECTIONS[(i + 1) % 6].unit();
            let diff = Coord::new(a.x - b.x, a.y - b.y, a.z - b.z);
            assert_eq!(diff.ring(), 1, "{:?} と {:?} が隣接していない", DIRECTIONS[i], DIRECTIONS[(i + 1) % 6]);
        }
    }

    #[test]
    fn 頂点と辺の数え上げ() {
        // 1 ヘクス単体では 6 頂点 6 辺、すべて相異なる。
        let c = Coord::ORIGIN;
        let corners = c.corners();
        for i in 0..6 {
            for j in (i + 1)..6 {
                assert_ne!(corners[i], corners[j], "頂点 {i} と {j} が重複");
            }
        }
        let borders = c.borders();
        for i in 0..6 {
            for j in (i + 1)..6 {
                assert_ne!(borders[i], borders[j], "辺 {i} と {j} が重複");
            }
        }
    }

    #[test]
    fn 辺の端点は隣接する2頂点() {
        let c = Coord::ORIGIN;
        let corners = c.corners();
        for b in c.borders() {
            let ends: Vec<_> = corners.iter().filter(|k| b.is_endpoint_of(k)).collect();
            assert_eq!(ends.len(), 2, "辺 {b:?} の端点が 2 個でない");
            assert!(ends[0].shares_two_with(ends[1]));
        }
    }
}
