//! 決定的な擬似乱数。外部クレートを使わない理由は [`crate`] のとおり。
//!
//! PCG-XSH-RR 64/32。周期 2^64、状態 16 バイト、複製が安い。
//! ダイスは「公平であること」自体が検証対象なので、剰余バイアスを残さない
//! （Lemire の方法 + 棄却）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    state: u64,
    inc: u64,
}

const MULT: u64 = 6364136223846793005;

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self::with_stream(seed, 0xda3e_39cb_94b9_5bdb)
    }

    pub fn with_stream(seed: u64, stream: u64) -> Self {
        let mut r = Rng {
            state: 0,
            inc: (stream << 1) | 1,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULT).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// `0..n` の一様乱数。n = 0 はパニック。
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "below(0)");
        // Lemire: 乗算した上位ビットを取り、下位が閾値未満の時だけ棄却する
        let mut m = (self.next_u32() as u64).wrapping_mul(n as u64);
        let mut low = m as u32;
        if low < n {
            let threshold = n.wrapping_neg() % n;
            while low < threshold {
                m = (self.next_u32() as u64).wrapping_mul(n as u64);
                low = m as u32;
            }
        }
        (m >> 32) as u32
    }

    /// 2 個のサイコロ。合計ではなく個別に返す（記録・再現のため）。
    #[inline]
    pub fn dice(&mut self) -> (u8, u8) {
        (
            (self.below(6) + 1) as u8,
            (self.below(6) + 1) as u8,
        )
    }

    /// Fisher-Yates。
    pub fn shuffle<T>(&mut self, xs: &mut [T]) {
        if xs.len() < 2 {
            return;
        }
        for i in (1..xs.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            xs.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同じseedなら同じ列() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
        let mut c = Rng::new(43);
        assert_ne!(a.next_u32(), c.next_u32());
    }

    #[test]
    fn belowは範囲内() {
        let mut r = Rng::new(7);
        for n in 1..=20u32 {
            for _ in 0..500 {
                assert!(r.below(n) < n);
            }
        }
    }

    #[test]
    fn ダイスの分布が理論値に近い() {
        // 2d6 の分布。36 万回振って、各目の実測比率が理論値の ±3% 以内。
        let mut r = Rng::new(0xC0FFEE);
        let n = 360_000;
        let mut hist = [0u32; 13];
        for _ in 0..n {
            let (a, b) = r.dice();
            assert!((1..=6).contains(&a) && (1..=6).contains(&b));
            hist[(a + b) as usize] += 1;
        }
        let ways = [0, 0, 1, 2, 3, 4, 5, 6, 5, 4, 3, 2, 1];
        for pip in 2..=12usize {
            let expected = n as f64 * ways[pip] as f64 / 36.0;
            let got = hist[pip] as f64;
            let err = (got - expected).abs() / expected;
            assert!(err < 0.03, "{pip} の出現率が理論から {:.1}% ずれた", err * 100.0);
        }
    }

    #[test]
    fn シャッフルは要素を保存する() {
        let mut r = Rng::new(1);
        let mut v: Vec<u32> = (0..100).collect();
        r.shuffle(&mut v);
        assert_ne!(v, (0..100).collect::<Vec<_>>(), "並びが変わっていない");
        v.sort_unstable();
        assert_eq!(v, (0..100).collect::<Vec<_>>());
    }
}
