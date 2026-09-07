//! 相手モデル（M2）: 発展カードの「使用機会ごとの使用確率」と、相手の傾向の混合。
//!
//! 推定の骨子は設計書 6.4:
//!   `P(c | 不使用履歴) ∝ P(c) × Π_t (1 − q(c, x_t, θ))`
//! を、カード単位ではなく**手札単位**（1 手番 1 枚制限つき）で回す。
//! 手番 t に何も使わなかった尤度は `1 − max_{c∈手札, 使用可} q(c, x_t, θ)`。
//! 同じ種類を 2 枚持っていても不使用の証拠を二重に掛けない。
//!
//! `q` の数値は説明のための初期値であり、`catan v2-fit` で凍結した対照 CPU の対局から
//! 実測した使用率で置き換える（docs/cpu-v2/m2-belief.md に記録）。
//! 人間や別方針の相手には合わない可能性があるので、**傾向 θ の混合**で過信を抑える。

use catan_core::action::DevCard;
use catan_core::observation::EventContext;
use catan_core::board::PlayerId;

/// 相手の傾向。表示のためではなく、予測の過信を抑えるための潜在変数。
pub const NUM_STYLES: usize = 4;
pub const STYLE_EAGER: u8 = 0;
pub const STYLE_BALANCED: u8 = 1;
pub const STYLE_HOARDER: u8 = 2;
pub const STYLE_NOVICE: u8 = 3;

pub fn style_name(s: u8) -> &'static str {
    match s {
        STYLE_EAGER => "早め",
        STYLE_BALANCED => "標準",
        STYLE_HOARDER => "温存",
        _ => "無作為/初心者",
    }
}

/// 傾向の事前分布
pub const STYLE_PRIOR: [f32; NUM_STYLES] = [0.35, 0.35, 0.15, 0.15];

/// 使用機会の特徴量。**すべて公開情報**から作る（[`EventContext`] 由来）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Opportunity {
    /// 盗賊が本人の産出を止めている pip
    pub blocked_pips: u16,
    /// 騎士を出せば最大騎士力を取れる／守れる
    pub army_gain: bool,
    /// 使用済み騎士の枚数
    pub knights_played: u8,
    /// 合法な道の置き先がある
    pub can_build_road: bool,
    /// 道を 2 本足せば最長路を取れる見込み（長さだけの近似）
    pub road_race: bool,
    /// 銀行に 2 枚取れる資源がある（収穫が使える）
    pub bank_has_two: bool,
    /// 他人の手札の総枚数（独占の取り分の上限）
    pub others_hand_total: u8,
    /// 勝利までの公開点の距離
    pub vp_to_go: u8,
}

impl Opportunity {
    /// 出来事の直前の公開状態から、プレイヤー `p` の使用機会を作る
    pub fn from_context(c: &EventContext, p: PlayerId, n: usize) -> Self {
        let q = p as usize;
        let knights_after = c.played_knights[q] + 1;
        let army_gain = knights_after >= catan_core::game::MIN_ARMY
            && match c.largest_army_owner {
                None => true,
                Some(o) if o == p => false,
                Some(o) => knights_after > c.played_knights[o as usize],
            };
        let holder_len = c.longest_road_owner.map_or(0, |o| c.longest_road[o as usize]);
        let road_race = c.longest_road_owner != Some(p)
            && c.roads_left[q] >= 2
            && c.longest_road[q] + 2 >= holder_len.max(4) + 1;
        let bank_has_two = c.bank.map_or(true, |b| b.iter().any(|&v| v >= 1) && b.iter().map(|&v| v as u32).sum::<u32>() >= 2);
        let mut others = 0u8;
        for r in 0..n {
            if r != q {
                others = others.saturating_add(c.hand_size[r]);
            }
        }
        Opportunity {
            blocked_pips: c.blocked_pips[q],
            army_gain,
            knights_played: c.played_knights[q],
            can_build_road: c.can_build_road[q],
            road_race,
            bank_has_two,
            others_hand_total: others,
            vp_to_go: 10u8.saturating_sub(c.public_vp[q]),
        }
    }
}

/// `q(c, x, d, θ)` のパラメータ。数値の由来は docs/cpu-v2/m2-belief.md。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpponentParams {
    /// 特徴量を使う（false なら保持期間ごとの基準率だけ）
    pub use_features: bool,
    /// 傾向の混合を使う（false なら全員「標準」）
    pub use_styles: bool,
    /// 保持期間を使う（false なら期間 1 の率を常に使う）
    pub use_duration: bool,
    /// 種類ごと・保持した自分の手番数（1..=5+）ごとの基準使用率。添字 0 は未使用
    pub dur: [[f32; 6]; 4],
    /// 騎士: 盗賊に塞がれている／最大騎士力が取れる時の倍率
    pub knight_reason_mul: f32,
    /// 騎士: 理由が無い時の倍率
    pub knight_plain_mul: f32,
    /// 街道建設: 最長路レース時の倍率
    pub road_race_mul: f32,
    /// 傾向ごとの倍率
    pub style_mul: [f32; NUM_STYLES],
    /// どんな傾向でも「使う」確率はこれ以上（モデル外の行動で全滅しないための床）
    pub floor: f32,
    pub ceil: f32,
}

impl Default for OpponentParams {
    /// `catan v2-fit --games 120 --seed 6000000`（凍結した対照 v2_control×4）の実測（docs/cpu-v2/m2-belief.md）。
    ///
    /// 保持した自分の手番数（買った次の手番 = 1）別の使用率:
    ///   騎士 0.934 / 0.836 / 0.719 / 0.600 / 0.500(5+)
    ///   街道建設 0.793 / 0.308 / 0.370 / 0.231 / 0.207
    ///   収穫 0.961 / 0.857 / (少数) / 独占 0.964 / 0.667 / (少数)
    /// 特徴量: 騎士は「盗賊に塞がれ」0.962・「騎士力が取れる」1.000 に対し「普通」0.787。
    /// 街道建設は「最長路レース」0.964 に対し「普通」0.482。
    /// これを「早め」傾向の値にし、他の傾向は倍率で下げる。**人間の値ではない**。
    fn default() -> Self {
        OpponentParams {
            use_features: true,
            use_styles: true,
            use_duration: true,
            dur: [
                [0.0, 0.90, 0.80, 0.70, 0.60, 0.50], // 騎士
                [0.0, 0.70, 0.35, 0.35, 0.25, 0.20], // 街道建設
                [0.0, 0.95, 0.85, 0.80, 0.75, 0.70], // 収穫
                [0.0, 0.95, 0.70, 0.60, 0.55, 0.50], // 独占
            ],
            knight_reason_mul: 1.08,
            knight_plain_mul: 0.85,
            road_race_mul: 1.35,
            style_mul: [1.0, 0.8, 0.45, 0.6],
            floor: 0.03,
            ceil: 0.97,
        }
    }
}

impl OpponentParams {
    /// 特徴量も傾向も使わない版（アブレーション「保持期間だけ」）
    pub fn duration_only() -> Self {
        OpponentParams { use_features: false, use_styles: false, use_duration: true, ..Self::default() }
    }
    /// 特徴量あり・傾向なし（アブレーション「使用機会あり」）
    pub fn features_only() -> Self {
        OpponentParams { use_features: true, use_styles: false, use_duration: true, ..Self::default() }
    }
}

/// 「この機会に、種類 `c` の札を持っていたら使う」確率。
///
/// `held` は買った次の手番を 1 とする「本人の手番数」。
/// 0 を返すのは**ルール上使えない**時だけ（道が置けない街道、銀行が空の収穫、勝利点）。
/// それ以外は `floor` 以上にして、モデル外の行動で粒子が全滅しないようにする。
pub fn q_use(c: DevCard, x: &Opportunity, style: u8, held: u32, p: &OpponentParams) -> f32 {
    let d = if p.use_duration { (held.max(1) as usize).min(5) } else { 1 };
    let raw = match c {
        DevCard::VictoryPoint => return 0.0,
        DevCard::Knight => {
            let base = p.dur[0][d];
            if p.use_features {
                if x.blocked_pips > 0 || x.army_gain {
                    base * p.knight_reason_mul
                } else {
                    base * p.knight_plain_mul
                }
            } else {
                base
            }
        }
        DevCard::RoadBuilding => {
            if !x.can_build_road {
                return 0.0;
            }
            let base = p.dur[1][d];
            if p.use_features && x.road_race {
                base * p.road_race_mul
            } else {
                base
            }
        }
        DevCard::YearOfPlenty => {
            if !x.bank_has_two {
                return 0.0;
            }
            p.dur[2][d]
        }
        DevCard::Monopoly => p.dur[3][d],
    };
    let mul = if p.use_styles { p.style_mul[style as usize % NUM_STYLES] } else { 1.0 };
    (raw * mul).clamp(p.floor, p.ceil)
}
