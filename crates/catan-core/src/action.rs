//! 行動の表現。
//!
//! 設計方針（catanatron の設計を踏襲し、そこに正確な廃棄・交易を足したもの）:
//! **「意図」と「結果」を分ける**。`Action` は意図だけを持ち、
//! ダイス目・引いた発展カード・盗んだ資源といった偶然の結果は [`ActionRecord::result`] に入る。
//! これでリプレイ・巻き戻し・学習データ生成が同じ型で回る。

use crate::board::{PlayerId, Resource, NUM_RESOURCES};
use crate::topology::{EdgeId, NodeId, TileId};

/// 資源の束（種類別の枚数）。`[木, 土, 羊, 麦, 鉄]`。
pub type Bundle = [u8; NUM_RESOURCES];

pub const EMPTY: Bundle = [0; NUM_RESOURCES];

#[inline]
pub fn bundle_of(r: Resource, n: u8) -> Bundle {
    let mut b = EMPTY;
    b[r.idx()] = n;
    b
}

#[inline]
pub fn bundle_total(b: &Bundle) -> u8 {
    b.iter().sum()
}

#[inline]
pub fn bundle_contains(hand: &Bundle, need: &Bundle) -> bool {
    (0..NUM_RESOURCES).all(|i| hand[i] >= need[i])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum DevCard {
    Knight = 0,
    RoadBuilding = 1,
    YearOfPlenty = 2,
    Monopoly = 3,
    VictoryPoint = 4,
}

pub const NUM_DEV_KINDS: usize = 5;
pub const DEV_CARDS: [DevCard; NUM_DEV_KINDS] = [
    DevCard::Knight,
    DevCard::RoadBuilding,
    DevCard::YearOfPlenty,
    DevCard::Monopoly,
    DevCard::VictoryPoint,
];

impl DevCard {
    #[inline]
    pub const fn idx(self) -> usize {
        self as usize
    }
    pub const fn ja(self) -> &'static str {
        match self {
            DevCard::Knight => "騎士",
            DevCard::RoadBuilding => "街道建設",
            DevCard::YearOfPlenty => "収穫",
            DevCard::Monopoly => "独占",
            DevCard::VictoryPoint => "勝利点",
        }
    }
}

/// 発展カード 25 枚の内訳（公式）: 騎士14・街道建設2・収穫2・独占2・勝利点5
pub const DEV_DECK_COMPOSITION: [(DevCard, u8); NUM_DEV_KINDS] = [
    (DevCard::Knight, 14),
    (DevCard::RoadBuilding, 2),
    (DevCard::YearOfPlenty, 2),
    (DevCard::Monopoly, 2),
    (DevCard::VictoryPoint, 5),
];

pub const DEV_DECK_SIZE: usize = 25;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    // --- 初期配置 ---
    SetupSettlement(NodeId),
    SetupRoad(EdgeId),

    // --- 手番 ---
    Roll,
    EndTurn,

    // --- 建設・購入 ---
    BuildRoad(EdgeId),
    BuildSettlement(NodeId),
    BuildCity(NodeId),
    BuyDevCard,

    // --- 発展カード ---
    PlayKnight,
    PlayRoadBuilding,
    PlayYearOfPlenty(Resource, Resource),
    PlayMonopoly(Resource),

    // --- 7 と盗賊 ---
    /// 捨てる資源の内訳。合計は手札の半分（切り捨て）でなければならない。
    Discard(Bundle),
    /// 盗賊を `tile` へ動かし、`victim` から 1 枚盗む（盗める相手が居なければ `None`）。
    MoveRobber { tile: TileId, victim: Option<PlayerId> },

    // --- 交易 ---
    /// 銀行/港との交換。`give` を `count` 枚出して `take` を 1 枚もらう。
    MaritimeTrade { give: Resource, count: u8, take: Resource },
    /// 他プレイヤーへの提案。`give` を渡して `want` を受け取りたい。
    OfferTrade { give: Bundle, want: Bundle },
    AcceptTrade,
    RejectTrade,
    /// 対案の **候補を 1 つ積む**。返答はまだ終わらない。
    ///
    /// 卓上での「木か土ならいいよ」を表す。1 本の条件では書けないので、
    /// 成立してよい条件を複数並べて、**提案者にどれかを選ばせる**。
    /// 向きは [`Action::CounterOffer`] と同じ（対案を出した側から見たもの）。
    CounterAlt { give: Bundle, want: Bundle },
    /// 積んだ対案の候補を 1 つ取り消す（返答を終える前だけ）。
    /// 間違えて積んだものを戻せないと、条件を作り直すのに返答ごとやり直すしかない。
    CounterAltRemove(u8),
    /// 対案。提案を断りつつ「自分は `give` を出すので `want` が欲しい」と返す。
    /// 公式ルールの「他のプレイヤーも自分から提案や対案を出せる」に対応する。
    /// 向きは**対案を出した側から見た**もの。
    ///
    /// 直前に [`Action::CounterAlt`] を積んでいれば、**それらもまとめて**返答になる。
    CounterOffer { give: Bundle, want: Bundle },
    /// 提案者が、承諾者の中から相手を選んで元の条件で成立させる。
    ConfirmTrade(PlayerId),
    /// 提案者が、その相手の**対案**を受け入れて成立させる。
    /// `alt` は候補の番号（「木か土」の、どちらを取るか）。
    AcceptCounter { from: PlayerId, alt: u8 },
    /// 提案者が取り下げる。
    CancelTrade,
}

/// 行動の「結果」。偶然が絡む行動だけが値を持つ。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    None,
    /// 振ったダイス 2 個
    Dice(u8, u8),
    /// 引いた発展カード
    Drew(DevCard),
    /// 盗んだ資源（相手が 0 枚なら `None`）
    Stole(Option<Resource>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionRecord {
    pub actor: PlayerId,
    pub action: Action,
    pub result: Outcome,
}

/// いま誰が何を選ぶ場面か。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prompt {
    /// 初期配置の開拓地
    SetupSettlement,
    /// 初期配置の道（直前に置いた開拓地に接すること）
    SetupRoad,
    /// 通常の手番。ダイス前後の両方を含む（発展カードはダイス前でも打てる）
    PlayTurn,
    /// 7 が出た後の廃棄。`to_act` が捨てる番のプレイヤー
    Discard,
    /// 盗賊の移動（7 または騎士カード）
    MoveRobber,
    /// 街道建設カードによる無償の道
    FreeRoad,
    /// 交易提案への諾否
    DecideTrade,
    /// 提案者が承諾者から相手を選ぶ
    DecideAcceptees,
    GameOver,
}
